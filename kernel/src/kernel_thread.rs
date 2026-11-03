#[cfg(target_arch = "riscv64")]
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_arch = "riscv64")]
static SMOKE_ENTERED: AtomicBool = AtomicBool::new(false);
#[cfg(target_arch = "riscv64")]
static SMOKE_RESUMED: AtomicBool = AtomicBool::new(false);

#[cfg(target_arch = "riscv64")]
pub fn reset_smoke_state() {
  SMOKE_ENTERED.store(false, Ordering::Relaxed);
  SMOKE_RESUMED.store(false, Ordering::Relaxed);
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
pub fn smoke_entry_address() -> usize {
  smoke_entry as *const () as usize
}

#[cfg(target_arch = "riscv64")]
extern "C" fn smoke_entry() -> ! {
  SMOKE_ENTERED.store(true, Ordering::Release);
  crate::kernel_state::yield_current_thread();
  SMOKE_RESUMED.store(true, Ordering::Release);
  crate::kernel_state::park_current_thread()
}
