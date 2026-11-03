#[cfg(target_arch = "riscv64")]
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_arch = "riscv64")]
static SMOKE_ENTERED: AtomicBool = AtomicBool::new(false);
#[cfg(target_arch = "riscv64")]
static SMOKE_RESUMED: AtomicBool = AtomicBool::new(false);
#[cfg(target_arch = "riscv64")]
static SMOKE_PREEMPTED: AtomicBool = AtomicBool::new(false);

#[cfg(target_arch = "riscv64")]
pub fn reset_smoke_state() {
  SMOKE_ENTERED.store(false, Ordering::Relaxed);
  SMOKE_RESUMED.store(false, Ordering::Relaxed);
  SMOKE_PREEMPTED.store(false, Ordering::Relaxed);
}

#[cfg(target_arch = "riscv64")]
pub fn smoke_entered() -> bool {
  SMOKE_ENTERED.load(Ordering::Acquire)
}

#[cfg(target_arch = "riscv64")]
pub fn smoke_resumed() -> bool {
  SMOKE_RESUMED.load(Ordering::Acquire)
}

#[cfg(target_arch = "riscv64")]
pub fn smoke_preempted() -> bool {
  SMOKE_PREEMPTED.load(Ordering::Acquire)
}

#[cfg(target_arch = "riscv64")]
pub fn smoke_entry_address() -> usize {
  smoke_entry as *const () as usize
}

#[cfg(target_arch = "riscv64")]
extern "C" fn smoke_entry() -> ! {
  SMOKE_ENTERED.store(true, Ordering::Release);
  let hart = crate::arch::riscv64::hart::current_index();
  let start_tick = crate::arch::riscv64::timer::ticks(hart)
    .unwrap_or_else(|_| panic!("smoke Thread timer state is unavailable"));
  while crate::arch::riscv64::timer::ticks(hart)
    .unwrap_or_else(|_| panic!("smoke Thread timer state is unavailable"))
    == start_tick
  {
    core::hint::spin_loop();
  }
  SMOKE_PREEMPTED.store(true, Ordering::Release);
  SMOKE_RESUMED.store(true, Ordering::Release);
  crate::kernel_state::park_current_thread()
}
