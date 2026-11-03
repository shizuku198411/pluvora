#[cfg(target_arch = "riscv64")]
use crate::platform::MAX_HARTS;
#[cfg(target_arch = "riscv64")]
use core::cell::UnsafeCell;
use core::mem::{offset_of, size_of};

#[cfg(target_arch = "riscv64")]
use core::{
  ptr::{addr_of, addr_of_mut},
  sync::atomic::{AtomicBool, Ordering},
};

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::hart;

pub const KERNEL_STACK_ALIGNMENT: usize = 16;
#[cfg(target_arch = "riscv64")]
pub const SMOKE_STACK_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  InvalidHartIndex,
  InvalidEntry,
  InvalidStackTop,
  HartLocalChanged,
  SmokeDidNotReturn,
  InvalidTrapFrame,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::InvalidHartIndex => "invalid scheduler context hart index",
      Self::InvalidEntry => "context entry address is invalid",
      Self::InvalidStackTop => "context stack top is invalid",
      Self::HartLocalChanged => "context switch changed the hart-local identity",
      Self::SmokeDidNotReturn => "context switch smoke test did not return",
      Self::InvalidTrapFrame => "trap frame context is invalid",
    }
  }
}

#[repr(C)]
pub struct SavedContext {
  ra: usize,
  sp: usize,
  s: [usize; 12], // s0-s11
}

impl SavedContext {
  pub const fn empty() -> Self {
    Self {
      ra: 0,
      sp: 0,
      s: [0; 12],
    }
  }

  pub fn initial(entry: usize, stack_top: usize) -> Result<Self, Error> {
    if entry == 0 {
      return Err(Error::InvalidEntry);
    }
    if stack_top == 0 || !stack_top.is_multiple_of(KERNEL_STACK_ALIGNMENT) {
      return Err(Error::InvalidStackTop);
    }
    Ok(Self {
      ra: entry,
      sp: stack_top,
      s: [0; 12],
    })
  }

  #[cfg(target_arch = "riscv64")]
  pub(crate) fn trap_resume(frame: usize) -> Result<Self, Error> {
    validate_trap_frame(frame)?;
    Ok(Self {
      ra: resume_trap_context as *const () as usize,
      sp: frame,
      s: [0; 12],
    })
  }

  pub const fn stack_pointer(&self) -> usize {
    self.sp
  }
}

fn validate_trap_frame(frame: usize) -> Result<(), Error> {
  if frame == 0 || !frame.is_multiple_of(KERNEL_STACK_ALIGNMENT) {
    return Err(Error::InvalidTrapFrame);
  }
  Ok(())
}

const _: () = {
  assert!(size_of::<SavedContext>() == 14 * size_of::<usize>());
  assert!(offset_of!(SavedContext, ra) == 0);
  assert!(offset_of!(SavedContext, sp) == size_of::<usize>());
  assert!(offset_of!(SavedContext, s) == 2 * size_of::<usize>());
};

#[cfg(target_arch = "riscv64")]
struct SchedulerContexts([UnsafeCell<SavedContext>; MAX_HARTS]);

#[cfg(target_arch = "riscv64")]
// SAFETY: each slot is mutated only by the hart with the matching dense index.
unsafe impl Sync for SchedulerContexts {}

#[cfg(target_arch = "riscv64")]
static SCHEDULER_CONTEXTS: SchedulerContexts =
  SchedulerContexts([const { UnsafeCell::new(SavedContext::empty()) }; MAX_HARTS]);

#[cfg(target_arch = "riscv64")]
pub(crate) fn scheduler_context(hart: usize) -> Result<*mut SavedContext, Error> {
  SCHEDULER_CONTEXTS
    .0
    .get(hart)
    .map(UnsafeCell::get)
    .ok_or(Error::InvalidHartIndex)
}

#[cfg(target_arch = "riscv64")]
pub(crate) unsafe fn switch_saved_context(
  outgoing: *mut SavedContext,
  incoming: *const SavedContext,
) {
  assert!(!outgoing.is_null(), "outgoing context pointer is null");
  assert!(!incoming.is_null(), "incoming context pointer is null");
  assert_ne!(outgoing.cast_const(), incoming, "context pointers alias");
  // SAFETY: the caller guarantees exclusive outgoing access and a live incoming context.
  unsafe { switch_context(outgoing, incoming) };
}

#[cfg(target_arch = "riscv64")]
pub(crate) unsafe fn switch_to_thread(outgoing: *mut SavedContext, incoming: *const SavedContext) {
  assert!(!outgoing.is_null(), "outgoing context pointer is null");
  assert!(!incoming.is_null(), "incoming context pointer is null");
  assert_ne!(outgoing.cast_const(), incoming, "context pointers alias");
  // SAFETY: caller owns both contexts and enters with scheduler interrupts masked.
  unsafe { switch_context_to_thread(outgoing, incoming) };
}

#[cfg(target_arch = "riscv64")]
pub(crate) unsafe fn switch_to_user(outgoing: *mut SavedContext, incoming: *const SavedContext) {
  assert!(!outgoing.is_null(), "outgoing context pointer is null");
  assert!(!incoming.is_null(), "incoming context pointer is null");
  assert_ne!(outgoing.cast_const(), incoming, "context pointers alias");
  // SAFETY: caller owns both contexts and keeps SIE masked until the incoming
  // TrapFrame performs SRET. SPIE enables interrupts only after U-mode entry.
  unsafe { switch_context_to_user(outgoing, incoming) };
}

#[cfg(target_arch = "riscv64")]
pub(crate) unsafe fn restore_scheduler_context(hart: usize) -> ! {
  let scheduler =
    scheduler_context(hart).unwrap_or_else(|_| panic!("scheduler context is unavailable"));
  // SAFETY: caller transferred execution ownership away from the current Thread.
  unsafe { restore_context(scheduler.cast_const()) }
}

#[cfg(target_arch = "riscv64")]
unsafe extern "C" {
  fn switch_context(old: *mut SavedContext, new: *const SavedContext);
  fn switch_context_to_thread(old: *mut SavedContext, new: *const SavedContext);
  fn switch_context_to_user(old: *mut SavedContext, new: *const SavedContext);
  fn restore_context(new: *const SavedContext) -> !;
  fn resume_trap_context() -> !;
}

#[cfg(target_arch = "riscv64")]
#[repr(C, align(16))]
struct SmokeStack([u8; SMOKE_STACK_BYTES]);

#[cfg(target_arch = "riscv64")]
static mut BOOT_CONTEXT: SavedContext = SavedContext::empty();
#[cfg(target_arch = "riscv64")]
static mut SMOKE_CONTEXT: SavedContext = SavedContext::empty();
#[cfg(target_arch = "riscv64")]
static mut SMOKE_STACK: SmokeStack = SmokeStack([0; SMOKE_STACK_BYTES]);
#[cfg(target_arch = "riscv64")]
static SMOKE_ENTERED: AtomicBool = AtomicBool::new(false);

#[cfg(target_arch = "riscv64")]
pub fn run_boot_smoke_test() -> Result<(), Error> {
  SMOKE_ENTERED.store(false, Ordering::Relaxed);
  let hart_before = hart::current_index();
  let stack_base = addr_of!(SMOKE_STACK) as usize;
  let stack_top = stack_base
    .checked_add(SMOKE_STACK_BYTES)
    .ok_or(Error::InvalidStackTop)?;
  let context = SavedContext::initial(smoke_entry as *const () as usize, stack_top)?;

  // SAFETY: boot executes this test once before scheduler dispatch.
  // the static contexts and stack are exclusively owned for the complete switch pair.
  unsafe {
    addr_of_mut!(SMOKE_CONTEXT).write(context);
    switch_context(addr_of_mut!(BOOT_CONTEXT), addr_of!(SMOKE_CONTEXT));
  }

  if !SMOKE_ENTERED.load(Ordering::Acquire) {
    return Err(Error::SmokeDidNotReturn);
  }
  if hart::current_index() != hart_before {
    return Err(Error::HartLocalChanged);
  }
  Ok(())
}

#[cfg(target_arch = "riscv64")]
extern "C" fn smoke_entry() -> ! {
  SMOKE_ENTERED.store(true, Ordering::Release);
  unsafe {
    switch_context(addr_of_mut!(SMOKE_CONTEXT), addr_of!(BOOT_CONTEXT));
  }
  unreachable!("completed smoke context resumed unexpectedly")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn context_001_saved_layout_matches_assembly_contract() {
    assert_eq!(size_of::<SavedContext>(), 14 * size_of::<usize>());
    assert_eq!(offset_of!(SavedContext, ra), 0);
    assert_eq!(offset_of!(SavedContext, sp), size_of::<usize>());
    assert_eq!(offset_of!(SavedContext, s), 2 * size_of::<usize>());
  }

  #[test]
  fn context_002_initial_context_requires_aligned_stack() {
    assert!(SavedContext::initial(0x1000, 0x8000).is_ok());
    assert!(matches!(
      SavedContext::initial(0x1000, 0x8001),
      Err(Error::InvalidStackTop)
    ));
  }

  #[test]
  fn context_003_initial_context_requires_entry() {
    assert!(matches!(
      SavedContext::initial(0, 0x8000),
      Err(Error::InvalidEntry)
    ));
  }

  #[test]
  fn context_004_trap_resume_requires_aligned_live_frame() {
    assert_eq!(validate_trap_frame(0), Err(Error::InvalidTrapFrame));
    assert_eq!(validate_trap_frame(0x8001), Err(Error::InvalidTrapFrame));
    assert_eq!(validate_trap_frame(0x8000), Ok(()));
  }
}
