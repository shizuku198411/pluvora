#[cfg(target_arch = "riscv64")]
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::platform::MAX_HARTS;

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::{hart, sbi};

pub const TIMER_HZ: u64 = 100;

static TIMEBASE_FREQUENCY: AtomicU64 = AtomicU64::new(0);
static TICKS: [AtomicU64; MAX_HARTS] = [const { AtomicU64::new(0) }; MAX_HARTS];
static DEADLINES: [AtomicU64; MAX_HARTS] = [const { AtomicU64::new(0) }; MAX_HARTS];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  InvalidFrequency,
  AlreadyInitialized,
  DeadlineOverflow,
  InvalidHartIndex,
  Sbi(crate::arch::riscv64::sbi::Error),
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::InvalidFrequency => "invalid timer frequency",
      Self::AlreadyInitialized => "timer frequency is already initialized",
      Self::DeadlineOverflow => "timer deadline overflow",
      Self::InvalidHartIndex => "invalid timer hart index",
      Self::Sbi(error) => error.as_str(),
    }
  }
}

impl From<crate::arch::riscv64::sbi::Error> for Error {
  fn from(error: crate::arch::riscv64::sbi::Error) -> Self {
    Self::Sbi(error)
  }
}

pub fn interval_ticks(frequency: u64) -> Result<u64, Error> {
  if frequency < TIMER_HZ {
    return Err(Error::InvalidFrequency);
  }
  let interval = frequency / TIMER_HZ;
  if interval == 0 {
    return Err(Error::InvalidFrequency);
  }
  Ok(interval)
}

pub fn initialize_frequency(frequency: u64) -> Result<(), Error> {
  interval_ticks(frequency)?;
  TIMEBASE_FREQUENCY
    .compare_exchange(0, frequency, Ordering::AcqRel, Ordering::Acquire)
    .map(|_| ())
    .map_err(|_| Error::AlreadyInitialized)
}

pub fn frequency() -> u64 {
  TIMEBASE_FREQUENCY.load(Ordering::Acquire)
}

pub fn ticks(hart_index: usize) -> Result<u64, Error> {
  TICKS
    .get(hart_index)
    .map(|ticks| ticks.load(Ordering::Acquire))
    .ok_or(Error::InvalidHartIndex)
}

pub fn deadline(hart_index: usize) -> Result<u64, Error> {
  DEADLINES
    .get(hart_index)
    .map(|deadline| deadline.load(Ordering::Acquire))
    .ok_or(Error::InvalidHartIndex)
}

#[cfg(target_arch = "riscv64")]
pub fn initialize_local() -> Result<u64, Error> {
  let deadline = arm_next()?;
  enable_local_timer_interrupt();
  Ok(deadline)
}

#[cfg(target_arch = "riscv64")]
pub fn handle_local() {
  let index = hart::current_index();
  arm_next().unwrap_or_else(|_| panic!("failed to rearm local timer"));
  TICKS[index].fetch_add(1, Ordering::Release);
  hart::current().request_reschedule();
}

#[cfg(target_arch = "riscv64")]
pub fn run_boot_smoke_test() -> Result<u64, Error> {
  let index = hart::current_index();
  let before = ticks(index)?;
  hart::run_with_local_interrupts_enabled(|| {
    while TICKS[index].load(Ordering::Acquire) == before {
      core::hint::spin_loop();
    }
  });
  ticks(index)
}

#[cfg(target_arch = "riscv64")]
fn arm_next() -> Result<u64, Error> {
  let index = hart::current_index();
  let interval = interval_ticks(frequency())?;
  let deadline = read_time()
    .checked_add(interval)
    .ok_or(Error::DeadlineOverflow)?;
  sbi::set_timer(deadline)?;
  DEADLINES[index].store(deadline, Ordering::Release);
  Ok(deadline)
}

#[cfg(target_arch = "riscv64")]
fn read_time() -> u64 {
  let value: u64;
  unsafe { asm!("rdtime {value}", value = out(reg) value, options(nomem, nostack)) };
  value
}

#[cfg(target_arch = "riscv64")]
fn enable_local_timer_interrupt() {
  const SIE_STIE: usize = 1 << 5;
  unsafe { asm!("csrs sie, {mask}", mask = in(reg) SIE_STIE, options(nomem, nostack)) };
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn timer_001_interval_uses_platform_frequency() {
    assert_eq!(interval_ticks(10_000_000), Ok(100_000));
  }

  #[test]
  fn timer_002_frequency_below_tick_rate_is_rejected() {
    assert_eq!(interval_ticks(TIMER_HZ - 1), Err(Error::InvalidFrequency));
  }

  #[test]
  fn timer_003_invalid_hart_index_is_rejected() {
    assert_eq!(ticks(MAX_HARTS), Err(Error::InvalidHartIndex));
    assert_eq!(deadline(MAX_HARTS), Err(Error::InvalidHartIndex));
  }
}
