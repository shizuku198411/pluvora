use crate::arch::riscv64::paging::PagingInfo;
use crate::dtb::Dtb;
use crate::platform::{MemoryRegion, PlatformInfo};
use crate::support::console;

pub fn banner() {
  console::newline();
  console::write_line("============================================================");
  console::write_line("  Pluvora kernel bootstrap");
  console::write_line("============================================================");
}

pub fn entry(hart_id: usize, dtb_address: usize) {
  section("Entry");
  usize_field("Boot hart ID", hart_id);
  usize_field("DTB address", dtb_address);
}

pub fn device_tree(dtb: &Dtb<'_>) {
  section("Device Tree");
  usize_field("Total size", dtb.header().total_size);
  usize_field("Format version", dtb.header().version as usize);
  usize_field("Header boot CPU", dtb.header().boot_cpu_id as usize);
}

pub fn harts(platform: &PlatformInfo, boot_hart_id: usize) {
  section("Harts");
  usize_field("Discovered count", platform.harts().len());

  for (index, hart_id) in platform.harts().iter().copied().enumerate() {
    console::write("  hart: index=");
    console::write_hex_usize(index);
    console::write(", id=");
    console::write_hex_usize(hart_id);
    console::newline();
  }

  if let Some(index) = platform.hart_index(boot_hart_id) {
    usize_field("Boot hart index", index);
  }
}

pub fn memory_map(platform: &PlatformInfo, kernel_region: MemoryRegion, dtb_region: MemoryRegion) {
  section("Physical Memory");
  console::write_line("  Available RAM:");
  for region in platform.memory_regions() {
    region_field("RAM", *region);
  }

  console::write_line("  Firmware reservations:");
  for region in platform.reserved_regions() {
    region_field("reserved", *region);
  }

  console::write_line("  Kernel reservations:");
  region_field("kernel", kernel_region);
  region_field("DTB", dtb_region);
}

pub fn paging(paging: &PagingInfo) {
  section("Sv39 Paging");
  u64_field("Root frame", paging.root_frame());
  usize_field("Page-table frames", paging.page_table_frames());
  usize_field("Mapped pages", paging.mapped_pages());
  console::write_line("  Permissions       : text=R-X rodata=R-- data/bss=RW-");
}

pub fn paging_active() {
  console::write_line("  Translation       : active (identity mapped)");
}

pub fn trap_ready(stvec: usize) {
  section("S-mode traps");
  usize_field("stvec", stvec);
  console::write_line("  Mode              : direct");
}

pub fn hart_bootstrap(boot_index: usize, requested: usize, online: usize) {
  section("Multi-hart Bootstrap");
  usize_field("Boot hart index", boot_index);
  usize_field("Start requests", requested);
  usize_field("Online count", online);
}

pub fn complete() {
  section("Status");
  console::write_line("  Early bootstrap completed successfully");
  console::write_line("============================================================");
}

pub fn error(stage: &str, message: &str) {
  section("Boot Failure");
  console::write("  Stage               : ");
  console::write_line(stage);
  console::write("  Error               : ");
  console::write_line(message);
  console::write_line("============================================================");
}

fn section(title: &str) {
  console::newline();
  console::write("[");
  console::write(title);
  console::write_line("]");
}

fn usize_field(label: &str, value: usize) {
  console::write("  ");
  console::write(label);
  write_padding(label.len());
  console::write(": ");
  console::write_hex_usize(value);
  console::newline();
}

fn u64_field(label: &str, value: u64) {
  console::write("  ");
  console::write(label);
  write_padding(label.len());
  console::write(": ");
  console::write_hex_u64(value);
  console::newline();
}

fn region_field(label: &str, region: MemoryRegion) {
  console::write("    ");
  console::write(label);
  write_region_padding(label.len());
  console::write(": base=");
  console::write_hex_u64(region.base);
  console::write(", size=");
  console::write_hex_u64(region.size);
  console::newline();
}

fn write_padding(label_width: usize) {
  const FIELD_WIDTH: usize = 18;
  for _ in label_width..FIELD_WIDTH {
    console::write(" ");
  }
}

fn write_region_padding(label_width: usize) {
  const FIELD_WIDTH: usize = 10;
  for _ in label_width..FIELD_WIDTH {
    console::write(" ");
  }
}
