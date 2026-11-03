use core::arch::asm;

use crate::arch::riscv64::hart;

pub fn halt() -> ! {
  loop {
    // SAFETY: WFI is a hint and may wake spuriously, so it remains in a loop.
    unsafe { asm!("wfi", options(nomem, nostack)) };
  }
}

pub fn wait_for_interrupt() {
  hart::run_with_local_interrupts_enabled(|| {
    let local = hart::current();
    if !local.reschedule_requested() && !local.has_ipi_requests() {
      // WFI may return spuriously. The scheduler loop always rechecks state.
      unsafe { asm!("wfi", options(nomem, nostack)) };
    }
  });
}
