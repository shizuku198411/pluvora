#![no_main]
#![no_std]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr;

global_asm!(include_str!("entry.S"));

const FDT_MAGIC: u32 = 0xd00d_feed;
const SBI_LEGACY_CONSOLE_PUTCHAR: usize = 1;

#[unsafe(no_mangle)]
extern "C" fn kernel_entry(hart_id: usize, dtb_address: usize) -> ! {
  console_write("\n[Pluvora] enterd kernel\n");
  console_write("hart ID: ");
  console_write_hex(hart_id);
  console_putchar(b'\n');
  console_write("DTB address: ");
  console_write_hex(dtb_address);
  console_putchar(b'\n');

  if dtb_has_valid_magic(dtb_address) {
    console_write("DTB magic: valid\n");
  } else {
    console_write("DTB magic: invalid\n");
  }

  park()
}

fn dtb_has_valid_magic(dtb_address: usize) -> bool {
  if dtb_address == 0 || !dtb_address.is_multiple_of(4) {
    return false;
  }

  // SAFETY: OpenSBI supplies a physical pointer to a readable FDT blob in
  // `a1`. Paging is not enabled yet, so this physical address is directly
  // addressable. The null and alignment checks above cover pointer validity
  // requirements that can be checked before parsing the blob.
  let encoded_magic = unsafe { ptr::read_volatile(dtb_address as *const u32) };
  u32::from_be(encoded_magic) == FDT_MAGIC
}

// initial implementation use only
fn console_write(message: &str) {
  for byte in message.bytes() {
    console_putchar(byte);
  }
}

// initial implementation use only
fn console_write_hex(value: usize) {
  console_write("0x");

  let mut shift = usize::BITS;
  while shift != 0 {
    shift -= 4;
    let digit = ((value >> shift) & 0xf) as u8;
    console_putchar(if digit < 10 {
      b'0' + digit
    } else {
      b'a' + digit -10
    });
  }
}

// initial implementation use only
fn console_putchar(byte: u8) {
  // SAFETY: This follows the OpenSBI legacy console-putchar calling convention.
  unsafe {
    asm!(
      "ecall",
      inlateout("a0") usize::from(byte) => _,
      in("a7") SBI_LEGACY_CONSOLE_PUTCHAR,
      options(nostack),
    );
  }
}

fn park() -> ! {
  loop {
    // SAFETY: WFI is used only as a low-power hint while this bootstrap
    // hart has no further work. Spurious wakeups return to the loop.
    unsafe { asm!("wfi", options(nomem, nostack)) };
  }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
  console_write("\n[Pluvora] kernel panic during bootstrap\n");
  park()
}