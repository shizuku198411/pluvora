#![no_main]
#![no_std]

mod dtb;

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

use dtb::{Dtb, Event};

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

  let mut node_count = 0usize;
  let mut property_count = 0usize;
  let mut maximum_depth = 0usize;

  for event in dtb.events() {
    match event {
      Ok(Event::BeginNode { depth, name }) => {
        node_count += 1;
        maximum_depth = maximum_depth.max(depth);

        // Print only top-level node names.
        if depth <= 1 {
          console_write("DTB node: ");
          if name.is_empty() {
            console_write("/");
          } else {
            console_write(name);
          }
          console_putchar(b'\n');
        }
      }
      Ok(Event::Property { depth, name, value }) => {
        property_count += 1;
        let _ = (depth, name, value);
      }
      Ok(Event::EndNode { depth }) => {
        let _ = depth;
      }
      Ok(Event::End) => break,
      Err(error) => {
        console_write("DTB error: ");
        console_write(error.as_str());
        console_putchar(b'\n');
        park()
      }
    }
  }

  console_write("DTB nodes: ");
  console_write_hex(node_count);
  console_putchar(b'\n');
  console_write("DTB props: ");
  console_write_hex(property_count);
  console_putchar(b'\n');
  console_write("DTB max depth: ");
  console_write_hex(maximum_depth);
  console_putchar(b'\n');

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
