.section .text.context, "ax"
.global switch_context
.type switch_context, @function
.align 2

.equ CONTEXT_RA,   0 * 8
.equ CONTEXT_SP,   1 * 8
.equ CONTEXT_S0,   2 * 8
.equ CONTEXT_S1,   3 * 8
.equ CONTEXT_S2,   4 * 8
.equ CONTEXT_S3,   5 * 8
.equ CONTEXT_S4,   6 * 8
.equ CONTEXT_S5,   7 * 8
.equ CONTEXT_S6,   8 * 8
.equ CONTEXT_S7,   9 * 8
.equ CONTEXT_S8,  10 * 8
.equ CONTEXT_S9,  11 * 8
.equ CONTEXT_S10, 12 * 8
.equ CONTEXT_S11, 13 * 8

# a0 = mutable SavedContext for the outgoing execution
# a1 = immutable SavedContext for the incoming execution
switch_context:
  sd    ra,   CONTEXT_RA(a0)
  sd    sp,   CONTEXT_SP(a0)
  sd    s0,   CONTEXT_S0(a0)
  sd    s1,   CONTEXT_S1(a0)
  sd    s2,   CONTEXT_S2(a0)
  sd    s3,   CONTEXT_S3(a0)
  sd    s4,   CONTEXT_S4(a0)
  sd    s5,   CONTEXT_S5(a0)
  sd    s6,   CONTEXT_S6(a0)
  sd    s7,   CONTEXT_S7(a0)
  sd    s8,   CONTEXT_S8(a0)
  sd    s9,   CONTEXT_S9(a0)
  sd    s10,  CONTEXT_S10(a0)
  sd    s11,  CONTEXT_S11(a0)

  ld    ra,   CONTEXT_RA(a1)
  ld    sp,   CONTEXT_SP(a1)
  ld    s0,   CONTEXT_S0(a1)
  ld    s1,   CONTEXT_S1(a1)
  ld    s2,   CONTEXT_S2(a1)
  ld    s3,   CONTEXT_S3(a1)
  ld    s4,   CONTEXT_S4(a1)
  ld    s5,   CONTEXT_S5(a1)
  ld    s6,   CONTEXT_S6(a1)
  ld    s7,   CONTEXT_S7(a1)
  ld    s8,   CONTEXT_S8(a1)
  ld    s9,   CONTEXT_S9(a1)
  ld    s10,  CONTEXT_S10(a1)
  ld    s11,  CONTEXT_S11(a1)
  ret

.size switch_context, . - switch_context

.global switch_context_to_thread
.type switch_context_to_thread, @function
.align 2

# a0 = mutable scheduler SavedContext
# a1 = immutable Thread SavedContext
switch_context_to_thread:
  sd    ra,   CONTEXT_RA(a0)
  sd    sp,   CONTEXT_SP(a0)
  sd    s0,   CONTEXT_S0(a0)
  sd    s1,   CONTEXT_S1(a0)
  sd    s2,   CONTEXT_S2(a0)
  sd    s3,   CONTEXT_S3(a0)
  sd    s4,   CONTEXT_S4(a0)
  sd    s5,   CONTEXT_S5(a0)
  sd    s6,   CONTEXT_S6(a0)
  sd    s7,   CONTEXT_S7(a0)
  sd    s8,   CONTEXT_S8(a0)
  sd    s9,   CONTEXT_S9(a0)
  sd    s10,  CONTEXT_S10(a0)
  sd    s11,  CONTEXT_S11(a0)

  ld    ra,   CONTEXT_RA(a1)
  ld    sp,   CONTEXT_SP(a1)
  ld    s0,   CONTEXT_S0(a1)
  ld    s1,   CONTEXT_S1(a1)
  ld    s2,   CONTEXT_S2(a1)
  ld    s3,   CONTEXT_S3(a1)
  ld    s4,   CONTEXT_S4(a1)
  ld    s5,   CONTEXT_S5(a1)
  ld    s6,   CONTEXT_S6(a1)
  ld    s7,   CONTEXT_S7(a1)
  ld    s8,   CONTEXT_S8(a1)
  ld    s9,   CONTEXT_S9(a1)
  ld    s10,  CONTEXT_S10(a1)
  ld    s11,  CONTEXT_S11(a1)

  # Enable interrupts only after the incoming Thread stack and registers are live.
  csrsi sstatus, 2
  ret

.size switch_context_to_thread, . - switch_context_to_thread

.global restore_context
.type restore_context, @function
.align 2

# a0 = immutable SavedContext for the incoming execution
# This function never returns to its caller.
restore_context:
  ld    ra,   CONTEXT_RA(a0)
  ld    sp,   CONTEXT_SP(a0)
  ld    s0,   CONTEXT_S0(a0)
  ld    s1,   CONTEXT_S1(a0)
  ld    s2,   CONTEXT_S2(a0)
  ld    s3,   CONTEXT_S3(a0)
  ld    s4,   CONTEXT_S4(a0)
  ld    s5,   CONTEXT_S5(a0)
  ld    s6,   CONTEXT_S6(a0)
  ld    s7,   CONTEXT_S7(a0)
  ld    s8,   CONTEXT_S8(a0)
  ld    s9,   CONTEXT_S9(a0)
  ld    s10,  CONTEXT_S10(a0)
  ld    s11,  CONTEXT_S11(a0)
  ret

.size restore_context, . - restore_context
