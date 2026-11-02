use crate::capability::{Capability, CapabilityHandle, CapabilityNodeId, Rights};
use crate::capability_space::{self, CapabilitySpaceId, CapabilitySpaceTable};
#[cfg(target_arch = "riscv64")]
use crate::memory::address_space::{AddressSpace, Asid};
use crate::object::{self, KernelObject, ObjectId, ObjectTable, ObjectType};
use crate::process::ProcessState;
use crate::scheduler::{self, SchedulerState, WakeResult};
use crate::thread::{self, ThreadState};

#[cfg(target_arch = "riscv64")]
use crate::sync::SpinLock;

#[cfg(target_arch = "riscv64")]
static KERNEL_STATE: SpinLock<KernelState> = SpinLock::new(KernelState::new());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  Object(object::Error),
  Capability(crate::capability::Error),
  CapabilitySpace(capability_space::Error),
  AddressSpace(crate::memory::address_space::Error),
  AlreadyInitialized,
  NodeExhausted,
  CommitInvariant,
  Scheduler(scheduler::Error),
  SchedulerUninitialized,
  Thread(thread::Error),
  ProcessNotAlive,
  RollbackFailed,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Object(_) => "kernel object operation failed",
      Self::Capability(_) => "capability operation failed",
      Self::CapabilitySpace(_) => "capability space operation failed",
      Self::AddressSpace(_) => "bootstrap address space is invalid",
      Self::AlreadyInitialized => "bootstrap kernel state is already initialized",
      Self::NodeExhausted => "capability node identity is exhausted",
      Self::CommitInvariant => "capability transaction commit invariant failed",
      Self::Scheduler(_) => "scheduler operation failed",
      Self::SchedulerUninitialized => "scheduler is not initialized",
      Self::Thread(_) => "thread state transition failed",
      Self::ProcessNotAlive => "owner process is not alive",
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

impl From<capability_space::Error> for Error {
  fn from(error: capability_space::Error) -> Self {
    Self::CapabilitySpace(error)
  }
}

impl From<scheduler::Error> for Error {
  fn from(error: scheduler::Error) -> Self {
    Self::Scheduler(error)
  }
}

impl From<thread::Error> for Error {
  fn from(error: thread::Error) -> Self {
    Self::Thread(error)
  }
}

pub struct KernelState {
  objects: ObjectTable,
  spaces: CapabilitySpaceTable,
  next_capability_node: u64,
  kernel_address_space: Option<CapabilityHandle>,
  scheduler: SchedulerState,
}

impl KernelState {
  pub const fn new() -> Self {
    Self {
      objects: ObjectTable::new(),
      spaces: CapabilitySpaceTable::new(),
      next_capability_node: 1,
      kernel_address_space: None,
      scheduler: SchedulerState::uninitialized(),
    }
  }

  pub const fn bootstrap_space(&self) -> CapabilitySpaceId {
    self.spaces.bootstrap_id()
  }

  pub fn create_capability_space(&mut self) -> Result<CapabilitySpaceId, Error> {
    Ok(self.spaces.create()?)
  }

  fn next_node(&self) -> Result<CapabilityNodeId, Error> {
    CapabilityNodeId::new(self.next_capability_node).ok_or(Error::NodeExhausted)
  }

  fn commit_node(&mut self) -> Result<(), Error> {
    self.next_capability_node = self
      .next_capability_node
      .checked_add(1)
      .ok_or(Error::NodeExhausted)?;
    Ok(())
  }

  pub const fn kernel_address_space_handle(&self) -> Option<CapabilityHandle> {
    self.kernel_address_space
  }

  pub fn capability_count(&self) -> usize {
    match self.spaces.get(self.bootstrap_space()) {
      Ok(space) => space.live_count(),
      Err(_) => 0,
    }
  }

  pub const fn capability_space_count(&self) -> usize {
    self.spaces.live_count()
  }

  pub const fn object_count(&self) -> usize {
    self.objects.live_count()
  }

  pub fn create_object_with_capability(
    &mut self,
    object: KernelObject,
    rights: Rights,
  ) -> Result<CapabilityHandle, Error> {
    self.create_object_in_space(self.bootstrap_space(), object, rights)
  }

  pub fn create_object_in_space(
    &mut self,
    space: CapabilitySpaceId,
    object: KernelObject,
    rights: Rights,
  ) -> Result<CapabilityHandle, Error> {
    self.spaces.get(space)?;
    let node = self.next_node()?;
    self
      .next_capability_node
      .checked_add(1)
      .ok_or(Error::NodeExhausted)?;

    let object_type = object.object_type();
    let id = self.objects.insert(object)?;
    let handle = match self
      .spaces
      .get_mut(space)?
      .insert_initial(id, object_type, rights, node)
    {
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
        .spaces
        .get_mut(space)?
        .remove(handle)
        .map_err(|_| Error::RollbackFailed)?;
      if removed != id || self.objects.rollback_unreferenced_insert(id).is_err() {
        return Err(Error::RollbackFailed);
      }
      return Err(Error::Object(object::Error::ReferenceOverflow));
    }
    self.commit_node()?;
    Ok(handle)
  }

  pub fn derive_capability(
    &mut self,
    parent: CapabilityHandle,
    rights: Rights,
  ) -> Result<CapabilityHandle, Error> {
    let space = self.bootstrap_space();
    let node = self.next_node()?;
    self
      .next_capability_node
      .checked_add(1)
      .ok_or(Error::NodeExhausted)?;
    let parent_object = self.spaces.get(space)?.identity(parent)?.0;
    self.objects.can_add_capability_reference(parent_object)?;

    let (derived, object) = self
      .spaces
      .get_mut(space)?
      .derive_local(parent, rights, node)?;
    if self.objects.add_capability_reference(object).is_err() {
      self
        .spaces
        .get_mut(space)?
        .remove(derived)
        .map_err(|_| Error::RollbackFailed)?;
      return Err(Error::CommitInvariant);
    }
    self.commit_node()?;
    Ok(derived)
  }

  pub fn remove_capability(&mut self, handle: CapabilityHandle) -> Result<(), Error> {
    let bootstrap = self.bootstrap_space();
    let (object, _) = self.spaces.get(bootstrap)?.identity(handle)?;
    self.objects.get(object)?;
    if self.objects.capability_references(object)? == 0 {
      return Err(Error::Object(object::Error::ReferenceUnderflow));
    }
    let removed = self.spaces.get_mut(bootstrap)?.remove(handle)?;
    debug_assert_eq!(removed, object);
    self.objects.remove_capability_reference(object)?;
    Ok(())
  }

  pub fn share_capability(
    &mut self,
    source_space: CapabilitySpaceId,
    source_handle: CapabilityHandle,
    destination_space: CapabilitySpaceId,
    rights: Rights,
  ) -> Result<CapabilityHandle, Error> {
    let source = self.spaces.get(source_space)?.snapshot(source_handle)?;
    let node = self.next_node()?;
    self
      .next_capability_node
      .checked_add(1)
      .ok_or(Error::NodeExhausted)?;
    let derived = Capability::derived(source, rights, node)?;
    self.objects.get(source.object())?;
    self.objects.can_add_capability_reference(source.object())?;
    if !self.spaces.get(destination_space)?.has_free_slot() {
      return Err(Error::Capability(crate::capability::Error::SpaceFull));
    }

    let handle = self
      .spaces
      .get_mut(destination_space)?
      .insert_existing(derived)
      .map_err(|_| Error::CommitInvariant)?;
    self
      .objects
      .add_capability_reference(source.object())
      .map_err(|_| Error::CommitInvariant)?;
    self.commit_node()?;
    Ok(handle)
  }

  pub fn transfer_capability(
    &mut self,
    source_space: CapabilitySpaceId,
    source_handle: CapabilityHandle,
    destination_space: CapabilitySpaceId,
  ) -> Result<CapabilityHandle, Error> {
    if source_space == destination_space {
      return Err(Error::CapabilitySpace(capability_space::Error::SameSpace));
    }
    let source = self.spaces.get(source_space)?.snapshot(source_handle)?;
    if !source.rights().contains(Rights::TRANSFER) {
      return Err(Error::Capability(crate::capability::Error::RightsDenied));
    }
    self.objects.get(source.object())?;
    if !self.spaces.get(destination_space)?.has_free_slot() {
      return Err(Error::Capability(crate::capability::Error::SpaceFull));
    }

    let (source_caps, destination_caps) = self.spaces.pair_mut(source_space, destination_space)?;
    let moved = source_caps
      .remove_full(source_handle)
      .map_err(|_| Error::CommitInvariant)?;
    debug_assert_eq!(moved.node(), source.node());
    debug_assert_eq!(moved.parent(), source.parent());
    destination_caps
      .insert_existing(moved)
      .map_err(|_| Error::CommitInvariant)
  }

  pub fn lookup(
    &self,
    handle: CapabilityHandle,
    expected_type: ObjectType,
    required: Rights,
  ) -> Result<&KernelObject, Error> {
    self.lookup_in_space(self.bootstrap_space(), handle, expected_type, required)
  }

  pub fn lookup_in_space(
    &self,
    space: CapabilitySpaceId,
    handle: CapabilityHandle,
    expected_type: ObjectType,
    required: Rights,
  ) -> Result<&KernelObject, Error> {
    let object = self
      .spaces
      .get(space)?
      .lookup(handle, expected_type, required)?;
    self.objects.validate_type(object, expected_type)?;
    Ok(self.objects.get(object)?)
  }

  pub fn initialize_scheduler(&mut self, online_mask: u64) -> Result<(), Error> {
    if self.scheduler.is_initialized() {
      return Err(Error::AlreadyInitialized);
    }
    self.scheduler.initialize(online_mask)?;
    Ok(())
  }

  pub fn scheduler_summary(&self) -> Result<(u64, usize, usize), Error> {
    let scheduler = self.scheduler()?;
    Ok((
      scheduler.online_mask(),
      scheduler.total_runnable(),
      scheduler.total_running(),
    ))
  }

  fn scheduler(&self) -> Result<&SchedulerState, Error> {
    self
      .scheduler
      .is_initialized()
      .then_some(&self.scheduler)
      .ok_or(Error::SchedulerUninitialized)
  }

  fn scheduler_mut(&mut self) -> Result<&mut SchedulerState, Error> {
    if !self.scheduler.is_initialized() {
      return Err(Error::SchedulerUninitialized);
    }
    Ok(&mut self.scheduler)
  }

  fn thread_snapshot(
    &mut self,
    id: ObjectId,
  ) -> Result<(ObjectId, ThreadState, u64, u8, Option<u16>), Error> {
    let thread = self
      .objects
      .get(id)?
      .as_thread()
      .ok_or(Error::Object(object::Error::WrongObjectType))?;
    Ok((
      thread.process(),
      thread.state(),
      thread.affinity(),
      thread.effective_priority(),
      thread.running_hart(),
    ))
  }

  fn require_process_alive(&self, id: ObjectId) -> Result<(), Error> {
    let process = self
      .objects
      .get(id)?
      .as_process()
      .ok_or(Error::Object(object::Error::WrongObjectType))?;
    if process.state() != ProcessState::Alive {
      return Err(Error::ProcessNotAlive);
    }
    Ok(())
  }

  pub fn wake_thread(&mut self, thread_id: ObjectId) -> Result<WakeResult, Error> {
    let (process, state, affinity, priority, _) = self.thread_snapshot(thread_id)?;
    self.require_process_alive(process)?;
    if !matches!(state, ThreadState::New | ThreadState::Blocked) {
      return Err(Error::Thread(thread::Error::InvalidState));
    }

    let result = self
      .scheduler_mut()?
      .enqueue(thread_id, affinity, priority)?;
    self
      .objects
      .get_mut(thread_id)?
      .as_thread_mut()
      .ok_or(Error::CommitInvariant)?
      .make_runnable()
      .map_err(|_| Error::CommitInvariant)?;
    Ok(result)
  }

  pub fn dispatch_next(&mut self, hart: usize) -> Result<Option<ObjectId>, Error> {
    let candidate = self.scheduler()?.peek_next(hart)?;
    let Some(thread_id) = candidate else {
      return Ok(None);
    };
    let (_, state, affinity, _, _) = self.thread_snapshot(thread_id)?;
    let hart_bit = 1u64
      .checked_shl(hart as u32)
      .ok_or(Error::CommitInvariant)?;
    if state != ThreadState::Runnable || affinity & hart_bit == 0 {
      return Err(Error::CommitInvariant);
    }

    self.scheduler_mut()?.dispatch_expected(hart, thread_id)?;
    self
      .objects
      .get_mut(thread_id)?
      .as_thread_mut()
      .ok_or(Error::CommitInvariant)?
      .begin_running(hart)
      .map_err(|_| Error::CommitInvariant)?;
    Ok(Some(thread_id))
  }

  pub fn preempt_current(&mut self, hart: usize) -> Result<WakeResult, Error> {
    let thread_id = self
      .scheduler()?
      .running_thread(hart)?
      .ok_or(Error::Scheduler(scheduler::Error::NoRunningThread))?;
    let (_, state, affinity, priority, running_hart) = self.thread_snapshot(thread_id)?;
    if state != ThreadState::Running || running_hart != u16::try_from(hart).ok() {
      return Err(Error::CommitInvariant);
    }
    let result = self.scheduler_mut()?.preempt(hart, affinity, priority)?;
    self
      .objects
      .get_mut(thread_id)?
      .as_thread_mut()
      .ok_or(Error::CommitInvariant)?
      .preempt()
      .map_err(|_| Error::CommitInvariant)?;
    Ok(result)
  }

  pub fn block_current(&mut self, hart: usize) -> Result<ObjectId, Error> {
    let thread_id = self
      .scheduler()?
      .running_thread(hart)?
      .ok_or(Error::Scheduler(scheduler::Error::NoRunningThread))?;
    let (_, state, _, _, running_hart) = self.thread_snapshot(thread_id)?;
    if state != ThreadState::Running || running_hart != u16::try_from(hart).ok() {
      return Err(Error::CommitInvariant);
    }
    let removed = self.scheduler_mut()?.block_running(hart)?;
    debug_assert_eq!(removed, thread_id);
    self
      .objects
      .get_mut(thread_id)?
      .as_thread_mut()
      .ok_or(Error::CommitInvariant)?
      .block()
      .map_err(|_| Error::CommitInvariant)?;
    Ok(thread_id)
  }

  #[cfg(test)]
  fn insert_test_object(&mut self, object: KernelObject) -> Result<ObjectId, Error> {
    Ok(self.objects.insert(object)?)
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
pub fn counts() -> (usize, usize, usize) {
  let state = KERNEL_STATE.lock();
  (
    state.object_count(),
    state.capability_count(),
    state.capability_space_count(),
  )
}

#[cfg(target_arch = "riscv64")]
pub fn initialize_scheduler_state(online_mask: u64) -> Result<(u64, usize, usize), Error> {
  let mut state = KERNEL_STATE.lock();
  state.initialize_scheduler(online_mask)?;
  state.scheduler_summary()
}

#[cfg(test)]
mod tests {
  use crate::object::MemoryObject;
  use crate::process::{Process, ProcessLimits};
  use crate::thread::{Thread, ThreadState};

  use super::*;

  fn memory() -> KernelObject {
    KernelObject::Memory(MemoryObject::new(0x8080_0000).unwrap())
  }

  fn scheduler_state(state: &mut KernelState) -> (ObjectId, ObjectId) {
    state.initialize_scheduler(0b1111).unwrap();
    let mut process = Process::new(
      ObjectId::from_raw_parts(90, 1),
      state.bootstrap_space(),
      ProcessLimits::new(4, 4, 8, 8).unwrap(),
    );
    process.start().unwrap();
    let process = state
      .insert_test_object(KernelObject::Process(process))
      .unwrap();
    let thread = Thread::new(process, 0b0101, 8).unwrap();
    let thread = state
      .insert_test_object(KernelObject::Thread(thread))
      .unwrap();
    (process, thread)
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
      .create_object_with_capability(memory(), Rights::READ.union(Rights::GRANT))
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

  #[test]
  fn cspace_006_same_handle_in_another_space_has_no_authority() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let other = state.create_capability_space().unwrap();
    let handle = state
      .create_object_in_space(source, memory(), Rights::READ)
      .unwrap();
    assert!(matches!(
      state.lookup_in_space(other, handle, ObjectType::Memory, Rights::READ),
      Err(Error::Capability(crate::capability::Error::InvalidHandle))
    ));
  }

  #[test]
  fn cap_020_share_creates_attenuated_destination_capability() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(
        source,
        memory(),
        Rights::READ.union(Rights::WRITE).union(Rights::GRANT),
      )
      .unwrap();
    let shared = state
      .share_capability(source, parent, destination, Rights::READ)
      .unwrap();
    assert!(
      state
        .lookup_in_space(source, parent, ObjectType::Memory, Rights::WRITE)
        .is_ok()
    );
    assert!(
      state
        .lookup_in_space(destination, shared, ObjectType::Memory, Rights::READ)
        .is_ok()
    );
    assert!(matches!(
      state.lookup_in_space(destination, shared, ObjectType::Memory, Rights::WRITE),
      Err(Error::Capability(crate::capability::Error::RightsDenied))
    ));
  }

  #[test]
  fn cap_021_share_without_grant_is_rejected_without_partial_state() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(source, memory(), Rights::READ)
      .unwrap();
    let before = state.spaces.get(destination).unwrap().live_count();
    assert!(matches!(
      state.share_capability(source, parent, destination, Rights::READ),
      Err(Error::Capability(crate::capability::Error::RightsDenied))
    ));
    assert_eq!(state.spaces.get(destination).unwrap().live_count(), before);
  }

  #[test]
  fn cap_023_share_retains_source_and_records_lineage() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(source, memory(), Rights::READ.union(Rights::GRANT))
      .unwrap();
    let shared = state
      .share_capability(source, parent, destination, Rights::READ)
      .unwrap();
    let parent_capability = state.spaces.get(source).unwrap().snapshot(parent).unwrap();
    let child = state
      .spaces
      .get(destination)
      .unwrap()
      .snapshot(shared)
      .unwrap();
    assert_eq!(child.parent(), Some(parent_capability.node()));
    assert!(state.spaces.get(source).unwrap().snapshot(parent).is_ok());
  }

  #[test]
  fn cap_024_transfer_invalidates_source_and_preserves_node() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let handle = state
      .create_object_in_space(source, memory(), Rights::READ.union(Rights::TRANSFER))
      .unwrap();
    let node = state
      .spaces
      .get(source)
      .unwrap()
      .snapshot(handle)
      .unwrap()
      .node();
    let moved = state
      .transfer_capability(source, handle, destination)
      .unwrap();
    assert!(state.spaces.get(source).unwrap().snapshot(handle).is_err());
    assert_eq!(
      state
        .spaces
        .get(destination)
        .unwrap()
        .snapshot(moved)
        .unwrap()
        .node(),
      node
    );
  }

  #[test]
  fn cap_025_failed_transfer_changes_neither_space() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let handle = state
      .create_object_in_space(source, memory(), Rights::READ)
      .unwrap();
    assert!(matches!(
      state.transfer_capability(source, handle, destination),
      Err(Error::Capability(crate::capability::Error::RightsDenied))
    ));
    assert!(state.spaces.get(source).unwrap().snapshot(handle).is_ok());
    assert_eq!(state.spaces.get(destination).unwrap().live_count(), 0);
  }

  #[test]
  fn cap_026_full_destination_rolls_back_before_commit() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(
        source,
        memory(),
        Rights::READ.union(Rights::GRANT).union(Rights::TRANSFER),
      )
      .unwrap();
    for _ in 0..crate::capability::MAX_CAPABILITY_SLOTS {
      state
        .share_capability(source, parent, destination, Rights::READ)
        .unwrap();
    }
    let source_count = state.spaces.get(source).unwrap().live_count();
    let destination_count = state.spaces.get(destination).unwrap().live_count();
    assert!(matches!(
      state.share_capability(source, parent, destination, Rights::READ),
      Err(Error::Capability(crate::capability::Error::SpaceFull))
    ));
    assert!(state.spaces.get(source).unwrap().snapshot(parent).is_ok());
    assert_eq!(state.spaces.get(source).unwrap().live_count(), source_count);
    assert_eq!(
      state.spaces.get(destination).unwrap().live_count(),
      destination_count
    );
  }

  #[test]
  fn cap_030_share_increments_object_reference_count() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(source, memory(), Rights::READ.union(Rights::GRANT))
      .unwrap();
    let object = state
      .spaces
      .get(source)
      .unwrap()
      .snapshot(parent)
      .unwrap()
      .object();
    let before = state.objects.capability_references(object).unwrap();
    state
      .share_capability(source, parent, destination, Rights::READ)
      .unwrap();
    assert_eq!(
      state.objects.capability_references(object).unwrap(),
      before + 1
    );
  }

  #[test]
  fn cap_031_transfer_preserves_object_reference_count() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let handle = state
      .create_object_in_space(source, memory(), Rights::READ.union(Rights::TRANSFER))
      .unwrap();
    let object = state
      .spaces
      .get(source)
      .unwrap()
      .snapshot(handle)
      .unwrap()
      .object();
    let before = state.objects.capability_references(object).unwrap();
    state
      .transfer_capability(source, handle, destination)
      .unwrap();
    assert_eq!(state.objects.capability_references(object), Ok(before));
  }

  #[test]
  fn cap_032_same_space_transfer_is_rejected_without_change() {
    let mut state = KernelState::new();
    let space = state.bootstrap_space();
    let handle = state
      .create_object_in_space(space, memory(), Rights::READ.union(Rights::TRANSFER))
      .unwrap();
    assert!(matches!(
      state.transfer_capability(space, handle, space),
      Err(Error::CapabilitySpace(capability_space::Error::SameSpace))
    ));
    assert!(state.spaces.get(space).unwrap().snapshot(handle).is_ok());
  }

  #[test]
  fn cap_033_stale_source_does_not_consume_a_node_id() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(source, memory(), Rights::READ.union(Rights::GRANT))
      .unwrap();
    let stale = CapabilityHandle::from_raw_parts(parent.slot(), parent.generation() + 1);
    let next_node = state.next_capability_node;
    assert!(matches!(
      state.share_capability(source, stale, destination, Rights::READ),
      Err(Error::Capability(crate::capability::Error::StaleHandle))
    ));
    assert_eq!(state.next_capability_node, next_node);
  }

  #[test]
  fn sched_020_wake_updates_queue_and_thread_state_together() {
    let mut state = KernelState::new();
    let (_, thread) = scheduler_state(&mut state);
    let wake = state.wake_thread(thread).unwrap();
    assert!(matches!(wake.target_hart, 0 | 2));
    assert_eq!(
      state
        .objects
        .get(thread)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::Runnable
    );
    assert_eq!(state.scheduler.total_runnable(), 1);
  }

  #[test]
  fn sched_021_dispatch_updates_running_membership_and_thread_state() {
    let mut state = KernelState::new();
    let (_, thread) = scheduler_state(&mut state);
    let target = state.wake_thread(thread).unwrap().target_hart;
    assert_eq!(state.dispatch_next(target), Ok(Some(thread)));
    let object = state.objects.get(thread).unwrap().as_thread().unwrap();
    assert_eq!(object.state(), ThreadState::Running);
    assert_eq!(object.running_hart(), Some(target as u16));
  }

  #[test]
  fn sched_022_preempt_returns_thread_to_one_queue() {
    let mut state = KernelState::new();
    let (_, thread) = scheduler_state(&mut state);
    let target = state.wake_thread(thread).unwrap().target_hart;
    state.dispatch_next(target).unwrap();
    state.preempt_current(target).unwrap();
    assert_eq!(
      state
        .objects
        .get(thread)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::Runnable
    );
    assert_eq!(state.scheduler.total_running(), 0);
    assert_eq!(state.scheduler.total_runnable(), 1);
  }

  #[test]
  fn sched_023_block_removes_thread_from_scheduler() {
    let mut state = KernelState::new();
    let (_, thread) = scheduler_state(&mut state);
    let target = state.wake_thread(thread).unwrap().target_hart;
    state.dispatch_next(target).unwrap();
    assert_eq!(state.block_current(target), Ok(thread));
    assert_eq!(
      state
        .objects
        .get(thread)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::Blocked
    );
    assert_eq!(state.scheduler.total_running(), 0);
    assert_eq!(state.scheduler.total_runnable(), 0);
  }

  #[test]
  fn sched_024_stopping_process_cannot_wake_thread() {
    let mut state = KernelState::new();
    let (process, thread) = scheduler_state(&mut state);
    state
      .objects
      .get_mut(process)
      .unwrap()
      .as_process_mut()
      .unwrap()
      .begin_stop()
      .unwrap();
    assert_eq!(state.wake_thread(thread), Err(Error::ProcessNotAlive));
    assert_eq!(state.scheduler.total_runnable(), 0);
    assert_eq!(
      state
        .objects
        .get(thread)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::New
    );
  }
}
