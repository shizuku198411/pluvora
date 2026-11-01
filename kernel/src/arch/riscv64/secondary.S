.section .text.secondary, "ax"
.global secondary_entry
.type secondary_entry, @function
.align 2

# SecondaryBootInfo Layout
#   offset    field
#   0x00      stack_top
#   0x08      satp
#   0x10      hart_index
#   0x18      expected_hart_id
#   0x20      hart_local_pointer
#   0x28      publication_state
.equ BOOT_INFO_STACK_TOP,    0
.equ BOOT_INFO_SATP,         8
.equ BOOT_INFO_HART_INDEX,  16
.equ BOOT_INFO_EXPECTED_ID, 24
.equ BOOT_INFO_HART_LOCAL,  32
.equ BOOT_INFO_STATE,       40
.equ BOOT_INFO_STARTING,     1

secondary_entry:
  # SBI HSM entry contract:
  #   a0 = raw hart ID
  #   a1 = SecondaryBootInfo pointer supplied as opaque
  #   satp = 0, sstatsu.SIE = 0
  # Other registers are undefined and must not be cunsumed.

.Lwait_for_publication:
  ld    t0, BOOT_INFO_STATE(a1)
  li    t1, BOOT_INFO_STARTING
  bne   t0, t1, .Lwait_for_publication
  fence r, rw

  ld    sp, BOOT_INFO_STACK_TOP(a1)
  ld    t0, BOOT_INFO_SATP(a1)
  ld    t1, BOOT_INFO_HART_INDEX(a1)
  ld    t2, BOOT_INFO_EXPECTED_ID(a1)
  ld    tp, BOOT_INFO_HART_LOCAL(a1)

  bne   a0, t2, .Lsecondary_park
  andi  sp, sp, -16

  csrw  satp, t0
  sfence.vma zero, zero

  mv    a1, t1
  tail  secondary_kernel_entry

.Lsecondary_park:
  wfi
  j     .Lsecondary_park

.size secondary_entry, . - secondary_entry

.section .bss.secondary_stacks, "aw", @nobits
.balign 16
.global __secondary_stacks_start
__secondary_stacks_start:
  .space 64 * 16384
.global __secondary_stacks_end
__secondary_stacks_end:
