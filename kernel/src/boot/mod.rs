mod report;

use core::ptr::addr_of;

use crate::arch::riscv64::{
  context, hart, ipi,
  paging::{self, KernelLayout},
  timer, trap,
};
use crate::dtb::Dtb;
use crate::kernel_state;
use crate::kernel_thread;
use crate::memory::EarlyFrameAllocator;
use crate::memory::address_space;
use crate::memory::{frame_allocator, kernel_stack};
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
  let paging = paging::build(&mut allocator, &platform, layout, kernel_region, dtb_region)
    .unwrap_or_else(|error| fail("Paging", error.as_str()));
  report::paging(&paging);

  // SAFETY: paging::build identity-maps the currently executing kernel text,
  // rodata, data, BSS/stack, and the DTB before satp is changed.
  unsafe { paging::activate(paging) }.unwrap_or_else(|error| fail("Paging", error.as_str()));
  report::paging_active();
  let asid_bits =
    paging::probe_asid_bits(paging).unwrap_or_else(|error| fail("ASID", error.as_str()));
  report::asid(asid_bits);

  let stvec = trap::install();
  ipi::enable_local();
  report::trap_ready(stvec);

  let boot_index = hart::initialize_boot_hart(&platform, hart_id)
    .unwrap_or_else(|error| fail("Harts", error.as_str()));
  timer::initialize_frequency(platform.timebase_frequency())
    .unwrap_or_else(|error| fail("Timer", error.as_str()));
  let timer_deadline =
    timer::initialize_local().unwrap_or_else(|error| fail("Timer", error.as_str()));
  let free_before = frame_allocator::initialize(&allocator)
    .unwrap_or_else(|error| fail("Runtime Memory", error.as_str()));
  let stack = kernel_stack::KernelStack::allocate()
    .unwrap_or_else(|error| fail("Kernel Stack", error.as_str()));
  let stack_top = stack.top();
  stack
    .release()
    .unwrap_or_else(|error| fail("Kernel Stack", error.as_str()));
  let free_after =
    frame_allocator::free_frames().unwrap_or_else(|error| fail("Runtime Memory", error.as_str()));
  if free_before != free_after {
    fail("runtime Memory", "kernel stack round-trip leaked frames");
  }
  report::runtime_memory(free_after, stack_top);
  let user_table =
    crate::memory::user_page_table::UserPageTable::clone_kernel_root(paging.root_frame())
      .unwrap_or_else(|error| fail("User Address Space", error.as_str()));
  let user_root = user_table.root_frame();
  let owned_frames = user_table.owned_frames();
  user_table
    .release()
    .unwrap_or_else(|error| fail("User Address Space", error.as_str()));
  let free_after_user = frame_allocator::free_frames()
    .unwrap_or_else(|error| fail("User Address Space", error.as_str()));
  if free_after_user != free_after {
    fail("User Address Space", "Page Table smoke leaked frames");
  }
  report::user_address_space(user_root, owned_frames);
  let kernel_space = kernel_state::initialize_kernel_address_space(paging.root_frame())
    .unwrap_or_else(|error| fail("Kernel Objects", error.as_str()));
  let (object_count, capability_count, capability_space_count) = kernel_state::counts();
  report::kernel_objects(
    kernel_space,
    object_count,
    capability_count,
    capability_space_count,
  );
  address_space::initialize_allocator(asid_bits)
    .unwrap_or_else(|error| fail("ASID", error.as_str()));
  hart::install_kernel_address_space();
  let requested = hart::start_secondaries(&platform, hart_id, paging)
    .unwrap_or_else(|error| fail("Harts", error.as_str()));
  report::hart_bootstrap(boot_index, requested, hart::online_count(&platform));
  let timer_tick =
    timer::run_boot_smoke_test().unwrap_or_else(|error| fail("Timer", error.as_str()));
  if !hart::current().take_reschedule_request() {
    fail(
      "Timer",
      "timer interrupt did not publish reschedule request",
    );
  }
  report::timer(platform.timebase_frequency(), timer_deadline, timer_tick);

  let (online_mask, runnable, running) =
    kernel_state::initialize_scheduler_state(hart::online_mask())
      .unwrap_or_else(|error| fail("Scheduler", error.as_str()));
  report::scheduler(online_mask, runnable, running);

  context::run_boot_smoke_test().unwrap_or_else(|error| fail("Context Switch", error.as_str()));
  report::context_switch();

  let process = kernel_state::initialize_bootstrap_process()
    .unwrap_or_else(|error| fail("Kernel Thread", error.as_str()));
  let text_memory = kernel_state::create_memory(kernel_state::bootstrap_space(), process.authority)
    .unwrap_or_else(|error| fail("Memory Object", error.as_str()));
  report::memory_object(text_memory.object, text_memory.capability);
  let stack_memory =
    kernel_state::create_memory(kernel_state::bootstrap_space(), process.authority)
      .unwrap_or_else(|error| fail("Memory Object", error.as_str()));
  let user_space = kernel_state::create_user_address_space(
    kernel_state::bootstrap_space(),
    process.authority,
    paging.root_frame(),
  )
  .unwrap_or_else(|error| fail("User Mapping", error.as_str()));
  let text_mapping = kernel_state::map_memory(
    process.space,
    user_space.capability,
    text_memory.capability,
    crate::memory::user_page_table::USER_TEXT_BASE,
    crate::arch::riscv64::paging::UserPermissions::ReadExecute,
  )
  .unwrap_or_else(|error| fail("User Mapping", error.as_str()));
  let stack_mapping = kernel_state::map_memory(
    process.space,
    user_space.capability,
    stack_memory.capability,
    crate::memory::user_page_table::USER_STACK_BASE,
    crate::arch::riscv64::paging::UserPermissions::ReadWrite,
  )
  .unwrap_or_else(|error| fail("User Mapping", error.as_str()));
  if text_mapping.address_space != stack_mapping.address_space {
    fail(
      "User Mapping",
      "user image mappings use different Address Spaces",
    );
  }
  report::user_image(text_mapping, stack_mapping);
  kernel_thread::reset_smoke_state();
  let affinity = 1u64
    .checked_shl(boot_index as u32)
    .unwrap_or_else(|| fail("Kernel Thread", "invalid boot hart affinity"));
  let thread = kernel_state::create_thread(
    kernel_state::bootstrap_space(),
    process.authority,
    affinity,
    crate::thread::MIN_PRIORITY,
    kernel_thread::smoke_entry_address(),
  )
  .unwrap_or_else(|error| fail("Kernel Thread", error.as_str()));
  let wake = kernel_state::wake_thread(thread.object)
    .unwrap_or_else(|error| fail("Kernel Thread", error.as_str()));
  if wake.target_hart != boot_index {
    fail("Kernel Thread", "smoke Thread selected a remote hart");
  }
  let dispatched = kernel_state::dispatch_current_hart_once()
    .unwrap_or_else(|error| fail("Kernel Thread", error.as_str()));
  if dispatched != Some(thread.object)
    || !kernel_thread::smoke_entered()
    || kernel_thread::smoke_preempted()
    || kernel_thread::smoke_resumed()
  {
    fail("Kernel Thread", "smoke Thread was not timer-preempted");
  }
  let (_, runnable, running) = kernel_state::global_scheduler_summary()
    .unwrap_or_else(|error| fail("Kernel Thread", error.as_str()));
  if runnable != 1 || running != 0 {
    fail("Kernel Thread", "yielded Thread was not republished");
  }

  let redispatched = kernel_state::dispatch_current_hart_once()
    .unwrap_or_else(|error| fail("Kernel Thread", error.as_str()));
  if redispatched != Some(thread.object)
    || !kernel_thread::smoke_preempted()
    || !kernel_thread::smoke_resumed()
  {
    fail("Kernel Thread", "preempted smoke Thread did not resume");
  }
  let (_, runnable, running) = kernel_state::global_scheduler_summary()
    .unwrap_or_else(|error| fail("Kernel Thread", error.as_str()));
  if runnable != 0 || running != 0 {
    fail("Kernel Thread", "smoke Thread did not park cleanly");
  }
  report::kernel_thread(thread.object, process.object, affinity);

  report::scheduler_runtime();
  report::complete();
  kernel_state::run_scheduler_loop()
}

pub fn run_secondary(hart_id: usize, hart_index: usize) -> ! {
  // secondary_entry has already selected the dedicated stack,
  // installed the shared Sv39 satp, executed SFENCE.VMA,
  // and set tp to HartLocal.
  let _stvec = trap::install();
  ipi::enable_local();
  hart::install_kernel_address_space();
  timer::initialize_local().unwrap_or_else(|_| cpu::halt());
  hart::complete_secondary(hart_id, hart_index).unwrap_or_else(|_| cpu::halt());
  kernel_state::run_scheduler_loop()
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
  cpu::halt()
}
