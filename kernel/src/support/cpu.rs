use core::arch::asm;

use crate::arch::riscv64::{hart, ipi};

pub fn halt() -> ! {
  loop {
    // SAFETY: WFI is a hint and may wake spuriously, so it remains in a loop.
    unsafe { asm!("wfi", options(nomem, nostack)) };
  }
}

pub fn idle() -> ! {
  loop {
    if hart::current().has_ipi_requests() {
      ipi::handle_local();
      continue;
    }
    // SSIE is enabled. A pending supervisor software interrupt wakes WFI
    // even while sstatus.SIE remains clear; the loop then polls the mailbox.
    unsafe { asm!("wfi", options(nomem, nostack)) };
  }
}
