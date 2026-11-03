use crate::arch::riscv64::context::{self, SavedContext};
use crate::memory::kernel_stack::KernelStack;
use crate::object::ObjectId;
use crate::platform::MAX_HARTS;

pub const MIN_PRIORITY: u8 = 0;
pub const MAX_PRIORITY: u8 = 31;

const _: () = {
  assert!(MAX_HARTS > 0);
  assert!(MAX_HARTS <= u64::BITS as usize);
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadState {
  New,
  Runnable,
  Running,
  Switching,
  Blocked,
  Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  InvalidAffinity,
  InvalidHartIndex,
  AffinityViolation,
  InvalidPriority,
  InvalidState,
  ExecutionAlreadyInstalled,
  ExecutionMissing,
  InvalidUserContext,
  CounterOverflow,
  Context(context::Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionKind {
  Kernel,
  User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserSwitchReason {
  Yield,
  Preempt,
}

pub struct Thread {
  process: ObjectId,
  execution: Option<ThreadExecution>,
  state: ThreadState,
  affinity: u64,
  base_priority: u8,
  effective_priority: u8,
  running_hart: Option<u16>,
  user_yields: u64,
  user_preemptions: u64,
}

impl Thread {
  pub fn new(process: ObjectId, affinity: u64, priority: u8) -> Result<Self, Error> {
    validate_affinity(affinity)?;
    validate_priority(priority)?;
    Ok(Self {
      process,
      execution: None,
      state: ThreadState::New,
      affinity,
      base_priority: priority,
      effective_priority: priority,
      running_hart: None,
      user_yields: 0,
      user_preemptions: 0,
    })
  }

  pub const fn process(&self) -> ObjectId {
    self.process
  }

  pub const fn state(&self) -> ThreadState {
    self.state
  }

  pub const fn affinity(&self) -> u64 {
    self.affinity
  }

  pub const fn base_priority(&self) -> u8 {
    self.base_priority
  }

  pub const fn effective_priority(&self) -> u8 {
    self.effective_priority
  }

  pub const fn running_hart(&self) -> Option<u16> {
    self.running_hart
  }

  pub const fn user_yields(&self) -> u64 {
    self.user_yields
  }

  pub const fn user_preemptions(&self) -> u64 {
    self.user_preemptions
  }

  pub(crate) fn record_user_switch(&mut self, reason: UserSwitchReason) -> Result<(), Error> {
    match reason {
      UserSwitchReason::Yield => {
        self.user_yields = self
          .user_yields
          .checked_add(1)
          .ok_or(Error::CounterOverflow)?;
      }
      UserSwitchReason::Preempt => {
        self.user_preemptions = self
          .user_preemptions
          .checked_add(1)
          .ok_or(Error::CounterOverflow)?;
      }
    }
    Ok(())
  }

  pub fn allows_hart(&self, index: usize) -> Result<bool, Error> {
    let bit = hart_bit(index)?;
    Ok(self.affinity & bit != 0)
  }

  pub fn make_runnable(&mut self) -> Result<(), Error> {
    if !matches!(self.state, ThreadState::New | ThreadState::Blocked) {
      return Err(Error::InvalidState);
    }
    self.state = ThreadState::Runnable;
    self.running_hart = None;
    Ok(())
  }

  pub fn begin_running(&mut self, hart_index: usize) -> Result<(), Error> {
    if self.state != ThreadState::Runnable {
      return Err(Error::InvalidState);
    }
    if !self.allows_hart(hart_index)? {
      return Err(Error::AffinityViolation);
    }
    let hart = u16::try_from(hart_index).map_err(|_| Error::InvalidHartIndex)?;
    self.state = ThreadState::Running;
    self.running_hart = Some(hart);
    Ok(())
  }

  pub fn preempt(&mut self) -> Result<(), Error> {
    if self.state != ThreadState::Running {
      return Err(Error::InvalidState);
    }
    self.state = ThreadState::Runnable;
    self.running_hart = None;
    Ok(())
  }

  pub(crate) fn begin_switch_out(&mut self) -> Result<(), Error> {
    if self.state != ThreadState::Running {
      return Err(Error::InvalidState);
    }
    self.state = ThreadState::Switching;
    self.running_hart = None;
    Ok(())
  }

  pub(crate) fn finish_yield(&mut self) -> Result<(), Error> {
    if self.state != ThreadState::Switching {
      return Err(Error::InvalidState);
    }
    self.state = ThreadState::Runnable;
    self.running_hart = None;
    Ok(())
  }

  pub fn block(&mut self) -> Result<(), Error> {
    if self.state != ThreadState::Running {
      return Err(Error::InvalidState);
    }
    self.state = ThreadState::Blocked;
    self.running_hart = None;
    Ok(())
  }

  pub fn stop(&mut self) -> Result<(), Error> {
    if matches!(self.state, ThreadState::Stopped | ThreadState::Switching) {
      return Err(Error::InvalidState);
    }
    self.state = ThreadState::Stopped;
    self.running_hart = None;
    Ok(())
  }

  #[cfg_attr(not(target_arch = "riscv64"), allow(dead_code))]
  pub(crate) fn stop_for_termination(&mut self) -> Result<(), Error> {
    if self.state == ThreadState::Stopped {
      return Err(Error::InvalidState);
    }
    self.state = ThreadState::Stopped;
    self.running_hart = None;
    Ok(())
  }

  pub fn set_effective_priority(&mut self, priority: u8) -> Result<(), Error> {
    validate_priority(priority)?;
    if priority < self.base_priority {
      return Err(Error::InvalidPriority);
    }
    self.effective_priority = priority;
    Ok(())
  }

  pub fn restore_base_priority(&mut self) {
    self.effective_priority = self.base_priority;
  }

  pub fn install_execution(
    &mut self,
    stack: KernelStack,
    entry: usize,
  ) -> Result<(), InstallExecutionError> {
    if self.execution.is_some() {
      return Err(InstallExecutionError {
        error: Error::ExecutionAlreadyInstalled,
        stack,
      });
    }
    let context = match SavedContext::initial(entry, stack.top()) {
      Ok(context) => context,
      Err(error) => {
        return Err(InstallExecutionError {
          error: Error::Context(error),
          stack,
        });
      }
    };
    self.execution = Some(ThreadExecution {
      context,
      kernel_stack: stack,
      kind: ExecutionKind::Kernel,
    });
    Ok(())
  }

  #[cfg(target_arch = "riscv64")]
  pub fn install_user_execution(
    &mut self,
    stack: KernelStack,
    entry: usize,
    user_stack_top: usize,
  ) -> Result<(), InstallExecutionError> {
    use core::mem::size_of;

    use crate::arch::riscv64::trap::TrapFrame;

    if self.execution.is_some() {
      return Err(InstallExecutionError {
        error: Error::ExecutionAlreadyInstalled,
        stack,
      });
    }
    let frame_address = match stack.top().checked_sub(size_of::<TrapFrame>()) {
      Some(address)
        if address >= stack.base() as usize
          && address.is_multiple_of(context::KERNEL_STACK_ALIGNMENT) =>
      {
        address
      }
      _ => {
        return Err(InstallExecutionError {
          error: Error::Context(context::Error::InvalidTrapFrame),
          stack,
        });
      }
    };
    let frame = match TrapFrame::initial_user(entry, user_stack_top) {
      Ok(frame) => frame,
      Err(_) => {
        return Err(InstallExecutionError {
          error: Error::InvalidUserContext,
          stack,
        });
      }
    };
    let context = match SavedContext::trap_resume(frame_address) {
      Ok(context) => context,
      Err(error) => {
        return Err(InstallExecutionError {
          error: Error::Context(error),
          stack,
        });
      }
    };

    // SAFETY: the owned KernelStack is live identity-mapped RAM and the frame
    // is aligned and fully contained before this New Thread is published.
    unsafe { (frame_address as *mut TrapFrame).write(frame) };
    self.execution = Some(ThreadExecution {
      context,
      kernel_stack: stack,
      kind: ExecutionKind::User,
    });
    Ok(())
  }

  pub const fn has_execution(&self) -> bool {
    self.execution.is_some()
  }

  pub const fn execution(&self) -> Option<&ThreadExecution> {
    self.execution.as_ref()
  }

  pub fn execution_mut(&mut self) -> Option<&mut ThreadExecution> {
    self.execution.as_mut()
  }

  pub fn into_execution(self) -> Option<ThreadExecution> {
    self.execution
  }
}

pub struct ThreadExecution {
  context: SavedContext,
  kernel_stack: KernelStack,
  kind: ExecutionKind,
}

impl ThreadExecution {
  pub const fn kind(&self) -> ExecutionKind {
    self.kind
  }

  pub const fn context(&self) -> &SavedContext {
    &self.context
  }

  pub fn context_mut(&mut self) -> &mut SavedContext {
    &mut self.context
  }

  #[cfg(target_arch = "riscv64")]
  pub(crate) fn prepare_user_dispatch(
    &mut self,
    kernel_tp: usize,
  ) -> Result<usize, crate::arch::riscv64::trap::BoundaryError> {
    if self.kind != ExecutionKind::User {
      return Err(crate::arch::riscv64::trap::BoundaryError::InvalidKernelStack);
    }
    let kernel_stack_top = self.kernel_stack.top();
    let frame_address = self.context.stack_pointer();
    let frame = frame_address as *mut crate::arch::riscv64::trap::TrapFrame;
    if frame.is_null() {
      return Err(crate::arch::riscv64::trap::BoundaryError::InvalidKernelStack);
    }
    // SAFETY: User Thread execution owns this live TrapFrame for its lifetime;
    // KERNEL_STATE serializes metadata updates before dispatch publication.
    unsafe { &mut *frame }.prepare_user_dispatch(kernel_tp, kernel_stack_top)?;
    Ok(kernel_stack_top)
  }

  pub const fn kernel_stack(&self) -> &KernelStack {
    &self.kernel_stack
  }

  pub fn into_kernel_stack(self) -> KernelStack {
    self.kernel_stack
  }
}

pub struct InstallExecutionError {
  error: Error,
  stack: KernelStack,
}

impl InstallExecutionError {
  pub const fn error(&self) -> Error {
    self.error
  }

  pub fn into_parts(self) -> (Error, KernelStack) {
    (self.error, self.stack)
  }
}

fn validate_affinity(affinity: u64) -> Result<(), Error> {
  let valid = u64::MAX >> (u64::BITS as usize - MAX_HARTS);
  if affinity == 0 || affinity & !valid != 0 {
    return Err(Error::InvalidAffinity);
  }
  Ok(())
}

fn validate_priority(priority: u8) -> Result<(), Error> {
  if !(MIN_PRIORITY..=MAX_PRIORITY).contains(&priority) {
    return Err(Error::InvalidPriority);
  }
  Ok(())
}

fn hart_bit(index: usize) -> Result<u64, Error> {
  if index >= MAX_HARTS || index >= u64::BITS as usize {
    return Err(Error::InvalidHartIndex);
  }
  Ok(1u64 << index)
}

#[cfg(test)]
mod tests {
  use super::*;

  const PROCESS: ObjectId = ObjectId::from_raw_parts(2, 1);

  fn thread() -> Thread {
    Thread::new(PROCESS, 0b0101, 8).unwrap()
  }

  #[test]
  fn thread_001_new_thread_preserves_owner_priority_and_affinity() {
    let thread = thread();
    assert_eq!(thread.process(), PROCESS);
    assert_eq!(thread.state(), ThreadState::New);
    assert_eq!(thread.affinity(), 0b0101);
    assert_eq!(thread.base_priority(), 8);
    assert_eq!(thread.effective_priority(), 8);
    assert_eq!(thread.running_hart(), None);
  }

  #[test]
  fn thread_002_runnable_thread_runs_only_on_allowed_hart() {
    let mut thread = thread();
    thread.make_runnable().unwrap();
    assert_eq!(thread.begin_running(1), Err(Error::AffinityViolation));
    assert_eq!(thread.state(), ThreadState::Runnable);
    assert_eq!(thread.running_hart(), None);
    thread.begin_running(2).unwrap();
    assert_eq!(thread.state(), ThreadState::Running);
    assert_eq!(thread.running_hart(), Some(2));
  }

  #[test]
  fn thread_003_preemption_clears_running_hart() {
    let mut thread = thread();
    thread.make_runnable().unwrap();
    thread.begin_running(0).unwrap();
    thread.preempt().unwrap();
    assert_eq!(thread.state(), ThreadState::Runnable);
    assert_eq!(thread.running_hart(), None);
  }

  #[test]
  fn thread_004_block_and_wake_follow_state_machine() {
    let mut thread = thread();
    thread.make_runnable().unwrap();
    thread.begin_running(0).unwrap();
    thread.block().unwrap();
    assert_eq!(thread.state(), ThreadState::Blocked);
    thread.make_runnable().unwrap();
    assert_eq!(thread.state(), ThreadState::Runnable);
  }

  #[test]
  fn thread_005_stopped_thread_cannot_restart() {
    let mut thread = thread();
    thread.stop().unwrap();
    assert_eq!(thread.state(), ThreadState::Stopped);
    assert_eq!(thread.make_runnable(), Err(Error::InvalidState));
    assert_eq!(thread.stop(), Err(Error::InvalidState));
  }

  #[test]
  fn thread_006_invalid_affinity_is_rejected() {
    assert!(matches!(
      Thread::new(PROCESS, 0, 8),
      Err(Error::InvalidAffinity)
    ));
    if let Some(out_of_range) = 1u64.checked_shl(MAX_HARTS as u32) {
      assert!(matches!(
        Thread::new(PROCESS, out_of_range, 8),
        Err(Error::InvalidAffinity)
      ));
    }
  }

  #[test]
  fn thread_007_invalid_priority_is_rejected() {
    assert!(matches!(
      Thread::new(PROCESS, 1, MAX_PRIORITY + 1),
      Err(Error::InvalidPriority)
    ));
  }

  #[test]
  fn thread_008_priority_donation_state_can_be_restored() {
    let mut thread = thread();
    assert_eq!(
      thread.set_effective_priority(7),
      Err(Error::InvalidPriority)
    );
    thread.set_effective_priority(20).unwrap();
    assert_eq!(thread.effective_priority(), 20);
    thread.restore_base_priority();
    assert_eq!(thread.effective_priority(), thread.base_priority());
  }

  #[test]
  fn thread_009_invalid_transition_does_not_change_state() {
    let mut thread = thread();
    assert_eq!(thread.begin_running(0), Err(Error::InvalidState));
    assert_eq!(thread.state(), ThreadState::New);
    assert_eq!(thread.running_hart(), None);
  }

  #[test]
  fn thread_010_execution_resources_install_once() {
    use crate::memory::kernel_stack::kernel_stack_for_test;

    let mut thread = Thread::new(PROCESS, 1, 8).unwrap();
    assert!(!thread.has_execution());
    thread
      .install_execution(kernel_stack_for_test(0x9000_0000), 0x8020_0000)
      .unwrap_or_else(|_| panic!("test execution installation failed"));
    assert!(thread.has_execution());
    assert_eq!(
      thread.execution().unwrap().kernel_stack().base(),
      0x9000_0000
    );
    let failure = thread
      .install_execution(kernel_stack_for_test(0x9001_0000), 0x8020_0000)
      .expect_err("second execution install must fail");
    assert_eq!(failure.error(), Error::ExecutionAlreadyInstalled);
    assert_eq!(failure.into_parts().1.base(), 0x9001_0000);
  }

  #[test]
  fn thread_011_switching_requires_running_and_finishes_runnable() {
    let mut thread = thread();
    assert_eq!(thread.begin_switch_out(), Err(Error::InvalidState));
    thread.make_runnable().unwrap();
    thread.begin_running(0).unwrap();
    thread.begin_switch_out().unwrap();
    assert_eq!(thread.state(), ThreadState::Switching);
    assert_eq!(thread.running_hart(), None);
    thread.finish_yield().unwrap();
    assert_eq!(thread.state(), ThreadState::Runnable);
  }

  #[test]
  fn thread_012_switching_cannot_use_general_wake_transition() {
    let mut thread = thread();
    thread.make_runnable().unwrap();
    thread.begin_running(0).unwrap();
    thread.begin_switch_out().unwrap();
    assert_eq!(thread.make_runnable(), Err(Error::InvalidState));
    assert_eq!(thread.stop(), Err(Error::InvalidState));
    assert_eq!(thread.state(), ThreadState::Switching);
  }

  #[test]
  fn thread_013_user_switch_counters_are_independent() {
    let mut thread = thread();
    thread.record_user_switch(UserSwitchReason::Yield).unwrap();
    thread.record_user_switch(UserSwitchReason::Yield).unwrap();
    thread
      .record_user_switch(UserSwitchReason::Preempt)
      .unwrap();
    assert_eq!(thread.user_yields(), 2);
    assert_eq!(thread.user_preemptions(), 1);
    assert_eq!(thread.state(), ThreadState::New);
  }

  #[test]
  fn thread_014_user_switch_counter_rejects_overflow() {
    let mut thread = thread();
    thread.user_yields = u64::MAX;
    assert_eq!(
      thread.record_user_switch(UserSwitchReason::Yield),
      Err(Error::CounterOverflow)
    );
    assert_eq!(thread.user_yields(), u64::MAX);
    assert_eq!(thread.user_preemptions(), 0);
  }

  #[test]
  fn thread_015_termination_can_stop_switching_thread() {
    let mut thread = thread();
    thread.make_runnable().unwrap();
    thread.begin_running(0).unwrap();
    thread.begin_switch_out().unwrap();
    thread.stop_for_termination().unwrap();
    assert_eq!(thread.state(), ThreadState::Stopped);
    assert_eq!(thread.running_hart(), None);
    assert_eq!(thread.stop_for_termination(), Err(Error::InvalidState));
  }
}
