OUTPUT_ARCH(riscv)
ENTRY(_start)

KERNEL_BASE = 0x80200000;

SECTIONS
{
  . = KERNEL_BASE;
  __kernel_start = .;

  .text : ALIGN(4K)
  {
    KEEP(*(.text.entry))
    *(.text .text.*)
  }

  .rodata : ALIGN(4K)
  {
    *(.rodata .rodata.*)
  }

  .data : ALIGN(4K)
  {
    *(.sdata .sdata.*)
    *(.data .data.*)
  }

  .bss (NOLOAD) : ALIGN(4K)
  {
    __bss_start = .;
    *(.sbss .sbss.*)
    *(.bss .bss.*)
    *(COMMON)
    __bss_end = .;
  }

  . = ALIGN(4K);
  __kernel_end = .;

  /DISCARD/ :
  {
    *(.eh_frame .eh_frame.*)
    *(.comment)
  }
}