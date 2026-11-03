use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::riscv64::paging::UserPermissions;
use crate::memory::user_page_table::UserPageTable;
#[cfg(target_arch = "riscv64")]
use crate::memory::user_page_table::{MappingFailure, MappingSuccess, PreparedMappingFrames};
use crate::object::ObjectId;
use crate::platform::MAX_HARTS;
#[cfg(target_arch = "riscv64")]
use crate::{arch::riscv64::tlb, sync::SpinLock};

const KERNEL_ASID: u16 = 0;
const KERNEL_GENERATION: u64 = 0;
pub const MAX_USER_MAPPINGS: usize = 2;

const _: () = assert!(MAX_HARTS <= u64::BITS as usize);

#[cfg(target_arch = "riscv64")]
static ASID_ALLOCATOR: SpinLock<Option<AsidState>> = SpinLock::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Asid {
  value: u16,
  generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchToken {
  satp: u64,
  asid: Asid,
}

impl DispatchToken {
  pub const fn satp(self) -> u64 {
    self.satp
  }

  pub const fn asid(self) -> Asid {
    self.asid
  }
}

impl Asid {
  pub const fn kernel() -> Self {
    Self {
      value: KERNEL_ASID,
      generation: KERNEL_GENERATION,
    }
  }

  pub const fn value(self) -> u16 {
    self.value
  }

  pub const fn generation(self) -> u64 {
    self.generation
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  InvalidRootFrame,
  InvalidHartIndex,
  GenerationOverflow,
  AsidUnavailable,
}

#[cfg(target_arch = "riscv64")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocatorError {
  InvalidAsidBits,
  AlreadyInitialized,
  Uninitialized,
  State(Error),
  Shootdown(tlb::Error),
}

#[cfg(target_arch = "riscv64")]
impl AllocatorError {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::InvalidAsidBits => "ASID width exceeds the Sv39 field",
      Self::AlreadyInitialized => "ASID allocator is already initialized",
      Self::Uninitialized => "ASID allocator is not initialized",
      Self::State(_) => "ASID allocator state transition failed",
      Self::Shootdown(_) => "global TLB shootdown failed during ASID rollover",
    }
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappingRecord {
  memory: ObjectId,
  virtual_address: u64,
  permissions: UserPermissions,
}

impl MappingRecord {
  pub const fn memory(&self) -> ObjectId {
    self.memory
  }

  pub const fn virtual_address(&self) -> u64 {
    self.virtual_address
  }

  pub const fn permissions(&self) -> UserPermissions {
    self.permissions
  }
}

enum AddressSpaceKind {
  Kernel { root_frame: u64 },
  User { table: UserPageTable },
}

pub struct AddressSpace {
  kind: AddressSpaceKind,
  asid: Asid,
  active_harts: AtomicU64,
  resident_harts: AtomicU64,
  mappings: [Option<MappingRecord>; MAX_USER_MAPPINGS],
  mapping_len: usize,
}

impl AddressSpace {
  pub fn new(root_frame: u64, asid: Asid) -> Result<Self, Error> {
    if root_frame == 0 || !root_frame.is_multiple_of(4096) {
      return Err(Error::InvalidRootFrame);
    }
    Ok(Self {
      kind: AddressSpaceKind::Kernel { root_frame },
      asid,
      active_harts: AtomicU64::new(0),
      resident_harts: AtomicU64::new(0),
      mappings: [const { None }; MAX_USER_MAPPINGS],
      mapping_len: 0,
    })
  }

  pub fn new_user(table: UserPageTable, asid: Asid) -> Result<Self, Error> {
    let root_frame = table.root_frame();
    if root_frame == 0 || !root_frame.is_multiple_of(4096) {
      return Err(Error::InvalidRootFrame);
    }
    Ok(Self {
      kind: AddressSpaceKind::User { table },
      asid,
      active_harts: AtomicU64::new(0),
      resident_harts: AtomicU64::new(0),
      mappings: [const { None }; MAX_USER_MAPPINGS],
      mapping_len: 0,
    })
  }

  pub const fn root_frame(&self) -> u64 {
    match &self.kind {
      AddressSpaceKind::Kernel { root_frame } => *root_frame,
      AddressSpaceKind::User { table } => table.root_frame(),
    }
  }

  pub const fn asid(&self) -> Asid {
    self.asid
  }

  pub fn dispatch_token(&self) -> Result<DispatchToken, crate::arch::riscv64::paging::Error> {
    Ok(DispatchToken {
      satp: crate::arch::riscv64::paging::satp_value_for(self.root_frame(), self.asid.value())?,
      asid: self.asid,
    })
  }

  pub const fn is_user(&self) -> bool {
    matches!(self.kind, AddressSpaceKind::User { .. })
  }

  pub const fn mapping_count(&self) -> usize {
    self.mapping_len
  }

  pub fn mapping(&self, index: usize) -> Option<&MappingRecord> {
    self.mappings.get(index).and_then(Option::as_ref)
  }

  pub fn has_initial_user_image(&self) -> bool {
    if !self.is_user() {
      return false;
    }
    let mut text = false;
    let mut stack = false;
    for mapping in self.mappings[..self.mapping_len].iter().flatten() {
      match (mapping.virtual_address(), mapping.permissions()) {
        (crate::memory::user_page_table::USER_TEXT_BASE, UserPermissions::ReadExecute) => {
          text = true
        }
        (crate::memory::user_page_table::USER_STACK_BASE, UserPermissions::ReadWrite) => {
          stack = true
        }
        _ => {}
      }
    }
    text && stack
  }

  #[cfg(target_arch = "riscv64")]
  pub fn map_user_page(
    &mut self,
    memory: ObjectId,
    physical_address: u64,
    virtual_address: u64,
    permissions: UserPermissions,
    frames: PreparedMappingFrames,
  ) -> Result<MappingSuccess, MappingFailure> {
    if self.mapping_len == MAX_USER_MAPPINGS {
      return Err(MappingFailure::new(
        crate::memory::user_page_table::Error::MappingLimit,
        frames,
      ));
    }
    if self.mappings[..self.mapping_len]
      .iter()
      .flatten()
      .any(|mapping| mapping.virtual_address() == virtual_address)
    {
      return Err(MappingFailure::new(
        crate::memory::user_page_table::Error::AlreadyMapped,
        frames,
      ));
    }
    let table = match &mut self.kind {
      AddressSpaceKind::User { table } => table,
      AddressSpaceKind::Kernel { .. } => {
        return Err(MappingFailure::new(
          crate::memory::user_page_table::Error::KernelAddressSpace,
          frames,
        ));
      }
    };
    let success = table.map_user_page(virtual_address, physical_address, permissions, frames)?;
    self.mappings[self.mapping_len] = Some(MappingRecord {
      memory,
      virtual_address,
      permissions,
    });
    self.mapping_len += 1;
    Ok(success)
  }

  pub fn enter_hart(&self, index: usize) -> Result<(), Error> {
    let bit = hart_bit(index)?;
    self.resident_harts.fetch_or(bit, Ordering::Release);
    self.active_harts.fetch_or(bit, Ordering::Release);
    Ok(())
  }

  pub fn leave_hart(&self, index: usize) -> Result<(), Error> {
    let bit = hart_bit(index)?;
    self.active_harts.fetch_and(!bit, Ordering::Release);
    Ok(())
  }

  pub fn active_harts(&self) -> u64 {
    self.active_harts.load(Ordering::Acquire)
  }

  pub fn resident_harts(&self) -> u64 {
    self.resident_harts.load(Ordering::Acquire)
  }

  pub fn clear_residency_after_shootdown(&self, acknowledged: u64) {
    let inactive = acknowledged & !self.active_harts.load(Ordering::Acquire);
    self.resident_harts.fetch_and(!inactive, Ordering::AcqRel);
  }
}

fn hart_bit(index: usize) -> Result<u64, Error> {
  if index >= MAX_HARTS || index >= u64::BITS as usize {
    return Err(Error::InvalidHartIndex);
  }
  Ok(1u64 << index)
}

pub struct AsidState {
  max_asid: u16,
  next: u32,
  generation: u64,
}

impl AsidState {
  pub const fn new(max_asid: u16) -> Self {
    Self {
      max_asid,
      next: 1,
      generation: 1,
    }
  }

  pub fn allocate_without_rollover(&mut self) -> Result<Asid, Error> {
    if self.max_asid == 0 {
      let generation = self.generation;
      self.generation = self
        .generation
        .checked_add(1)
        .ok_or(Error::GenerationOverflow)?;
      return Ok(Asid {
        value: 0,
        generation,
      });
    }
    if self.next > u32::from(self.max_asid) {
      return Err(Error::AsidUnavailable);
    }
    let value = self.next as u16;
    self.next += 1;
    Ok(Asid {
      value,
      generation: self.generation,
    })
  }

  pub fn complete_rollover(&mut self) -> Result<(), Error> {
    self.generation = self
      .generation
      .checked_add(1)
      .ok_or(Error::GenerationOverflow)?;
    self.next = 1;
    Ok(())
  }
}

#[cfg(target_arch = "riscv64")]
pub fn initialize_allocator(asid_bits: u32) -> Result<(), AllocatorError> {
  if asid_bits > 16 {
    return Err(AllocatorError::InvalidAsidBits);
  }
  let max_asid = if asid_bits == 0 {
    0
  } else {
    ((1u32 << asid_bits) - 1) as u16
  };
  let mut allocator = ASID_ALLOCATOR.lock();
  if allocator.is_some() {
    return Err(AllocatorError::AlreadyInitialized);
  }
  *allocator = Some(AsidState::new(max_asid));
  Ok(())
}

#[cfg(target_arch = "riscv64")]
pub fn allocate_asid() -> Result<Asid, AllocatorError> {
  let mut allocator = ASID_ALLOCATOR.lock();
  let state = allocator.as_mut().ok_or(AllocatorError::Uninitialized)?;
  match state.allocate_without_rollover() {
    Ok(asid) => Ok(asid),
    Err(Error::AsidUnavailable) => {
      tlb::shootdown_global().map_err(AllocatorError::Shootdown)?;
      state.complete_rollover().map_err(AllocatorError::State)?;
      state
        .allocate_without_rollover()
        .map_err(AllocatorError::State)
    }
    Err(error) => Err(AllocatorError::State(error)),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn asid_001_zero_is_reserved_for_the_kernel() {
    assert_eq!(Asid::kernel().value(), 0);
    let mut state = AsidState::new(2);
    assert_eq!(state.allocate_without_rollover().unwrap().value(), 1);
  }

  #[test]
  fn asid_002_allocation_is_unique_until_exhaustion() {
    let mut state = AsidState::new(2);
    assert_eq!(state.allocate_without_rollover().unwrap().value(), 1);
    assert_eq!(state.allocate_without_rollover().unwrap().value(), 2);
    assert_eq!(
      state.allocate_without_rollover(),
      Err(Error::AsidUnavailable)
    );
  }

  #[test]
  fn asid_004_rollover_advances_generation_and_restarts_at_one() {
    let mut state = AsidState::new(1);
    let first = state.allocate_without_rollover().unwrap();
    assert_eq!(
      state.allocate_without_rollover(),
      Err(Error::AsidUnavailable)
    );
    state.complete_rollover().unwrap();
    let second = state.allocate_without_rollover().unwrap();
    assert_eq!(second.value(), 1);
    assert!(second.generation() > first.generation());
  }

  #[test]
  fn aspace_002_enter_and_leave_preserve_residency() {
    let space = AddressSpace::new(0x8020_0000, Asid::kernel()).unwrap();
    space.enter_hart(2).unwrap();
    assert_eq!(space.active_harts(), 1 << 2);
    assert_eq!(space.resident_harts(), 1 << 2);
    space.leave_hart(2).unwrap();
    assert_eq!(space.active_harts(), 0);
    assert_eq!(space.resident_harts(), 1 << 2);
  }

  #[test]
  fn aspace_004_invalid_hart_index_is_rejected() {
    let space = AddressSpace::new(0x8020_0000, Asid::kernel()).unwrap();
    assert_eq!(space.enter_hart(MAX_HARTS), Err(Error::InvalidHartIndex));
  }

  #[test]
  fn aspace_005_kernel_space_is_not_a_user_mapping_target() {
    let space = AddressSpace::new(0x8020_0000, Asid::kernel()).unwrap();
    assert!(!space.is_user());
    assert_eq!(space.mapping_count(), 0);
    assert!(space.mapping(0).is_none());
  }

  #[test]
  fn aspace_006_initial_user_image_capacity_is_two_mappings() {
    assert_eq!(MAX_USER_MAPPINGS, 2);
  }

  #[test]
  fn aspace_007_dispatch_token_encodes_root_and_asid() {
    let mut state = AsidState::new(2);
    let asid = state.allocate_without_rollover().unwrap();
    let space = AddressSpace::new(0x8020_0000, asid).unwrap();
    let token = space.dispatch_token().unwrap();
    assert_eq!(token.asid(), asid);
    assert_eq!(
      token.satp(),
      crate::arch::riscv64::paging::satp_value_for(0x8020_0000, 1).unwrap()
    );
  }

  #[test]
  fn aspace_008_kernel_dispatch_token_uses_asid_zero() {
    let space = AddressSpace::new(0x8020_0000, Asid::kernel()).unwrap();
    let token = space.dispatch_token().unwrap();
    assert_eq!(token.asid(), Asid::kernel());
    assert_eq!(
      token.satp(),
      crate::arch::riscv64::paging::satp_value_for(0x8020_0000, 0).unwrap()
    );
  }
}
