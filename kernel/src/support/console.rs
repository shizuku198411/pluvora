use core::arch::asm;

const SBI_LEGACY_CONSOLE_PUTCHAR: usize = 1;

pub fn write(message: &str) {
  for byte in message.bytes() {
    putchar(byte);
  }
}

pub fn write_line(message: &str) {
  write(message);
  newline();
}

pub fn newline() {
  putchar(b'\n');
}

pub fn write_hex_usize(value: usize) {
  write_hex(value as u64, usize::BITS);
}

pub fn write_hex_u64(value: u64) {
  write_hex(value, u64::BITS);
}

fn write_hex(value: u64, bits: u32) {
  write("0x");

  let mut shift = bits;
  while shift != 0 {
    shift -= 4;
    let digit = ((value >> shift) & 0xf) as u8;
    putchar(if digit < 10 {
      b'0' + digit
    } else {
      b'a' + digit - 10
    });
  }
}

fn putchar(byte: u8) {
  // SAFETY: This follows the OpenSBI legacy console-putchar convention. This
  // bootstrap backend will be replaced by the kernel SBI/console abstraction.
  unsafe {
    asm!(
      "ecall",
      inlateout("a0") usize::from(byte) => _,
      in("a7") SBI_LEGACY_CONSOLE_PUTCHAR,
      options(nostack),
    );
  }
}
