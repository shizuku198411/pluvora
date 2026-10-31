.section .text.entry, "ax"
.global _start
.type _start, @function
.align 2

.equ MAX_HARTS, 8
.equ BOOT_STACK_SIZE, 16384

_start:
  # OpenSBI enters the next stage with:
  #   a0 = hart ID
  #   a1 = physical address of the flattened device tree
  # Use only temporary registers so both arguments reach rust_entry unchanged.
  li    t0, MAX_HARTS
  bgeu  a0, t0, .Lpark

  la    t0, __boot_stacks_start
  li    t1, BOOT_STACK_SIZE
  addi  t2, a0, 1
  mul   t1, t1, t2
  add   sp, t0, t1
  andi  sp, sp, -16

  tail  kernel_entry

.Lpark:
  wfi
  j     .Lpark

.size _start, . - _start

.section .bss.stack, "aw", @nobits
.balign 16
.globl __boot_stacks_start
__boot_stacks_start:
    .space MAX_HARTS * BOOT_STACK_SIZE
.globl __boot_stacks_end
__boot_stacks_end:
