use core::arch::asm;

use crate::support::{console, cpu};

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

#[repr(C)]
pub struct TrapFrame {
  registers: [usize; 32],
  sstatus: usize,
  sepc: usize,
  scause: usize,
  stval: usize,
}

impl TrapFrame {
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

impl Exception {
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

unsafe extern "C" {
  fn trap_entry();
}

pub fn install() -> usize {
  let address = trap_entry as *const () as usize;
  // SAFETY: trap_entry is 4-byte aligned assembly code using direct mode.
  unsafe { asm!("csrw stvec, {address}", address = in(reg) address, options(nostack)) };
  address
}

pub fn breakpoint_self_test() {
  // Use a fixed 32-bit EBREAK encoding so the handler can advance sepc by
  // exactly four bytes even when the C extension is enabled.
  // SAFETY: boot installs trap_entry before calling this function, and the
  // breakpoint handler advances sepc before returning with SRET.
  unsafe { asm!(".4byte 0x00100073", options(nostack)) };
}

#[unsafe(no_mangle)]
extern "C" fn trap_handler(frame: &mut TrapFrame) {
  let cause = Cause::from_scause(frame.scause);

  if cause == Cause::Exception(Exception::Breakpoint) {
    frame.sepc = frame
      .sepc
      .checked_add(4)
      .unwrap_or_else(|| fatal_trap(frame, cause));
    return;
  }

  fatal_trap(frame, cause)
}

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
  cpu::park()
}

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
