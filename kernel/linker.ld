OUTPUT_ARCH(riscv)
ENTRY(_start)

KERNEL_BASE = 0x80200000;

SECTIONS
{
  . = KERNEL_BASE;
  __kernel_start = .;

  .text : ALIGN(4K)
  {
    __text_start = .;
    KEEP(*(.text.entry))
    *(.text .text.*)
    __text_end = .;
  }

  .rodata : ALIGN(4K)
  {
    __rodata_start = .;
    *(.rodata .rodata.*)
    __rodata_end = .;
  }

  .data : ALIGN(4K)
  {
    __data_start = .;
    *(.sdata .sdata.*)
    *(.data .data.*)
    __data_end = .;
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