.section .text.entry, "ax"
.global _start
.type _start, @function
.align 2

.equ BOOT_STACK_SIZE, 16384

_start:
  # OpenSBI enters the next stage with:
  #   a0 = hart ID
  #   a1 = physical address of the flattened device tree
  # Only the OpenSBI-selected boot hart enters this path.
  la    sp, __boot_stack_end
  andi  sp, sp, -16

  # Clear BSS before Rust observes zero-initialized static objects.
  # Clear byte-by-byte so this remains correct even when __bss_end is not
  # naturally aligned to a machine word.
  la    t0, __bss_start
  la    t1, __bss_end

.Lclear_bss:
  bgeu  t0, t1, .Lbss_done
  sb    zero, 0(t0)
  addi  t0, t0, 1
  j     .Lclear_bss

.Lbss_done:
  tail  kernel_entry

.size _start, . - _start

.section .bss.stack, "aw", @nobits
.balign 16
.globl __boot_stack_start
__boot_stack_start:
  .space BOOT_STACK_SIZE
.globl __boot_stack_end
__boot_stack_end:
