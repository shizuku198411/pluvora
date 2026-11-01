mod report;

use core::ptr::addr_of;

use crate::arch::riscv64::{
  paging::{self, KernelLayout},
  trap,
};
use crate::dtb::Dtb;
use crate::memory::EarlyFrameAllocator;
use crate::platform::{MemoryRegion, PlatformInfo, discover};
use crate::support::cpu;

unsafe extern "C" {
  static __kernel_start: u8;
  static __kernel_end: u8;
  static __text_start: u8;
  static __text_end: u8;
  static __rodata_start: u8;
  static __rodata_end: u8;
  static __data_start: u8;
  static __data_end: u8;
  static __bss_start: u8;
  static __bss_end: u8;
}

pub fn run(hart_id: usize, dtb_address: usize) -> ! {
  report::banner();
  report::entry(hart_id, dtb_address);

  let dtb = load_dtb(dtb_address);
  report::device_tree(&dtb);

  let platform = discover_platform(&dtb, hart_id);
  report::harts(&platform, hart_id);

  let kernel_region =
    kernel_region().unwrap_or_else(|| fail("Memory", "invalid kernel image region"));
  let dtb_region = dtb_region(dtb_address, dtb.header().total_size)
    .unwrap_or_else(|| fail("Memory", "invalid DTB region"));
  report::memory_map(&platform, kernel_region, dtb_region);

  let mut allocator = EarlyFrameAllocator::new(&platform, kernel_region, dtb_region)
    .unwrap_or_else(|error| fail("Memory", error.as_str()));

  let layout = kernel_layout().unwrap_or_else(|| fail("Paging", "invalid kernel layout"));
  let paging = paging::build(&mut allocator, layout, dtb_region)
    .unwrap_or_else(|error| fail("Paging", error.as_str()));
  report::paging(&paging);

  // SAFETY: paging::build identity-maps the currently executing kernel text,
  // rodata, data, BSS/stack, and the DTB before satp is changed.
  unsafe { paging::activate(paging) }.unwrap_or_else(|error| fail("Paging", error.as_str()));
  report::paging_active();

  let stvec = trap::install();
  report::trap_ready(stvec);

  report::complete();
  cpu::park()
}

fn load_dtb(dtb_address: usize) -> Dtb<'static> {
  // SAFETY: OpenSBI supplies an immutable FDT blob in a1. Boot memory remains
  // directly mapped and the DTB is explicitly reserved for this boot phase.
  unsafe { Dtb::from_ptr(dtb_address) }.unwrap_or_else(|error| fail("Device Tree", error.as_str()))
}

fn discover_platform(dtb: &Dtb<'_>, hart_id: usize) -> PlatformInfo {
  discover(dtb, hart_id).unwrap_or_else(|error| fail("Platform", error.as_str()))
}

fn kernel_region() -> Option<MemoryRegion> {
  let start = addr_of!(__kernel_start) as u64;
  let end = addr_of!(__kernel_end) as u64;
  let size = end.checked_sub(start)?;
  (size != 0).then_some(MemoryRegion { base: start, size })
}

fn dtb_region(dtb_address: usize, dtb_size: usize) -> Option<MemoryRegion> {
  let base = dtb_address as u64;
  let size = dtb_size as u64;
  base.checked_add(size)?;
  (size != 0).then_some(MemoryRegion { base, size })
}

fn kernel_layout() -> Option<KernelLayout> {
  Some(KernelLayout {
    text: symbol_region(addr_of!(__text_start), addr_of!(__text_end))?,
    rodata: symbol_region(addr_of!(__rodata_start), addr_of!(__rodata_end))?,
    data: symbol_region(addr_of!(__data_start), addr_of!(__data_end)),
    bss: symbol_region(addr_of!(__bss_start), addr_of!(__bss_end))?,
  })
}

fn symbol_region(start: *const u8, end: *const u8) -> Option<MemoryRegion> {
  let start = start as u64;
  let end = end as u64;
  let size = end.checked_sub(start)?;
  (size != 0).then_some(MemoryRegion { base: start, size })
}

fn fail(stage: &str, message: &str) -> ! {
  report::error(stage, message);
  cpu::park()
}
