mod report;

use core::ptr::addr_of;

use crate::dtb::Dtb;
use crate::memory::EarlyFrameAllocator;
use crate::platform::{MemoryRegion, PlatformInfo, discover};
use crate::support::cpu;

const ALLOCATOR_SMOKE_TEST_FRAMES: usize = 4;

unsafe extern "C" {
  static __kernel_start: u8;
  static __kernel_end: u8;
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

  report::allocator_header();
  for index in 0..ALLOCATOR_SMOKE_TEST_FRAMES {
    let frame = allocator
      .allocate_zeroed()
      .unwrap_or_else(|error| fail("Memory", error.as_str()))
      .unwrap_or_else(|| fail("Memory", "no free physical frame"));
    report::allocated_frame(index, frame.start());
  }

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

fn fail(stage: &str, message: &str) -> ! {
  report::error(stage, message);
  cpu::park()
}
