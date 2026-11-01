.section .text.trap, "ax"
.global trap_entry
.type trap_entry, @function
.align 2

.equ TRAP_FRAME_SIZE, 288

trap_entry:
  addi    sp, sp, -TRAP_FRAME_SIZE

  # save t0 before using it to reconstruct the pre-trap sp.
  sd      t0, 5 * 8(sp)
  addi    t0, sp, TRAP_FRAME_SIZE

  sd    zero, 0 * 8(sp)
  sd    ra,   1 * 8(sp)
  sd    t0,   2 * 8(sp)
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

  ld    t0, 32 * 8(sp)
  ld    t1, 33 * 8(sp)
  csrw  sstatus, t0
  csrw  sepc, t1

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

  addi  sp, sp, TRAP_FRAME_SIZE
  sret

.size trap_entry, . - trap_entry
