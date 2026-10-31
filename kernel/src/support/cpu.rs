use core::arch::asm;

pub fn park() -> ! {
  loop {
    // SAFETY: WFI is a hint and may wake spuriously, so it remains in a loop.
    unsafe { asm!("wfi", options(nomem, nostack)) };
  }
}
