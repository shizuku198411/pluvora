#[cfg(target_arch = "riscv64")]
use core::arch::asm;

#[cfg(target_arch = "riscv64")]
const EXTENSION_HSM: usize = 0x0048_534d;
#[cfg(target_arch = "riscv64")]
const HSM_HART_START: usize = 0;

#[cfg(target_arch = "riscv64")]
const SBI_SUCCESS: isize = 0;

#[cfg(target_arch = "riscv64")]
const EXTENSION_IPI: usize = 0x0073_5049;
#[cfg(target_arch = "riscv64")]
const IPI_SEND_IPI: usize = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
  code: isize,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self.code {
      -1 => "SBI operation failed",
      -2 => "SBI operation is not supported",
      -3 => "SBI parameter is invalid",
      -4 => "SBI operation was denied",
      -5 => "SBI address is invalid",
      -6 => "SBI resource is already available",
      _ => "unknown SBI error",
    }
  }
}

#[cfg(target_arch = "riscv64")]
pub fn hart_start(hart_id: usize, start_address: usize, opaque: usize) -> Result<(), Error> {
  let (error, _value) = unsafe {
    call(
      EXTENSION_HSM,
      HSM_HART_START,
      hart_id,
      start_address,
      opaque,
    )
  };

  if error == SBI_SUCCESS {
    Ok(())
  } else {
    Err(Error { code: error })
  }
}

#[cfg(target_arch = "riscv64")]
pub fn send_ipi(raw_hart_id: usize) -> Result<(), Error> {
  let (hart_mask, hart_mask_base) = single_hart_mask(raw_hart_id);
  let (error, _value) = unsafe { call(EXTENSION_IPI, IPI_SEND_IPI, hart_mask, hart_mask_base, 0) };
  if error == SBI_SUCCESS {
    Ok(())
  } else {
    Err(Error { code: error })
  }
}

fn single_hart_mask(raw_hart_id: usize) -> (usize, usize) {
  (1, raw_hart_id)
}

// Performs an SBI call with three arguments.
//
// # SAFETY
//
// The caller must use an extension/function pair whose argument contract
// matches `argument0..=argument2` and must accept any SBI-visible side effects.
#[cfg(target_arch = "riscv64")]
unsafe fn call(
  extension: usize,
  function: usize,
  argument0: usize,
  argument1: usize,
  argument2: usize,
) -> (isize, usize) {
  let error: usize;
  let value: usize;

  unsafe {
    asm!(
      "ecall",
      inlateout("a0") argument0 => error,
      inlateout("a1") argument1 => value,
      in("a2") argument2,
      in("a6") function,
      in("a7") extension,
      options(nostack),
    );
  }

  (error as isize, value)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn sbi_001_standard_error_codes_have_stable_messages() {
    assert_eq!(Error { code: -1 }.as_str(), "SBI operation failed");
    assert_eq!(
      Error { code: -2 }.as_str(),
      "SBI operation is not supported"
    );
    assert_eq!(Error { code: -3 }.as_str(), "SBI parameter is invalid");
    assert_eq!(Error { code: -4 }.as_str(), "SBI operation was denied");
    assert_eq!(Error { code: -5 }.as_str(), "SBI address is invalid");
    assert_eq!(
      Error { code: -6 }.as_str(),
      "SBI resource is already available"
    );
  }

  #[test]
  fn sbi_002_unknown_error_code_is_not_treated_as_a_known_result() {
    let error = Error { code: -99 };
    assert_eq!(error.code, -99);
    assert_eq!(error.as_str(), "unknown SBI error");
  }

  #[test]
  fn ipi_005_single_hart_mask_supports_sparse_raw_ids() {
    assert_eq!(single_hart_mask(0), (1, 0));
    assert_eq!(single_hart_mask(9), (1, 9));
    assert_eq!(single_hart_mask(usize::MAX - 1), (1, usize::MAX - 1));
  }
}
