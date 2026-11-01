#![no_main]
#![no_std]

mod arch;
mod boot;
mod dtb;
mod memory;
mod platform;
mod support;

use core::arch::global_asm;
use core::panic::PanicInfo;

use support::{console, cpu};

global_asm!(include_str!("entry.S"));
global_asm!(include_str!("arch/riscv64/secondary.S"));
global_asm!(include_str!("arch/riscv64/trap.S"));

#[unsafe(no_mangle)]
extern "C" fn kernel_entry(hart_id: usize, dtb_address: usize) -> ! {
  boot::run(hart_id, dtb_address)
}

#[unsafe(no_mangle)]
extern "C" fn secondary_kernel_entry(hart_id: usize, hart_index: usize) -> ! {
  boot::run_secondary(hart_id, hart_index)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
  console::newline();
  console::write_line("[PANIC] unrecoverable kernel error during bootstrap");
  cpu::park()
}
