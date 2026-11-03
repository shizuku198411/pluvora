use crate::arch::riscv64::paging::PagingInfo;
use crate::capability::CapabilityHandle;
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

pub fn kernel_objects(
  kernel_space: CapabilityHandle,
  object_count: usize,
  capability_count: usize,
  capability_space_count: usize,
) {
  const FIELD_WIDTH: usize = 21;

  section("Kernel Objects");
  usize_field_with_width("Object capacity", crate::object::MAX_OBJECTS, FIELD_WIDTH);
  usize_field_with_width("Live objects", object_count, FIELD_WIDTH);
  usize_field_with_width(
    "CSpace capacity",
    crate::capability_space::MAX_CAPABILITY_SPACES,
    FIELD_WIDTH,
  );
  usize_field_with_width("Live CSpaces", capability_space_count, FIELD_WIDTH);
  usize_field_with_width(
    "Capability capacity",
    crate::capability::MAX_CAPABILITY_SLOTS,
    FIELD_WIDTH,
  );
  usize_field_with_width("Live capabilities", capability_count, FIELD_WIDTH);
  usize_field_with_width("Kernel cap slot", kernel_space.slot() as usize, FIELD_WIDTH);
  u64_field_with_width(
    "Kernel cap generation",
    u64::from(kernel_space.generation()),
    FIELD_WIDTH,
  );
}

pub fn scheduler(online_mask: u64, runnable: usize, running: usize) {
  section("Scheduler");
  u64_field("Online hart mask", online_mask);
  usize_field("Queue capacity", crate::scheduler::MAX_RUNNABLE_PER_HART);
  usize_field("Runnable threads", runnable);
  usize_field("Running threads", running);
}

pub fn scheduler_runtime() {
  section("Scheduler Runtime");
  console::write_line("  Ready publication : complete");
  console::write_line("  Hart loops        : enabled");
  console::write_line("  Idle policy       : interrupt-driven WFI");
  console::write_line("  Wake notification : reschedule IPI");
}

pub fn context_switch() {
  section("Context Switch");
  console::write_line("  Callee-saved ABI  : verified");
  usize_field(
    "Smoke stack bytes",
    crate::arch::riscv64::context::SMOKE_STACK_BYTES,
  );
}

pub fn timer(frequency: u64, deadline: u64, tick: u64) {
  section("Supervisor Timer");
  u64_field("Timebase frequency", frequency);
  u64_field("Tick rate", crate::arch::riscv64::timer::TIMER_HZ);
  u64_field("Initial deadline", deadline);
  u64_field("Observed ticks", tick);
  console::write_line("  Reschedule request: published");
}

pub fn kernel_thread(
  thread: crate::object::ObjectId,
  process: crate::object::ObjectId,
  affinity: u64,
) {
  section("Kernel Thread Dispatch");
  usize_field("Process identity", process.into_raw());
  usize_field("Thread identity", thread.into_raw());
  u64_field("Affinity", affinity);
  console::write_line("  Entry executed    : yes");
  console::write_line("  Timer preemption  : yes");
  console::write_line("  Resume completed  : yes");
  console::write_line("  Resume mechanism  : sret");
  console::write_line("  Final state       : blocked");
  console::write_line("  Boot context      : restored");
}

pub fn memory_object(object: crate::object::ObjectId, capability: CapabilityHandle) {
  section("Owned Memory Object");
  usize_field("Object identity", object.into_raw());
  usize_field("Capability slot", capability.slot() as usize);
  u64_field("Capability generation", u64::from(capability.generation()));
  usize_field("Page count", 1);
  console::write_line("  Allocation        : zero-filled runtime frame");
  console::write_line("  Ownership         : Memory Object");
  console::write_line("  Process quota     : charged");
}

pub fn initialized_user_image(image: crate::kernel_state::InitializedUserImage) {
  section("Initial User Image");
  usize_field("Text memory", image.memory.into_raw());
  usize_field("Image bytes", image.bytes);
  console::write_line("  Remaining bytes   : zero-filled");
  console::write_line("  Publication order : write-before-execute");
  console::write_line("  Local FENCE.I     : complete");
  console::write_line("  Remote FENCE.I    : required on first dispatch");
}

pub fn user_image(
  text: crate::kernel_state::MappedMemory,
  stack: crate::kernel_state::MappedMemory,
) {
  section("Capability User Image");
  usize_field("Text memory", text.memory.into_raw());
  u64_field("Text virtual", text.virtual_address);
  console::write_line("  Text permissions  : read-execute");
  usize_field("Stack memory", stack.memory.into_raw());
  u64_field("Stack virtual", stack.virtual_address);
  u64_field(
    "Initial stack top",
    crate::memory::user_page_table::USER_STACK_TOP,
  );
  u64_field(
    "Guard page",
    crate::memory::user_page_table::USER_GUARD_BASE,
  );
  console::write_line("  Stack permissions : read-write");
  console::write_line("  Guard mapping     : absent");
  console::write_line("  Mapping references: two held");
  console::write_line("  Shared PT path    : verified");
}

pub fn prepared_user_thread(
  attached: crate::kernel_state::AttachedAddressSpace,
  thread: crate::kernel_state::PreparedUserThread,
) {
  section("User Process");
  usize_field("Address Space", attached.current.into_raw());
  usize_field("Prepared Thread", thread.object.into_raw());
  console::write_line("  Thread state      : New");
  console::write_line("  Execution mode    : User");
}

pub fn user_dispatch(summary: crate::kernel_state::UserDispatchSummary) {
  section("User-mode Dispatch");
  usize_field("Thread", summary.thread.into_raw());
  console::write_line("  U-mode entered    : yes");
  console::write_line("  Exit mechanism    : user ecall");
  console::write_line("  Final state       : Stopped");
  u64_field("Active hart mask", summary.active_harts);
  u64_field("Resident hart mask", summary.resident_harts);
  usize_field("Current ASID", summary.current_asid);
  console::write_line("  Kernel return     : verified");
}

pub fn user_scheduling(summary: crate::kernel_state::UserPairSummary, dispatches: usize) {
  section("User Scheduling");
  usize_field("User threads", 2);
  usize_field("Dispatches", dispatches);
  u64_field("Completed yields", summary.total_yields);
  u64_field("Timer/IPI preemptions", summary.total_preemptions);
  console::write_line("  Same Process/AS   : verified");
  console::write_line("  Round robin       : verified");
  console::write_line("  Final states      : Stopped / Stopped");
  usize_field("Runnable threads", summary.runnable);
  usize_field("Running threads", summary.running);
  u64_field("Active hart mask", summary.active_harts);
}

pub fn process_termination(summary: crate::kernel_state::TerminationSummary) {
  section("Process Termination");
  console::write("  Reason            : ");
  console::write_line(summary.reason.name());
  console::write_line("  Process state     : Stopping");
  console::write("  Progress          : ");
  console::write_line(match summary.progress {
    crate::process::TerminationProgress::Pending => "Pending",
    crate::process::TerminationProgress::Quiescent => "Quiescent",
  });
  usize_field("Stopped threads", summary.stopped_threads);
  usize_field("Live threads", summary.live_threads);
  u64_field("Active hart mask", summary.active_harts);
  console::write_line("  Death notification: pending IPC endpoint");
  console::write_line("  Reclamation       : pending revocation/unmap");
  console::write_line("  Dead transition   : correctly deferred");
}

pub fn runtime_memory(free_frames: usize, stack_top: usize) {
  section("Runtime Memory");
  usize_field("Free frames", free_frames);
  usize_field(
    "Kernel stack pages",
    crate::memory::kernel_stack::KERNEL_STACK_PAGES,
  );
  usize_field("Stack top alignment", stack_top & 0xf);
}

pub fn user_address_space(root: u64, owned_frames: usize) {
  section("User Address Space");
  u64_field("Smoke root", root);
  usize_field("Owned PT frames", owned_frames);
  u64_field(
    "User window start",
    crate::memory::user_page_table::USER_WINDOW_START,
  );
  u64_field(
    "User window end",
    crate::memory::user_page_table::USER_WINDOW_END,
  );
  console::write_line("  Kernel mappings   : supervisor-only shared");
  console::write_line("  User mappings     : private U-bit subtree");
  console::write_line("  W^X policy        : enforced by type");
  console::write_line("  Release smoke     : verified");
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

pub fn asid(bits: u32) {
  let maximum = if bits == 0 { 0 } else { (1u64 << bits) - 1 };
  section("Address Spaces");
  usize_field("ASID bits", bits as usize);
  u64_field("Maximum ASID", maximum);
  usize_field("Kernel ASID", 0);
  u64_field("Generation", 0);
}

pub fn trap_ready(stvec: usize) {
  section("S-mode traps");
  usize_field("stvec", stvec);
  console::write_line("  Mode              : direct");
}

pub fn user_trap_boundary(scratch: usize) {
  section("User Trap Boundary");
  usize_field("Idle sscratch", scratch);
  console::write_line("  U-mode entry      : armed kernel stack");
  console::write_line("  Trap stack pivot  : sscratch exchange");
  console::write_line("  Handler scratch   : cleared");
  console::write_line("  User exceptions   : termination policy classified");
  console::write_line("  Runnable users    : disabled");
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
  usize_field_with_width(label, value, 18);
}

fn usize_field_with_width(label: &str, value: usize, field_width: usize) {
  console::write("  ");
  console::write(label);
  write_padding(label.len(), field_width);
  console::write(": ");
  console::write_hex_usize(value);
  console::newline();
}

fn u64_field(label: &str, value: u64) {
  u64_field_with_width(label, value, 18);
}

fn u64_field_with_width(label: &str, value: u64, field_width: usize) {
  console::write("  ");
  console::write(label);
  write_padding(label.len(), field_width);
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

fn write_padding(label_width: usize, field_width: usize) {
  for _ in label_width..field_width {
    console::write(" ");
  }
}

fn write_region_padding(label_width: usize) {
  const FIELD_WIDTH: usize = 10;
  for _ in label_width..FIELD_WIDTH {
    console::write(" ");
  }
}
