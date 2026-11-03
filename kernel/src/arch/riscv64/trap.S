.section .text.trap, "ax"
.global trap_entry
.type trap_entry, @function
.global resume_trap_context
.type resume_trap_context, @function
.align 2

.equ TRAP_FRAME_SIZE, 288
.equ SSTATUS_SPP, 1 << 8

trap_entry:
  # sscratch == 0 means S-mode owns sp. A non-zero value is an armed
  # user Thread kernel stack top.
  csrrw   sp, sscratch, sp
  bnez    sp, .Ltrap_from_user

  # S-origin: restore the kernel sp and clear sscratch before allocating
  # the TrapFrame on the current kernel stack.
  csrrw   sp, sscratch, sp
  addi    sp, sp, -TRAP_FRAME_SIZE
  sd      t0, 5 * 8(sp)
  addi    t0, sp, TRAP_FRAME_SIZE
  sd      t0, 2 * 8(sp)
  j       .Ltrap_stack_ready

.Ltrap_from_user:
  # U-origin: sp is the armed kernel stack top and sscratch is the original
  # user sp. Clear sscratch before entering the Rust handler so nested
  # S-mode traps continue to use their current kernel sp.
  addi    sp, sp, -TRAP_FRAME_SIZE
  sd      t0, 5 * 8(sp)
  csrr    t0, sscratch
  sd      t0, 2 * 8(sp)
  csrw    sscratch, zero

.Ltrap_stack_ready:
  sd    zero, 0 * 8(sp)
  sd    ra,   1 * 8(sp)
  sd    gp,   3 * 8(sp)
  sd    tp,   4 * 8(sp)
  sd    t1,   6 * 8(sp)
  sd    t2,   7 * 8(sp)
  sd    s0,   8 * 8(sp)
  sd    s1,   9 * 8(sp)
  sd    a0,  10 * 8(sp)
  sd    a1,  11 * 8(sp)
  sd    a2,  12 * 8(sp)
  sd    a3,  13 * 8(sp)
  sd    a4,  14 * 8(sp)
  sd    a5,  15 * 8(sp)
  sd    a6,  16 * 8(sp)
  sd    a7,  17 * 8(sp)
  sd    s2,  18 * 8(sp)
  sd    s3,  19 * 8(sp)
  sd    s4,  20 * 8(sp)
  sd    s5,  21 * 8(sp)
  sd    s6,  22 * 8(sp)
  sd    s7,  23 * 8(sp)
  sd    s8,  24 * 8(sp)
  sd    s9,  25 * 8(sp)
  sd    s10, 26 * 8(sp)
  sd    s11, 27 * 8(sp)
  sd    t3,  28 * 8(sp)
  sd    t4,  29 * 8(sp)
  sd    t5,  30 * 8(sp)
  sd    t6,  31 * 8(sp)

  csrr  t0, sstatus
  csrr  t1, sepc
  csrr  t2, scause
  csrr  t3, stval
  sd    t0, 32 * 8(sp)
  sd    t1, 33 * 8(sp)
  sd    t2, 34 * 8(sp)
  sd    t3, 35 * 8(sp)

  mv    a0, sp
  call  trap_handler

resume_trap_context:
  ld    t0, 32 * 8(sp)
  ld    t1, 33 * 8(sp)
  csrw  sstatus, t0
  csrw  sepc, t1

  andi  t2, t0, SSTATUS_SPP
  bnez  t2, .Lresume_supervisor

  # U-return: re-arm the kernel stack top for the next user trap.
  addi  t2, sp, TRAP_FRAME_SIZE
  csrw  sscratch, t2
  j     .Lrestore_trap_registers

.Lresume_supervisor:
  csrw  sscratch, zero

.Lrestore_trap_registers:

  ld    ra,   1 * 8(sp)
  ld    gp,   3 * 8(sp)
  ld    tp,   4 * 8(sp)
  ld    t0,   5 * 8(sp)
  ld    t1,   6 * 8(sp)
  ld    t2,   7 * 8(sp)
  ld    s0,   8 * 8(sp)
  ld    s1,   9 * 8(sp)
  ld    a0,  10 * 8(sp)
  ld    a1,  11 * 8(sp)
  ld    a2,  12 * 8(sp)
  ld    a3,  13 * 8(sp)
  ld    a4,  14 * 8(sp)
  ld    a5,  15 * 8(sp)
  ld    a6,  16 * 8(sp)
  ld    a7,  17 * 8(sp)
  ld    s2,  18 * 8(sp)
  ld    s3,  19 * 8(sp)
  ld    s4,  20 * 8(sp)
  ld    s5,  21 * 8(sp)
  ld    s6,  22 * 8(sp)
  ld    s7,  23 * 8(sp)
  ld    s8,  24 * 8(sp)
  ld    s9,  25 * 8(sp)
  ld    s10, 26 * 8(sp)
  ld    s11, 27 * 8(sp)
  ld    t3,  28 * 8(sp)
  ld    t4,  29 * 8(sp)
  ld    t5,  30 * 8(sp)
  ld    t6,  31 * 8(sp)

  # Restore the exact pre-trap sp last while the TrapFrame base is live.
  ld    sp,   2 * 8(sp)
  sret

.size trap_entry, . - trap_entry
.size resume_trap_context, . - resume_trap_context
