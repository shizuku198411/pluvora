#[cfg(target_arch = "riscv64")]
use core::arch::asm;

use crate::arch::riscv64::{hart, sbi};
#[cfg(target_arch = "riscv64")]
use crate::{arch::riscv64::tlb, support::cpu};
const REQUEST_RESCHEDULE: usize = 1 << 0;
const REQUEST_TLB_SHOOTDOWN: usize = 1 << 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
  Reschedule,
  TlbShootdown,
}

impl Request {
  const fn bit(self) -> usize {
    match self {
      Self::Reschedule => REQUEST_RESCHEDULE,
      Self::TlbShootdown => REQUEST_TLB_SHOOTDOWN,
    }
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  InvalidHartIndex,
  HartNotOnline,
  Sbi(sbi::Error),
}

impl From<sbi::Error> for Error {
  fn from(error: sbi::Error) -> Self {
    Self::Sbi(error)
  }
}

#[cfg(target_arch = "riscv64")]
pub fn enable_local() {
  const SIE_SSIE: usize = 1 << 1;
  unsafe { asm!("csrs sie, {mask}", mask = in(reg) SIE_SSIE, options(nomem, nostack)) };
}

#[cfg(target_arch = "riscv64")]
pub fn send(target_index: usize, request: Request) -> Result<(), Error> {
  let target = target_for_send(target_index)?;
  target.publish_ipi_request(request.bit());

  // Release publication above happens-before the target's Acquire swap.
  // The SBI call is only the notification mechanism.
  sbi::send_ipi(target.raw_hart_id()).map_err(Into::into)
}

fn target_for_send(target_index: usize) -> Result<&'static hart::HartLocal, Error> {
  let target = hart::local_by_index(target_index).ok_or(Error::InvalidHartIndex)?;
  if !target.is_online() {
    return Err(Error::HartNotOnline);
  }
  Ok(target)
}

#[cfg(target_arch = "riscv64")]
pub fn handle_local() {
  clear_local_pending();
  let requests = hart::current().take_ipi_requests();

  if requests & REQUEST_RESCHEDULE != 0 {
    hart::current().request_reschedule();
  }
  if requests & REQUEST_TLB_SHOOTDOWN != 0 {
    tlb::handle_local();
  }
  if requests & !(REQUEST_RESCHEDULE | REQUEST_TLB_SHOOTDOWN) != 0 {
    cpu::halt();
  }
}

#[cfg(target_arch = "riscv64")]
fn clear_local_pending() {
  const SIP_SSIP: usize = 1 << 1;
  unsafe { asm!("csrc sip, {mask}", mask = in(reg) SIP_SSIP, options(nomem, nostack)) };
}

#[cfg(test)]
mod tests {
  use crate::platform::MAX_HARTS;

  use super::*;

  #[test]
  fn ipi_006_invalid_or_offline_target_is_rejected_before_sbi() {
    assert!(matches!(
      target_for_send(MAX_HARTS),
      Err(Error::InvalidHartIndex)
    ));
    assert!(matches!(target_for_send(0), Err(Error::HartNotOnline)));
  }

  #[test]
  fn ipi_007_reschedule_request_uses_the_defined_mailbox_bit() {
    assert_eq!(Request::Reschedule.bit(), REQUEST_RESCHEDULE);
    assert!(REQUEST_RESCHEDULE.is_power_of_two());
  }
}
