#![no_std]

#[cfg(test)]
extern crate std;

pub mod arch;
pub mod dtb;
pub mod memory;
pub mod platform;
pub mod sync;

#[cfg(target_arch = "riscv64")]
pub mod boot;
#[cfg(target_arch = "riscv64")]
pub mod support;
