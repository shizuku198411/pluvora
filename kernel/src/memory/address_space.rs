use core::sync::atomic::{AtomicU64, Ordering};

use crate::platform::MAX_HARTS;
#[cfg(target_arch = "riscv64")]
use crate::{arch::riscv64::tlb, sync::SpinLock};

const KERNEL_ASID: u16 = 0;
const KERNEL_GENERATION: u64 = 0;

const _: () = assert!(MAX_HARTS <= u64::BITS as usize);

#[cfg(target_arch = "riscv64")]
static ASID_ALLOCATOR: SpinLock<Option<AsidState>> = SpinLock::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Asid {
  value: u16,
  generation: u64,
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

pub struct AddressSpace {
  root_frame: u64,
  asid: Asid,
  active_harts: AtomicU64,
  resident_harts: AtomicU64,
}

impl AddressSpace {
  pub fn new(root_frame: u64, asid: Asid) -> Result<Self, Error> {
    if root_frame == 0 || !root_frame.is_multiple_of(4096) {
      return Err(Error::InvalidRootFrame);
    }
    Ok(Self {
      root_frame,
      asid,
      active_harts: AtomicU64::new(0),
      resident_harts: AtomicU64::new(0),
    })
  }

  pub const fn root_frame(&self) -> u64 {
    self.root_frame
  }

  pub const fn asid(&self) -> Asid {
    self.asid
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
}
