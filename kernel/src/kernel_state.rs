use crate::capability::{CapabilityHandle, CapabilitySpace, Rights};
#[cfg(target_arch = "riscv64")]
use crate::memory::address_space::{AddressSpace, Asid};
use crate::object::{self, KernelObject, ObjectTable, ObjectType};

#[cfg(target_arch = "riscv64")]
use crate::sync::SpinLock;

#[cfg(target_arch = "riscv64")]
static KERNEL_STATE: SpinLock<KernelState> = SpinLock::new(KernelState::new());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  Object(object::Error),
  Capability(crate::capability::Error),
  AddressSpace(crate::memory::address_space::Error),
  AlreadyInitialized,
  RollbackFailed,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Object(_) => "kernel object operation failed",
      Self::Capability(_) => "capability operation failed",
      Self::AddressSpace(_) => "bootstrap address space is invalid",
      Self::AlreadyInitialized => "bootstrap kernel state is already initialized",
      Self::RollbackFailed => "kernel state transaction rollback failed",
    }
  }
}

impl From<object::Error> for Error {
  fn from(error: object::Error) -> Self {
    Self::Object(error)
  }
}

impl From<crate::capability::Error> for Error {
  fn from(error: crate::capability::Error) -> Self {
    Self::Capability(error)
  }
}

pub struct KernelState {
  objects: ObjectTable,
  bootstrap_caps: CapabilitySpace,
  kernel_address_space: Option<CapabilityHandle>,
}

impl KernelState {
  pub const fn new() -> Self {
    Self {
      objects: ObjectTable::new(),
      bootstrap_caps: CapabilitySpace::new(),
      kernel_address_space: None,
    }
  }

  pub const fn kernel_address_space_handle(&self) -> Option<CapabilityHandle> {
    self.kernel_address_space
  }

  pub const fn capability_count(&self) -> usize {
    self.bootstrap_caps.live_count()
  }

  pub const fn object_count(&self) -> usize {
    self.objects.live_count()
  }

  pub fn create_object_with_capability(
    &mut self,
    object: KernelObject,
    rights: Rights,
  ) -> Result<CapabilityHandle, Error> {
    let object_type = object.object_type();
    let id = self.objects.insert(object)?;

    let handle = match self.bootstrap_caps.insert_initial(id, object_type, rights) {
      Ok(handle) => handle,
      Err(error) => {
        self
          .objects
          .rollback_unreferenced_insert(id)
          .map_err(|_| Error::RollbackFailed)?;
        return Err(Error::Capability(error));
      }
    };

    if self.objects.add_capability_reference(id).is_err() {
      let removed = self
        .bootstrap_caps
        .remove(handle)
        .map_err(|_| Error::RollbackFailed)?;
      if removed != id || self.objects.rollback_unreferenced_insert(id).is_err() {
        return Err(Error::RollbackFailed);
      }
      return Err(Error::Object(object::Error::ReferenceOverflow));
    }
    Ok(handle)
  }

  pub fn derive_capability(
    &mut self,
    parent: CapabilityHandle,
    rights: Rights,
  ) -> Result<CapabilityHandle, Error> {
    let (derived, object) = self.bootstrap_caps.derive_local(parent, rights)?;
    if self.objects.add_capability_reference(object).is_err() {
      self
        .bootstrap_caps
        .remove(derived)
        .map_err(|_| Error::RollbackFailed)?;
      return Err(Error::Object(object::Error::ReferenceOverflow));
    }
    Ok(derived)
  }

  pub fn remove_capability(&mut self, handle: CapabilityHandle) -> Result<(), Error> {
    let (object, _) = self.bootstrap_caps.identity(handle)?;
    self.objects.get(object)?;
    if self.objects.capability_references(object)? == 0 {
      return Err(Error::Object(object::Error::ReferenceUnderflow));
    }
    let removed = self.bootstrap_caps.remove(handle)?;
    debug_assert_eq!(removed, object);
    self.objects.remove_capability_reference(object)?;
    Ok(())
  }

  pub fn lookup(
    &self,
    handle: CapabilityHandle,
    expected_type: ObjectType,
    required: Rights,
  ) -> Result<&KernelObject, Error> {
    let object = self
      .bootstrap_caps
      .lookup(handle, expected_type, required)?;
    self.objects.validate_type(object, expected_type)?;
    Ok(self.objects.get(object)?)
  }
}

impl Default for KernelState {
  fn default() -> Self {
    Self::new()
  }
}

#[cfg(target_arch = "riscv64")]
pub fn initialize_kernel_address_space(root_frame: u64) -> Result<CapabilityHandle, Error> {
  let mut state = KERNEL_STATE.lock();
  if state.kernel_address_space.is_some() {
    return Err(Error::AlreadyInitialized);
  }

  let address_space = AddressSpace::new(root_frame, Asid::kernel()).map_err(Error::AddressSpace)?;
  let rights = Rights::MAP.union(Rights::SHARE);
  let handle =
    state.create_object_with_capability(KernelObject::AddressSpace(address_space), rights)?;
  state.kernel_address_space = Some(handle);
  Ok(handle)
}

#[cfg(target_arch = "riscv64")]
pub fn counts() -> (usize, usize) {
  let state = KERNEL_STATE.lock();
  (state.object_count(), state.capability_count())
}

#[cfg(test)]
mod tests {
  use crate::object::MemoryObject;

  use super::*;

  fn memory() -> KernelObject {
    KernelObject::Memory(MemoryObject::new(0x8080_0000).unwrap())
  }

  #[test]
  fn kstate_001_create_publishes_object_and_capability_together() {
    let mut state = KernelState::new();
    let handle = state
      .create_object_with_capability(memory(), Rights::READ.union(Rights::SHARE))
      .unwrap();
    assert_eq!(state.object_count(), 1);
    assert_eq!(state.capability_count(), 1);
    assert!(matches!(
      state.lookup(handle, ObjectType::Memory, Rights::READ),
      Ok(KernelObject::Memory(_))
    ));
  }

  #[test]
  fn kstate_002_invalid_initial_rights_leave_no_partial_object() {
    let mut state = KernelState::new();
    assert!(matches!(
      state.create_object_with_capability(memory(), Rights::CREATE),
      Err(Error::Capability(_))
    ));
    assert_eq!(state.object_count(), 0);
    assert_eq!(state.capability_count(), 0);
  }

  #[test]
  fn kstate_003_derivation_updates_reference_count_transactionally() {
    let mut state = KernelState::new();
    let parent = state
      .create_object_with_capability(memory(), Rights::READ.union(Rights::SHARE))
      .unwrap();
    let derived = state.derive_capability(parent, Rights::READ).unwrap();
    assert_eq!(state.object_count(), 1);
    assert_eq!(state.capability_count(), 2);
    state.remove_capability(derived).unwrap();
    assert_eq!(state.object_count(), 1);
    assert_eq!(state.capability_count(), 1);
  }

  #[test]
  fn kstate_004_object_id_never_substitutes_for_authority() {
    let mut state = KernelState::new();
    let handle = state
      .create_object_with_capability(memory(), Rights::READ)
      .unwrap();
    assert!(matches!(
      state.lookup(handle, ObjectType::Memory, Rights::WRITE),
      Err(Error::Capability(crate::capability::Error::RightsDenied))
    ));
  }

  #[test]
  fn kstate_005_failed_derivation_leaves_counts_unchanged() {
    let mut state = KernelState::new();
    let parent = state
      .create_object_with_capability(memory(), Rights::READ)
      .unwrap();
    assert!(matches!(
      state.derive_capability(parent, Rights::READ),
      Err(Error::Capability(crate::capability::Error::RightsDenied))
    ));
    assert_eq!(state.object_count(), 1);
    assert_eq!(state.capability_count(), 1);
  }
}
