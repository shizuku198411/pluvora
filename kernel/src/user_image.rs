#[cfg(target_arch = "riscv64")]
use core::arch::asm;

pub const EXIT_SYSCALL: usize = 0;
pub const YIELD_SYSCALL: usize = 1;
pub const INITIAL_USER_IMAGE: &[u8] = &[
  0x13, 0x04, 0x30, 0x00, // addi s0, zero, 3
  0x93, 0x08, 0x10, 0x00, // addi a7, zero, 1 (Yield)
  0x73, 0x00, 0x00, 0x00, // ecall
  0x13, 0x04, 0xf4, 0xff, // addi s0, s0, -1
  0xe3, 0x1a, 0x04, 0xfe, // bne s0, zero, -12
  0x13, 0x05, 0x00, 0x00, // addi a0, zero, 0
  0x93, 0x08, 0x00, 0x00, // addi a7, zero, 0 (Exit)
  0x73, 0x00, 0x00, 0x00, // ecall
  0x6f, 0x00, 0x00, 0x00, // jal zero, 0 (must not return)
];

#[cfg(target_arch = "riscv64")]
pub(crate) fn synchronize_local_instruction_stream() {
  // SAFETY: FENCE/FENCE.I only order the current hart. The caller completed
  // stores to an instruction page and has not published it runnable.
  unsafe { asm!("fence rw, rw", "fence.i", options(nostack)) };
}

const _: () = {
  assert!(!INITIAL_USER_IMAGE.is_empty());
  assert!(INITIAL_USER_IMAGE.len() <= crate::memory::PAGE_SIZE as usize);
  assert!(
    INITIAL_USER_IMAGE
      .len()
      .is_multiple_of(core::mem::size_of::<u32>())
  );
};

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn uimg_001_initial_image_is_nonempty_aligned_and_page_bounded() {
    assert!(!INITIAL_USER_IMAGE.is_empty());
    assert!(INITIAL_USER_IMAGE.len().is_multiple_of(4));
    assert!(INITIAL_USER_IMAGE.len() <= crate::memory::PAGE_SIZE as usize);
  }

  #[test]
  fn uimg_002_initial_image_ends_with_fail_closed_self_loop() {
    assert_eq!(
      &INITIAL_USER_IMAGE[INITIAL_USER_IMAGE.len() - 4..],
      &[0x6f, 0, 0, 0]
    );
  }

  #[test]
  fn uimg_003_exit_syscall_number_is_zero() {
    assert_eq!(EXIT_SYSCALL, 0);
  }

  #[test]
  fn uimg_004_repeating_image_contains_three_yields_then_exit() {
    assert_eq!(INITIAL_USER_IMAGE.len(), 36);
    assert_eq!(YIELD_SYSCALL, 1);
    assert_eq!(
      INITIAL_USER_IMAGE
        .windows(4)
        .filter(|instruction| *instruction == [0x73, 0, 0, 0])
        .count(),
      2
    );
  }
}
