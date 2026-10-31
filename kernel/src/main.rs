#![no_main]
#![no_std]

mod dtb;
mod platform;

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

use dtb::Dtb;
use platform::discover;

global_asm!(include_str!("entry.S"));

const SBI_LEGACY_CONSOLE_PUTCHAR: usize = 1;

#[unsafe(no_mangle)]
extern "C" fn kernel_entry(hart_id: usize, dtb_address: usize) -> ! {
  console_write("\n[Pluvora] entered kernel\n");
  console_write("hart ID: ");
  console_write_hex(hart_id);
  console_putchar(b'\n');
  console_write("DTB address: ");
  console_write_hex(dtb_address);
  console_putchar(b'\n');

  // SAFETY: OpenSBI passes a readable, immutable FDT blob in a1.
  // Pluvora has not enabled paging or reclaimed boot memory at this point.
  let dtb = match unsafe { Dtb::from_ptr(dtb_address) } {
    Ok(dtb) => dtb,
    Err(error) => {
      console_write("DTB error: ");
      console_write(error.as_str());
      console_putchar(b'\n');
      park();
    }
  };

  console_write("DTB size: ");
  console_write_hex(dtb.header().total_size);
  console_putchar(b'\n');
  console_write("DTB version: ");
  console_write_hex(dtb.header().version as usize);
  console_putchar(b'\n');
  console_write("DTB boot CPU: ");
  console_write_hex(dtb.header().boot_cpu_id as usize);
  console_putchar(b'\n');

  let platform = match discover(&dtb, hart_id) {
    Ok(platform) => platform,
    Err(error) => {
      console_write("Platform error: ");
      console_write(error.as_str());
      console_putchar(b'\n');
      park();
    }
  };

  console_write("Hart count: ");
  console_write_hex(platform.harts().len());
  console_putchar(b'\n');

  for (index, discovered_hart_id) in platform.harts().iter().enumerate() {
    console_write("Hart index ");
    console_write_hex(index);
    console_write(" -> ID ");
    console_write_hex(*discovered_hart_id);
    console_putchar(b'\n');
  }

  console_write("Boot hart index: ");
  console_write_hex(platform.hart_index(hart_id).unwrap());
  console_putchar(b'\n');

  for region in platform.memory_regions() {
    console_write("Memory base: ");
    console_write_hex_u64(region.base);
    console_write(", size: ");
    console_write_hex_u64(region.size);
    console_putchar(b'\n');
  }

  park()
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
      b'a' + digit - 10
    });
  }
}

// initial implementation use only
fn console_write_hex_u64(value: u64) {
  console_write("0x");

  let mut shift = u64::BITS;
  while shift != 0 {
    shift -= 4;
    let digit = ((value >> shift) & 0xf) as u8;
    console_putchar(if digit < 10 {
      b'0' + digit
    } else {
      b'a' + digit - 10
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
