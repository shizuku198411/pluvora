#[cfg(target_arch = "riscv64")]
use core::arch::asm;
#[cfg(target_arch = "riscv64")]
use core::hint::spin_loop;
use core::mem::{offset_of, size_of};
use core::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::paging::PagingInfo;
use crate::arch::riscv64::sbi;
use crate::platform::{MAX_HARTS, PlatformInfo};

#[cfg(target_arch = "riscv64")]
const SECONDARY_STACK_SIZE: usize = 16 * 1024;
#[cfg(target_arch = "riscv64")]
const START_WAIT_LIMIT: usize = 10_000_000;

const HART_OFFLINE: usize = 0;
const HART_STARTING: usize = 1;
const HART_ONLINE: usize = 2;

const BOOT_INFO_EMPTY: usize = 0;
#[cfg(target_arch = "riscv64")]
const BOOT_INFO_STARTING: usize = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  InvalidHartIndex,
  InvalidStackLayout,
  InvalidSatp,
  HartIdMismatch,
  AlreadyInitialized,
  StartTimeout,
  Sbi(sbi::Error),
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::InvalidHartIndex => "invalid dense hart index",
      Self::InvalidStackLayout => "invalid secondary stack layout",
      Self::InvalidSatp => "satp value does not fit the target register",
      Self::HartIdMismatch => "secondary hart ID does not match boot descriptor",
      Self::AlreadyInitialized => "hart state is already initiated",
      Self::StartTimeout => "secondary hart did not become online",
      Self::Sbi(error) => error.as_str(),
    }
  }
}

impl From<sbi::Error> for Error {
  fn from(error: sbi::Error) -> Self {
    Self::Sbi(error)
  }
}

#[repr(C, align(64))]
pub struct HartLocal {
  raw_hart_id: AtomicUsize,
  dense_index: AtomicUsize,
  state: AtomicUsize,
  current_thread: AtomicUsize,
  interrupt_state: AtomicUsize,
  shootdown_queue: AtomicUsize,
}

impl HartLocal {
  const fn empty() -> Self {
    Self {
      raw_hart_id: AtomicUsize::new(usize::MAX),
      dense_index: AtomicUsize::new(usize::MAX),
      state: AtomicUsize::new(HART_OFFLINE),
      current_thread: AtomicUsize::new(0),
      interrupt_state: AtomicUsize::new(0),
      shootdown_queue: AtomicUsize::new(0),
    }
  }
}

#[repr(C)]
struct SecondaryBootInfo {
  stack_top: AtomicUsize,
  satp: AtomicUsize,
  hart_index: AtomicUsize,
  expected_hart_id: AtomicUsize,
  hart_local_pointer: AtomicUsize,
  publication_state: AtomicUsize,
}

impl SecondaryBootInfo {
  const fn empty() -> Self {
    Self {
      stack_top: AtomicUsize::new(0),
      satp: AtomicUsize::new(0),
      hart_index: AtomicUsize::new(usize::MAX),
      expected_hart_id: AtomicUsize::new(0),
      hart_local_pointer: AtomicUsize::new(0),
      publication_state: AtomicUsize::new(BOOT_INFO_EMPTY),
    }
  }
}

static HARTS: [HartLocal; MAX_HARTS] = [const { HartLocal::empty() }; MAX_HARTS];
#[cfg(target_arch = "riscv64")]
static BOOT_INFO: [SecondaryBootInfo; MAX_HARTS] =
  [const { SecondaryBootInfo::empty() }; MAX_HARTS];

#[cfg(target_arch = "riscv64")]
unsafe extern "C" {
  static __secondary_stacks_start: u8;
  static __secondary_stacks_end: u8;
  fn secondary_entry();
}

#[cfg(target_arch = "riscv64")]
pub fn initialize_boot_hart(platform: &PlatformInfo, boot_hart_id: usize) -> Result<usize, Error> {
  initialize_descriptors(platform)?;
  let boot_index = platform
    .hart_index(boot_hart_id)
    .ok_or(Error::InvalidHartIndex)?;
  let local = HARTS.get(boot_index).ok_or(Error::InvalidHartIndex)?;
  local.state.store(HART_ONLINE, Ordering::Release);
  install_hart_local_pointer(local);
  Ok(boot_index)
}

#[cfg(target_arch = "riscv64")]
pub fn start_secondaries(
  platform: &PlatformInfo,
  boot_hart_id: usize,
  paging: PagingInfo,
) -> Result<usize, Error> {
  validate_stack_layout()?;
  let satp_value = paging.satp_value().map_err(|_| Error::InvalidSatp)?;
  let satp = usize::try_from(satp_value).map_err(|_| Error::InvalidSatp)?;
  let mut requested = 0usize;

  for (index, hart_id) in platform.harts().iter().copied().enumerate() {
    if hart_id == boot_hart_id {
      continue;
    }

    let local = HARTS.get(index).ok_or(Error::InvalidHartIndex)?;
    if local
      .state
      .compare_exchange(
        HART_OFFLINE,
        HART_STARTING,
        Ordering::AcqRel,
        Ordering::Acquire,
      )
      .is_err()
    {
      return Err(Error::AlreadyInitialized);
    }

    let info = BOOT_INFO.get(index).ok_or(Error::InvalidHartIndex)?;
    info.stack_top.store(stack_top(index)?, Ordering::Relaxed);
    info.satp.store(satp, Ordering::Relaxed);
    info.hart_index.store(index, Ordering::Relaxed);
    info.expected_hart_id.store(hart_id, Ordering::Relaxed);
    info
      .hart_local_pointer
      .store(local as *const HartLocal as usize, Ordering::Relaxed);
    info
      .publication_state
      .store(BOOT_INFO_STARTING, Ordering::Release);

    sbi::hart_start(
      hart_id,
      secondary_entry as *const () as usize,
      info as *const SecondaryBootInfo as usize,
    )?;
    requested += 1;
  }

  wait_for_online(platform, boot_hart_id)?;
  Ok(requested)
}

pub fn complete_secondary(hart_id: usize, index: usize) -> Result<(), Error> {
  let local = HARTS.get(index).ok_or(Error::InvalidHartIndex)?;
  complete_secondary_local(local, hart_id, index)
}

fn complete_secondary_local(local: &HartLocal, hart_id: usize, index: usize) -> Result<(), Error> {
  if local.raw_hart_id.load(Ordering::Relaxed) != hart_id
    || local.dense_index.load(Ordering::Relaxed) != index
  {
    return Err(Error::HartIdMismatch);
  }

  local
    .state
    .compare_exchange(
      HART_STARTING,
      HART_ONLINE,
      Ordering::Release,
      Ordering::Acquire,
    )
    .map_err(|_| Error::AlreadyInitialized)?;
  Ok(())
}

pub fn online_count(platform: &PlatformInfo) -> usize {
  count_online(&HARTS[..platform.harts().len()])
}

fn count_online(harts: &[HartLocal]) -> usize {
  harts
    .iter()
    .filter(|hart| hart.state.load(Ordering::Acquire) == HART_ONLINE)
    .count()
}

#[cfg(target_arch = "riscv64")]
fn initialize_descriptors(platform: &PlatformInfo) -> Result<(), Error> {
  for (index, hart_id) in platform.harts().iter().copied().enumerate() {
    let local = HARTS.get(index).ok_or(Error::InvalidHartIndex)?;
    if local.state.load(Ordering::Acquire) != HART_OFFLINE {
      return Err(Error::AlreadyInitialized);
    }
    local.raw_hart_id.store(hart_id, Ordering::Relaxed);
    local.dense_index.store(index, Ordering::Relaxed);
  }
  Ok(())
}

#[cfg(target_arch = "riscv64")]
fn wait_for_online(platform: &PlatformInfo, boot_hart_id: usize) -> Result<(), Error> {
  for (index, hart_id) in platform.harts().iter().copied().enumerate() {
    if hart_id == boot_hart_id {
      continue;
    }

    let state = &HARTS[index].state;
    for _ in 0..START_WAIT_LIMIT {
      if state.load(Ordering::Acquire) == HART_ONLINE {
        break;
      }
      spin_loop();
    }
    if state.load(Ordering::Acquire) != HART_ONLINE {
      return Err(Error::StartTimeout);
    }
  }
  Ok(())
}

#[cfg(target_arch = "riscv64")]
fn stack_top(index: usize) -> Result<usize, Error> {
  let start = core::ptr::addr_of!(__secondary_stacks_start) as usize;
  start
    .checked_add(stack_offset(index)?)
    .ok_or(Error::InvalidStackLayout)
}

fn stack_offset(index: usize) -> Result<usize, Error> {
  if index >= MAX_HARTS {
    return Err(Error::InvalidHartIndex);
  }
  index
    .checked_add(1)
    .and_then(|slots| slots.checked_mul(16 * 1024))
    .ok_or(Error::InvalidStackLayout)
}

#[cfg(target_arch = "riscv64")]
fn validate_stack_layout() -> Result<(), Error> {
  let start = core::ptr::addr_of!(__secondary_stacks_start) as usize;
  let end = core::ptr::addr_of!(__secondary_stacks_end) as usize;
  let expected = MAX_HARTS
    .checked_mul(SECONDARY_STACK_SIZE)
    .ok_or(Error::InvalidStackLayout)?;
  if end.checked_sub(start) != Some(expected) {
    return Err(Error::InvalidStackLayout);
  }
  Ok(())
}

#[cfg(target_arch = "riscv64")]
fn install_hart_local_pointer(local: &HartLocal) {
  let pointer = local as *const HartLocal as usize;
  unsafe { asm!("mv tp, {pointer}", pointer = in(reg) pointer, options(nostack)) };
}

const _: () = {
  assert!(size_of::<SecondaryBootInfo>() == 48);
  assert!(offset_of!(SecondaryBootInfo, stack_top) == 0);
  assert!(offset_of!(SecondaryBootInfo, satp) == 8);
  assert!(offset_of!(SecondaryBootInfo, hart_index) == 16);
  assert!(offset_of!(SecondaryBootInfo, expected_hart_id) == 24);
  assert!(offset_of!(SecondaryBootInfo, hart_local_pointer) == 32);
  assert!(offset_of!(SecondaryBootInfo, publication_state) == 40);
};

#[cfg(test)]
mod tests {
  use super::*;

  fn configured_local(hart_id: usize, index: usize, state: usize) -> HartLocal {
    let local = HartLocal::empty();
    local.raw_hart_id.store(hart_id, Ordering::Relaxed);
    local.dense_index.store(index, Ordering::Relaxed);
    local.state.store(state, Ordering::Relaxed);
    local
  }

  #[test]
  fn hart_003_secondary_stack_slots_are_unique_and_bounded() {
    assert_eq!(stack_offset(0), Ok(16 * 1024));
    assert_eq!(stack_offset(1), Ok(32 * 1024));
    assert_eq!(stack_offset(MAX_HARTS - 1), Ok(MAX_HARTS * 16 * 1024));
    assert_eq!(stack_offset(MAX_HARTS), Err(Error::InvalidHartIndex));
  }

  #[test]
  fn hart_007_each_hart_has_distinct_local_storage() {
    let harts = [const { HartLocal::empty() }; 2];
    assert_ne!(&harts[0] as *const HartLocal, &harts[1] as *const HartLocal);
    assert_eq!(size_of::<HartLocal>(), 64);
  }

  #[test]
  fn hart_008_online_count_only_accepts_online_state() {
    let harts = [
      configured_local(0, 0, HART_ONLINE),
      configured_local(1, 1, HART_STARTING),
      configured_local(2, 2, HART_OFFLINE),
      configured_local(3, 3, HART_ONLINE),
    ];
    assert_eq!(count_online(&harts), 2);
  }

  #[test]
  fn hart_011_secondary_boot_info_layout_matches_assembly_contract() {
    assert_eq!(size_of::<SecondaryBootInfo>(), 48);
    assert_eq!(offset_of!(SecondaryBootInfo, stack_top), 0);
    assert_eq!(offset_of!(SecondaryBootInfo, satp), 8);
    assert_eq!(offset_of!(SecondaryBootInfo, hart_index), 16);
    assert_eq!(offset_of!(SecondaryBootInfo, expected_hart_id), 24);
    assert_eq!(offset_of!(SecondaryBootInfo, hart_local_pointer), 32);
    assert_eq!(offset_of!(SecondaryBootInfo, publication_state), 40);

    let info = SecondaryBootInfo::empty();
    assert_eq!(
      info.publication_state.load(Ordering::Relaxed),
      BOOT_INFO_EMPTY
    );
  }

  #[test]
  fn hart_013_only_matching_starting_hart_can_become_online() {
    let local = configured_local(9, 2, HART_STARTING);
    assert_eq!(complete_secondary_local(&local, 9, 2), Ok(()));
    assert_eq!(local.state.load(Ordering::Acquire), HART_ONLINE);
    assert_eq!(
      complete_secondary_local(&local, 9, 2),
      Err(Error::AlreadyInitialized)
    );

    let mismatched = configured_local(9, 2, HART_STARTING);
    assert_eq!(
      complete_secondary_local(&mismatched, 10, 2),
      Err(Error::HartIdMismatch)
    );
    assert_eq!(mismatched.state.load(Ordering::Relaxed), HART_STARTING);
  }
}
