#[cfg(target_arch = "riscv64")]
use core::arch::asm;
use core::mem::{offset_of, size_of};

#[cfg(target_arch = "riscv64")]
use crate::{
  arch::riscv64::{ipi, timer},
  kernel_state,
  support::{console, cpu},
};

const INTERRUPT_BIT: usize = 1 << (usize::BITS - 1);
const CAUSE_CODE_MASK: usize = !INTERRUPT_BIT;

const EXCEPTION_ILLEGAL_INSTRUCTION: usize = 2;
const EXCEPTION_BREAKPOINT: usize = 3;
const EXCEPTION_USER_ECALL: usize = 8;
const EXCEPTION_SUPERVISOR_ECALL: usize = 9;
const EXCEPTION_INSTRUCTION_PAGE_FAULT: usize = 12;
const EXCEPTION_LOAD_PAGE_FAULT: usize = 13;
const EXCEPTION_STORE_PAGE_FAULT: usize = 15;

const INTERRUPT_SUPERVISOR_SOFTWARE: usize = 1;
const INTERRUPT_SUPERVISOR_TIMER: usize = 5;
const INTERRUPT_SUPERVISOR_EXTERNAL: usize = 9;

const SSTATUS_SIE: usize = 1 << 1;
const SSTATUS_SPIE: usize = 1 << 5;
const SSTATUS_SPP: usize = 1 << 8;
const SSTATUS_SUM: usize = 1 << 18;
const SSTATUS_MXR: usize = 1 << 19;
const SSTATUS_UXL_SHIFT: usize = 32;
const SSTATUS_UXL_64: usize = 0b10 << SSTATUS_UXL_SHIFT;

pub const USER_STACK_ALIGNMENT: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivilegeMode {
  User,
  Supervisor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryError {
  InvalidKernelStack,
  InvalidKernelThreadPointer,
  ScratchNotClear,
}

impl BoundaryError {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::InvalidKernelStack => "user trap kernel stack is invalid",
      Self::InvalidKernelThreadPointer => "user trap kernel tp is invalid",
      Self::ScratchNotClear => "S-mode sscratch invariant is not clear",
    }
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserContextError {
  InvalidEntry,
  InvalidStackPointer,
  InvalidRegister,
}

impl UserContextError {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::InvalidEntry => "invalid user entry address",
      Self::InvalidStackPointer => "invalid user stack pointer",
      Self::InvalidRegister => "invalid user register index",
    }
  }
}

#[repr(C)]
pub struct TrapFrame {
  registers: [usize; 32],
  sstatus: usize,
  sepc: usize,
  scause: usize,
  stval: usize,
  kernel_tp: usize,
  kernel_stack_top: usize,
}

impl TrapFrame {
  pub fn initial_user(entry: usize, stack_pointer: usize) -> Result<Self, UserContextError> {
    if entry == 0 || !crate::arch::riscv64::paging::is_sv39_address(entry as u64) {
      return Err(UserContextError::InvalidEntry);
    }
    if stack_pointer == 0
      || !stack_pointer.is_multiple_of(USER_STACK_ALIGNMENT)
      || !crate::arch::riscv64::paging::is_sv39_address(stack_pointer as u64)
    {
      return Err(UserContextError::InvalidStackPointer);
    }

    let mut registers = [0; 32];
    registers[2] = stack_pointer;
    Ok(Self {
      registers,
      sstatus: initial_user_sstatus(),
      sepc: entry,
      scause: 0,
      stval: 0,
      kernel_tp: 0,
      kernel_stack_top: 0,
    })
  }

  pub(crate) fn prepare_user_dispatch(
    &mut self,
    kernel_tp: usize,
    kernel_stack_top: usize,
  ) -> Result<(), BoundaryError> {
    if kernel_tp == 0 {
      return Err(BoundaryError::InvalidKernelThreadPointer);
    }
    validate_kernel_stack_top(kernel_stack_top)?;
    self.kernel_tp = kernel_tp;
    self.kernel_stack_top = kernel_stack_top;
    Ok(())
  }

  pub const fn kernel_tp(&self) -> usize {
    self.kernel_tp
  }

  pub const fn kernel_stack_top(&self) -> usize {
    self.kernel_stack_top
  }

  pub const fn register(&self, index: usize) -> Option<usize> {
    if index < self.registers.len() {
      Some(self.registers[index])
    } else {
      None
    }
  }

  pub fn set_register(&mut self, index: usize, value: usize) -> Result<(), UserContextError> {
    if index == 0 || index >= self.registers.len() {
      return Err(UserContextError::InvalidRegister);
    }
    self.registers[index] = value;
    Ok(())
  }

  pub const fn user_syscall(&self) -> UserSyscall {
    match self.registers[17] {
      crate::user_image::EXIT_SYSCALL => UserSyscall::Exit {
        status: self.registers[10],
      },
      crate::user_image::YIELD_SYSCALL => UserSyscall::Yield,
      number => UserSyscall::Unknown { number },
    }
  }

  pub fn complete_yield_syscall(&mut self) -> Result<(), UserContextError> {
    let next = self
      .sepc
      .checked_add(4)
      .ok_or(UserContextError::InvalidEntry)?;
    if !crate::arch::riscv64::paging::is_sv39_address(next as u64) {
      return Err(UserContextError::InvalidEntry);
    }
    self.registers[10] = 0;
    self.sepc = next;
    Ok(())
  }

  pub const fn privilege_mode(&self) -> PrivilegeMode {
    privilege_mode_from_sstatus(self.sstatus)
  }

  pub const fn origin(&self) -> PrivilegeMode {
    self.privilege_mode()
  }

  pub const fn is_user_origin(&self) -> bool {
    matches!(self.origin(), PrivilegeMode::User)
  }

  pub const fn stack_pointer(&self) -> usize {
    self.registers[2]
  }

  pub const fn sstatus(&self) -> usize {
    self.sstatus
  }

  pub const fn sepc(&self) -> usize {
    self.sepc
  }

  pub const fn scause(&self) -> usize {
    self.scause
  }

  pub const fn stval(&self) -> usize {
    self.stval
  }
}

const _: () = {
  assert!(size_of::<TrapFrame>() == 38 * size_of::<usize>());
  assert!(offset_of!(TrapFrame, registers) == 0);
  assert!(offset_of!(TrapFrame, sstatus) == 32 * size_of::<usize>());
  assert!(offset_of!(TrapFrame, sepc) == 33 * size_of::<usize>());
  assert!(offset_of!(TrapFrame, scause) == 34 * size_of::<usize>());
  assert!(offset_of!(TrapFrame, stval) == 35 * size_of::<usize>());
  assert!(offset_of!(TrapFrame, kernel_tp) == 36 * size_of::<usize>());
  assert!(offset_of!(TrapFrame, kernel_stack_top) == 37 * size_of::<usize>());
};

const fn initial_user_sstatus() -> usize {
  let mut status = SSTATUS_SPIE | SSTATUS_UXL_64;
  status &= !(SSTATUS_SIE | SSTATUS_SPP | SSTATUS_SUM | SSTATUS_MXR);
  status
}

const fn privilege_mode_from_sstatus(sstatus: usize) -> PrivilegeMode {
  if sstatus & SSTATUS_SPP == 0 {
    PrivilegeMode::User
  } else {
    PrivilegeMode::Supervisor
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cause {
  Exception(Exception),
  Interrupt(Interrupt),
}

impl Cause {
  pub const fn from_scause(scause: usize) -> Self {
    let code = scause & CAUSE_CODE_MASK;
    if scause & INTERRUPT_BIT != 0 {
      Self::Interrupt(Interrupt::from_code(code))
    } else {
      Self::Exception(Exception::from_code(code))
    }
  }

  pub const fn name(self) -> &'static str {
    match self {
      Self::Exception(exception) => exception.name(),
      Self::Interrupt(exception) => exception.name(),
    }
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Exception {
  IllegalInstruction,
  Breakpoint,
  UserEnvironmentCall,
  SupervisorEnvironmentCall,
  InstructionPageFault,
  LoadPageFault,
  StorePageFault,
  Unknown(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserExceptionAction {
  Syscall,
  TerminateFault,
}

pub const fn user_exception_action(exception: Exception) -> UserExceptionAction {
  match exception {
    Exception::UserEnvironmentCall => UserExceptionAction::Syscall,
    Exception::IllegalInstruction
    | Exception::Breakpoint
    | Exception::SupervisorEnvironmentCall
    | Exception::InstructionPageFault
    | Exception::LoadPageFault
    | Exception::StorePageFault
    | Exception::Unknown(_) => UserExceptionAction::TerminateFault,
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserSyscall {
  Exit { status: usize },
  Yield,
  Unknown { number: usize },
}

impl Exception {
  pub const fn code(self) -> usize {
    match self {
      Self::IllegalInstruction => EXCEPTION_ILLEGAL_INSTRUCTION,
      Self::Breakpoint => EXCEPTION_BREAKPOINT,
      Self::UserEnvironmentCall => EXCEPTION_USER_ECALL,
      Self::SupervisorEnvironmentCall => EXCEPTION_SUPERVISOR_ECALL,
      Self::InstructionPageFault => EXCEPTION_INSTRUCTION_PAGE_FAULT,
      Self::LoadPageFault => EXCEPTION_LOAD_PAGE_FAULT,
      Self::StorePageFault => EXCEPTION_STORE_PAGE_FAULT,
      Self::Unknown(code) => code,
    }
  }

  const fn from_code(code: usize) -> Self {
    match code {
      EXCEPTION_ILLEGAL_INSTRUCTION => Self::IllegalInstruction,
      EXCEPTION_BREAKPOINT => Self::Breakpoint,
      EXCEPTION_USER_ECALL => Self::UserEnvironmentCall,
      EXCEPTION_SUPERVISOR_ECALL => Self::SupervisorEnvironmentCall,
      EXCEPTION_INSTRUCTION_PAGE_FAULT => Self::InstructionPageFault,
      EXCEPTION_LOAD_PAGE_FAULT => Self::LoadPageFault,
      EXCEPTION_STORE_PAGE_FAULT => Self::StorePageFault,
      code => Self::Unknown(code),
    }
  }

  const fn name(self) -> &'static str {
    match self {
      Self::IllegalInstruction => "illegal instruction",
      Self::Breakpoint => "breakpoint",
      Self::UserEnvironmentCall => "environment call from U-mode",
      Self::SupervisorEnvironmentCall => "environment call from S-mode",
      Self::InstructionPageFault => "instruction page fault",
      Self::LoadPageFault => "load page fault",
      Self::StorePageFault => "store/AMO page fault",
      Self::Unknown(_) => "unknown exception",
    }
  }
}

pub const fn termination_reason(
  exception: Exception,
  sepc: usize,
  stval: usize,
) -> crate::process::TerminationReason {
  use crate::process::TerminationReason;
  match exception {
    Exception::IllegalInstruction => TerminationReason::IllegalInstruction { sepc, stval },
    Exception::Breakpoint => TerminationReason::Breakpoint { sepc },
    Exception::InstructionPageFault => TerminationReason::InstructionPageFault { sepc, stval },
    Exception::LoadPageFault => TerminationReason::LoadPageFault { sepc, stval },
    Exception::StorePageFault => TerminationReason::StorePageFault { sepc, stval },
    Exception::Unknown(_)
    | Exception::SupervisorEnvironmentCall
    | Exception::UserEnvironmentCall => TerminationReason::UnknownException {
      code: exception.code(),
      sepc,
      stval,
    },
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interrupt {
  SupervisorSoftware,
  SupervisorTimer,
  SupervisorExternal,
  Unknown(usize),
}

impl Interrupt {
  const fn from_code(code: usize) -> Self {
    match code {
      INTERRUPT_SUPERVISOR_SOFTWARE => Self::SupervisorSoftware,
      INTERRUPT_SUPERVISOR_TIMER => Self::SupervisorTimer,
      INTERRUPT_SUPERVISOR_EXTERNAL => Self::SupervisorExternal,
      code => Self::Unknown(code),
    }
  }

  const fn name(self) -> &'static str {
    match self {
      Self::SupervisorSoftware => "supervisor software interrupt",
      Self::SupervisorTimer => "supervisor timer interrupt",
      Self::SupervisorExternal => "supervisor external interrupt",
      Self::Unknown(_) => "unknown interrupt",
    }
  }
}

#[cfg(target_arch = "riscv64")]
unsafe extern "C" {
  fn trap_entry();
}

#[cfg(target_arch = "riscv64")]
pub fn install() -> usize {
  let address = trap_entry as *const () as usize;
  // SAFETY: trap_entry is 4-byte aligned assembly code using direct mode.
  // S-mode execution keeps sscratch clear until a user return is armed.
  unsafe {
    asm!(
      "csrw sscratch, zero",
      "csrw stvec, {address}",
      address = in(reg) address,
      options(nostack),
    )
  };
  address
}

#[cfg(target_arch = "riscv64")]
pub fn idle_scratch() -> Result<usize, BoundaryError> {
  let value: usize;
  // SAFETY: reading sscratch is permitted in S-mode.
  unsafe { asm!("csrr {value}, sscratch", value = out(reg) value, options(nostack)) };
  if value != 0 {
    return Err(BoundaryError::ScratchNotClear);
  }
  Ok(value)
}

pub(crate) fn validate_kernel_stack_top(kernel_stack_top: usize) -> Result<(), BoundaryError> {
  if kernel_stack_top == 0
    || !kernel_stack_top.is_multiple_of(crate::arch::riscv64::context::KERNEL_STACK_ALIGNMENT)
  {
    return Err(BoundaryError::InvalidKernelStack);
  }
  Ok(())
}

#[cfg(target_arch = "riscv64")]
#[expect(dead_code, reason = "used when user dispatch is enabled")]
pub(crate) unsafe fn arm_user_trap_stack(kernel_stack_top: usize) -> Result<(), BoundaryError> {
  validate_kernel_stack_top(kernel_stack_top)?;
  idle_scratch()?;
  // SAFETY: caller owns the current hart transition to U-mode and supplied the
  // current Thread's live kernel stack top.
  unsafe {
    asm!(
      "csrw sscratch, {top}",
      top = in(reg) kernel_stack_top,
      options(nostack),
    )
  };
  Ok(())
}

#[cfg(target_arch = "riscv64")]
pub(crate) fn disarm_user_trap_stack() {
  // SAFETY: caller has returned to a stable S-mode stack.
  unsafe { asm!("csrw sscratch, zero", options(nostack)) };
}

#[cfg(target_arch = "riscv64")]
#[unsafe(no_mangle)]
extern "C" fn trap_handler(frame: &mut TrapFrame) {
  let cause = Cause::from_scause(frame.scause);
  if frame.is_user_origin() {
    return handle_user_trap(frame, cause);
  }

  if cause == Cause::Interrupt(Interrupt::SupervisorSoftware) {
    ipi::handle_local();
    if kernel_state::stop_current_kernel_if_terminating() {
      unreachable!("terminating kernel join returned");
    }
    if crate::arch::riscv64::hart::current().reschedule_requested() {
      kernel_state::try_preempt_current_thread(frame);
    }
    return;
  }

  if cause == Cause::Interrupt(Interrupt::SupervisorTimer) {
    timer::handle_local();
    if kernel_state::stop_current_kernel_if_terminating() {
      unreachable!("terminating kernel join returned");
    }
    kernel_state::try_preempt_current_thread(frame);
    return;
  }

  if cause == Cause::Exception(Exception::Breakpoint) {
    frame.sepc = frame
      .sepc
      .checked_add(4)
      .unwrap_or_else(|| fatal_trap(frame, cause));
    return;
  }

  fatal_trap(frame, cause)
}

#[cfg(target_arch = "riscv64")]
fn handle_user_trap(frame: &mut TrapFrame, cause: Cause) {
  match cause {
    Cause::Exception(Exception::UserEnvironmentCall) => match frame.user_syscall() {
      UserSyscall::Exit { status } => {
        kernel_state::terminate_current_process(crate::process::TerminationReason::Exit { status })
      }
      UserSyscall::Yield => {
        frame.complete_yield_syscall().unwrap_or_else(|_| {
          kernel_state::terminate_current_process(
            crate::process::TerminationReason::UnknownException {
              code: EXCEPTION_USER_ECALL,
              sepc: frame.sepc(),
              stval: frame.stval(),
            },
          )
        });
        kernel_state::switch_out_current_user_thread(
          frame as *mut _,
          crate::thread::UserSwitchReason::Yield,
        )
      }
      UserSyscall::Unknown { number } => {
        kernel_state::terminate_current_process(crate::process::TerminationReason::UnknownSyscall {
          number,
        })
      }
    },
    Cause::Interrupt(Interrupt::SupervisorTimer) => {
      timer::handle_local();
      if kernel_state::stop_current_user_if_terminating() {
        unreachable!("terminating user join returned");
      }
      preempt_user_if_requested(frame);
    }
    Cause::Interrupt(Interrupt::SupervisorSoftware) => {
      ipi::handle_local();
      if kernel_state::stop_current_user_if_terminating() {
        unreachable!("terminating user join returned");
      }
      preempt_user_if_requested(frame);
    }
    Cause::Exception(exception) => kernel_state::terminate_current_process(termination_reason(
      exception,
      frame.sepc(),
      frame.stval(),
    )),
    Cause::Interrupt(_) => unexpected_user_trap(frame, cause),
  }
}

#[cfg(target_arch = "riscv64")]
fn preempt_user_if_requested(frame: &mut TrapFrame) {
  let local = crate::arch::riscv64::hart::current();
  if !local.reschedule_requested() || !local.can_preempt() {
    return;
  }
  if !local.take_reschedule_request() {
    panic!("user preemption lost reschedule request");
  }
  kernel_state::switch_out_current_user_thread(
    frame as *mut _,
    crate::thread::UserSwitchReason::Preempt,
  )
}

#[cfg(target_arch = "riscv64")]
fn unexpected_user_trap(frame: &TrapFrame, cause: Cause) -> ! {
  console::newline();
  console::write_line("[Unexpected U-mode Trap]");
  console::write("  Cause             : ");
  console::write_line(cause.name());
  field("scause", frame.scause());
  field("sepc", frame.sepc());
  field("stval", frame.stval());
  field("sstatus", frame.sstatus());
  field("Saved user sp", frame.stack_pointer());
  cpu::halt()
}

#[cfg(target_arch = "riscv64")]
fn fatal_trap(frame: &TrapFrame, cause: Cause) -> ! {
  console::newline();
  console::write_line("[Fatal S-mode Trap]");
  console::write("  Cause             : ");
  console::write_line(cause.name());
  field("scause", frame.scause());
  field("sepc", frame.sepc());
  field("stval", frame.stval());
  field("sstatus", frame.sstatus());
  field("Saved sp", frame.stack_pointer());
  cpu::halt()
}

#[cfg(target_arch = "riscv64")]
fn field(label: &str, value: usize) {
  console::write("  ");
  console::write(label);
  for _ in label.len()..18 {
    console::write(" ");
  }
  console::write(": ");
  console::write_hex_usize(value);
  console::newline();
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn trap_001_illegal_instruction_is_decoded() {
    assert_eq!(
      Cause::from_scause(2),
      Cause::Exception(Exception::IllegalInstruction)
    );
  }

  #[test]
  fn trap_002_breakpoint_is_decoded() {
    assert_eq!(
      Cause::from_scause(3),
      Cause::Exception(Exception::Breakpoint)
    );
  }

  #[test]
  fn trap_004_page_faults_are_distinct() {
    assert_eq!(
      Cause::from_scause(12),
      Cause::Exception(Exception::InstructionPageFault)
    );
    assert_eq!(
      Cause::from_scause(13),
      Cause::Exception(Exception::LoadPageFault)
    );
    assert_eq!(
      Cause::from_scause(15),
      Cause::Exception(Exception::StorePageFault)
    );
  }

  #[test]
  fn trap_005_supervisor_interrupts_are_decoded() {
    assert_eq!(
      Cause::from_scause(INTERRUPT_BIT | 1),
      Cause::Interrupt(Interrupt::SupervisorSoftware)
    );
    assert_eq!(
      Cause::from_scause(INTERRUPT_BIT | 5),
      Cause::Interrupt(Interrupt::SupervisorTimer)
    );
    assert_eq!(
      Cause::from_scause(INTERRUPT_BIT | 9),
      Cause::Interrupt(Interrupt::SupervisorExternal)
    );
  }

  #[test]
  fn trap_007_supervisor_timer_interrupt_is_decoded() {
    assert_eq!(
      Cause::from_scause(INTERRUPT_BIT | INTERRUPT_SUPERVISOR_TIMER),
      Cause::Interrupt(Interrupt::SupervisorTimer)
    );
  }

  #[test]
  fn trap_008_unknown_causes_keep_their_numeric_code() {
    assert_eq!(
      Cause::from_scause(63),
      Cause::Exception(Exception::Unknown(63))
    );
    assert_eq!(
      Cause::from_scause(INTERRUPT_BIT | 63),
      Cause::Interrupt(Interrupt::Unknown(63))
    );
  }

  #[test]
  fn trap_010_trap_frame_layout_matches_assembly_abi() {
    assert_eq!(size_of::<TrapFrame>(), 304);
    assert_eq!(offset_of!(TrapFrame, registers), 0);
    assert_eq!(offset_of!(TrapFrame, sstatus), 32 * 8);
    assert_eq!(offset_of!(TrapFrame, sepc), 33 * 8);
    assert_eq!(offset_of!(TrapFrame, scause), 34 * 8);
    assert_eq!(offset_of!(TrapFrame, stval), 35 * 8);
    assert_eq!(offset_of!(TrapFrame, kernel_tp), 36 * 8);
    assert_eq!(offset_of!(TrapFrame, kernel_stack_top), 37 * 8);
  }

  #[test]
  fn trap_011_initial_user_frame_has_least_privilege_state() {
    let frame = TrapFrame::initial_user(0x1000, 0x8000).unwrap();
    assert_eq!(frame.privilege_mode(), PrivilegeMode::User);
    assert_eq!(frame.sepc(), 0x1000);
    assert_eq!(frame.stack_pointer(), 0x8000);
    assert_eq!(frame.scause(), 0);
    assert_eq!(frame.stval(), 0);
    assert_eq!(frame.sstatus() & SSTATUS_SPIE, SSTATUS_SPIE);
    assert_eq!(
      frame.sstatus() & (0b11 << SSTATUS_UXL_SHIFT),
      SSTATUS_UXL_64
    );
    assert_eq!(
      frame.sstatus() & (SSTATUS_SIE | SSTATUS_SPP | SSTATUS_SUM | SSTATUS_MXR),
      0
    );
    assert!(
      frame
        .registers
        .iter()
        .enumerate()
        .all(|(index, value)| index == 2 || *value == 0)
    );
  }

  #[test]
  fn trap_012_user_entry_must_be_nonzero_and_canonical() {
    assert!(matches!(
      TrapFrame::initial_user(0, 0x8000),
      Err(UserContextError::InvalidEntry)
    ));
    assert!(matches!(
      TrapFrame::initial_user(0x0000_0040_0000_0000, 0x8000),
      Err(UserContextError::InvalidEntry)
    ));
  }

  #[test]
  fn trap_013_user_stack_must_be_aligned_and_canonical() {
    assert!(matches!(
      TrapFrame::initial_user(0x1000, 0),
      Err(UserContextError::InvalidStackPointer)
    ));
    assert!(matches!(
      TrapFrame::initial_user(0x1000, 0x8008),
      Err(UserContextError::InvalidStackPointer)
    ));
    assert!(matches!(
      TrapFrame::initial_user(0x1000, 0x0000_0040_0000_0000),
      Err(UserContextError::InvalidStackPointer)
    ));
  }

  #[test]
  fn trap_014_spp_is_the_trap_origin_authority() {
    assert_eq!(privilege_mode_from_sstatus(0), PrivilegeMode::User);
    assert_eq!(
      privilege_mode_from_sstatus(SSTATUS_SPIE | SSTATUS_SUM),
      PrivilegeMode::User
    );
    assert_eq!(
      privilege_mode_from_sstatus(SSTATUS_SPP),
      PrivilegeMode::Supervisor
    );
  }

  #[test]
  fn trap_015_initial_user_frame_reports_user_origin() {
    let frame = TrapFrame::initial_user(0x1000, 0x4000).unwrap();
    assert_eq!(frame.origin(), PrivilegeMode::User);
    assert!(frame.is_user_origin());
  }

  #[test]
  fn trap_016_user_ecall_enters_syscall_dispatch() {
    assert_eq!(
      user_exception_action(Exception::UserEnvironmentCall),
      UserExceptionAction::Syscall
    );
  }

  #[test]
  fn trap_017_user_faults_require_termination() {
    for exception in [
      Exception::IllegalInstruction,
      Exception::Breakpoint,
      Exception::InstructionPageFault,
      Exception::LoadPageFault,
      Exception::StorePageFault,
      Exception::Unknown(63),
    ] {
      assert_eq!(
        user_exception_action(exception),
        UserExceptionAction::TerminateFault
      );
    }
  }

  #[test]
  fn trap_018_kernel_stack_arming_requires_alignment() {
    assert_eq!(
      validate_kernel_stack_top(0),
      Err(BoundaryError::InvalidKernelStack)
    );
    assert_eq!(
      validate_kernel_stack_top(0x8001),
      Err(BoundaryError::InvalidKernelStack)
    );
    assert_eq!(validate_kernel_stack_top(0x8000), Ok(()));
  }

  #[test]
  fn trap_019_user_dispatch_metadata_requires_kernel_tp_and_aligned_stack() {
    let mut frame = TrapFrame::initial_user(0x1000, 0x4000).unwrap();
    assert_eq!(
      frame.prepare_user_dispatch(0, 0x9000),
      Err(BoundaryError::InvalidKernelThreadPointer)
    );
    assert_eq!(
      frame.prepare_user_dispatch(0x8100_0000, 0x9008),
      Err(BoundaryError::InvalidKernelStack)
    );
    frame.prepare_user_dispatch(0x8100_0000, 0x9000).unwrap();
    assert_eq!(frame.kernel_tp(), 0x8100_0000);
    assert_eq!(frame.kernel_stack_top(), 0x9000);
  }

  #[test]
  fn trap_020_user_syscall_decodes_exit_yield_and_unknown() {
    let mut frame = TrapFrame::initial_user(0x1000, 0x4000).unwrap();
    frame.set_register(10, 7).unwrap();
    frame
      .set_register(17, crate::user_image::EXIT_SYSCALL)
      .unwrap();
    assert_eq!(frame.user_syscall(), UserSyscall::Exit { status: 7 });
    frame
      .set_register(17, crate::user_image::YIELD_SYSCALL)
      .unwrap();
    assert_eq!(frame.user_syscall(), UserSyscall::Yield);
    frame.set_register(17, 99).unwrap();
    assert_eq!(frame.user_syscall(), UserSyscall::Unknown { number: 99 });
  }

  #[test]
  fn trap_021_yield_completion_sets_result_and_advances_sepc() {
    let mut frame = TrapFrame::initial_user(0x1000, 0x4000).unwrap();
    frame.set_register(10, usize::MAX).unwrap();
    frame.complete_yield_syscall().unwrap();
    assert_eq!(frame.register(10), Some(0));
    assert_eq!(frame.sepc(), 0x1004);
  }

  #[test]
  fn trap_022_register_zero_and_invalid_yield_continuation_are_rejected() {
    let mut frame = TrapFrame::initial_user(0x1000, 0x4000).unwrap();
    assert_eq!(
      frame.set_register(0, 1),
      Err(UserContextError::InvalidRegister)
    );
    assert_eq!(frame.register(32), None);
    frame.sepc = 0x0000_003f_ffff_fffc;
    assert_eq!(
      frame.complete_yield_syscall(),
      Err(UserContextError::InvalidEntry)
    );
  }

  #[test]
  fn trap_023_user_faults_map_to_typed_termination_reasons() {
    use crate::process::TerminationReason;
    assert_eq!(
      termination_reason(Exception::IllegalInstruction, 0x1000, 0x55),
      TerminationReason::IllegalInstruction {
        sepc: 0x1000,
        stval: 0x55
      }
    );
    assert_eq!(
      termination_reason(Exception::Breakpoint, 0x2000, 0),
      TerminationReason::Breakpoint { sepc: 0x2000 }
    );
    assert_eq!(
      termination_reason(Exception::LoadPageFault, 0x3000, 0xdead),
      TerminationReason::LoadPageFault {
        sepc: 0x3000,
        stval: 0xdead
      }
    );
    assert_eq!(
      termination_reason(Exception::Unknown(63), 0x4000, 0xbeef),
      TerminationReason::UnknownException {
        code: 63,
        sepc: 0x4000,
        stval: 0xbeef
      }
    );
  }
}
