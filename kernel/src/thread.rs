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
}

pub struct Thread {
  process: ObjectId,
  state: ThreadState,
  affinity: u64,
  base_priority: u8,
  effective_priority: u8,
  running_hart: Option<u16>,
}

impl Thread {
  pub fn new(process: ObjectId, affinity: u64, priority: u8) -> Result<Self, Error> {
    validate_affinity(affinity)?;
    validate_priority(priority)?;
    Ok(Self {
      process,
      state: ThreadState::New,
      affinity,
      base_priority: priority,
      effective_priority: priority,
      running_hart: None,
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

  pub fn block(&mut self) -> Result<(), Error> {
    if self.state != ThreadState::Running {
      return Err(Error::InvalidState);
    }
    self.state = ThreadState::Blocked;
    self.running_hart = None;
    Ok(())
  }

  pub fn stop(&mut self) -> Result<(), Error> {
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
}
