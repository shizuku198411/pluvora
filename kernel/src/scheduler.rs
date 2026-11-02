use crate::object::{MAX_OBJECTS, ObjectId};
use crate::platform::MAX_HARTS;
use crate::thread::MAX_PRIORITY;

pub const MAX_RUNNABLE_PER_HART: usize = MAX_OBJECTS;

const _: () = {
  assert!(MAX_HARTS > 0);
  assert!(MAX_HARTS <= u64::BITS as usize);
  assert!(MAX_RUNNABLE_PER_HART > 0);
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  AlreadyInitialized,
  InvalidOnlineMask,
  InvalidHartIndex,
  HartOffline,
  NoAllowedHart,
  QueueFull,
  DuplicateThread,
  HartBusy,
  NoRunningThread,
  ThreadNotQueued,
  CandidateChanged,
  InvalidPriority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WakeResult {
  pub target_hart: usize,
  pub request_reschedule: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
  thread: ObjectId,
  priority: u8,
}

#[derive(Clone, Copy)]
struct RunQueue {
  entries: [Option<Entry>; MAX_RUNNABLE_PER_HART],
  len: usize,
  cursor: usize,
}

impl RunQueue {
  const fn new() -> Self {
    Self {
      entries: [None; MAX_RUNNABLE_PER_HART],
      len: 0,
      cursor: 0,
    }
  }

  const fn len(&self) -> usize {
    self.len
  }

  fn contains(&self, thread: ObjectId) -> bool {
    self
      .entries
      .iter()
      .flatten()
      .any(|entry| entry.thread == thread)
  }

  fn push(&mut self, entry: Entry) -> Result<(), Error> {
    if entry.priority > MAX_PRIORITY {
      return Err(Error::InvalidPriority);
    }
    if self.contains(entry.thread) {
      return Err(Error::DuplicateThread);
    }
    let slot = self
      .entries
      .iter_mut()
      .find(|slot| slot.is_none())
      .ok_or(Error::QueueFull)?;
    *slot = Some(entry);
    self.len = self.len.checked_add(1).ok_or(Error::QueueFull)?;
    Ok(())
  }

  fn peek_next(&self) -> Option<Entry> {
    self.select_index().and_then(|index| self.entries[index])
  }

  fn pop_expected(&mut self, expected: ObjectId) -> Result<Entry, Error> {
    let index = self.select_index().ok_or(Error::ThreadNotQueued)?;
    let entry = self.entries[index].ok_or(Error::ThreadNotQueued)?;
    if entry.thread != expected {
      return Err(Error::CandidateChanged);
    }
    self.entries[index] = None;
    self.len = self.len.checked_sub(1).ok_or(Error::ThreadNotQueued)?;
    self.cursor = (index + 1) % MAX_RUNNABLE_PER_HART;
    Ok(entry)
  }

  fn remove(&mut self, thread: ObjectId) -> Result<Entry, Error> {
    let (index, entry) = self
      .entries
      .iter()
      .copied()
      .enumerate()
      .find_map(|(index, entry)| match entry {
        Some(entry) if entry.thread == thread => Some((index, entry)),
        _ => None,
      })
      .ok_or(Error::ThreadNotQueued)?;
    self.entries[index] = None;
    self.len = self.len.checked_sub(1).ok_or(Error::ThreadNotQueued)?;
    Ok(entry)
  }

  fn select_index(&self) -> Option<usize> {
    let mut selected = None;
    let mut selected_priority = 0u8;
    for offset in 0..MAX_RUNNABLE_PER_HART {
      let index = (self.cursor + offset) % MAX_RUNNABLE_PER_HART;
      let Some(entry) = self.entries[index] else {
        continue;
      };
      if selected.is_none() || entry.priority > selected_priority {
        selected = Some(index);
        selected_priority = entry.priority;
      }
    }
    selected
  }
}

pub struct SchedulerState {
  online_mask: u64,
  queues: [RunQueue; MAX_HARTS],
  running: [Option<Entry>; MAX_HARTS],
}

impl SchedulerState {
  pub const fn uninitialized() -> Self {
    Self {
      online_mask: 0,
      queues: [RunQueue::new(); MAX_HARTS],
      running: [None; MAX_HARTS],
    }
  }

  pub fn initialize(&mut self, online_mask: u64) -> Result<(), Error> {
    if self.is_initialized() {
      return Err(Error::AlreadyInitialized);
    }
    let valid = valid_hart_mask();
    if online_mask == 0 || online_mask & !valid != 0 {
      return Err(Error::InvalidOnlineMask);
    }
    self.online_mask = online_mask;
    Ok(())
  }

  pub const fn is_initialized(&self) -> bool {
    self.online_mask != 0
  }

  pub const fn online_mask(&self) -> u64 {
    self.online_mask
  }

  pub fn queue_len(&self, hart: usize) -> Result<usize, Error> {
    self.require_online(hart)?;
    Ok(self.queues[hart].len())
  }

  pub fn total_runnable(&self) -> usize {
    self.queues.iter().map(RunQueue::len).sum()
  }

  pub fn total_running(&self) -> usize {
    self.running.iter().flatten().count()
  }

  pub fn running_thread(&self, hart: usize) -> Result<Option<ObjectId>, Error> {
    self.require_online(hart)?;
    Ok(self.running[hart].map(|entry| entry.thread))
  }

  pub fn enqueue(
    &mut self,
    thread: ObjectId,
    affinity: u64,
    priority: u8,
  ) -> Result<WakeResult, Error> {
    if priority > MAX_PRIORITY {
      return Err(Error::InvalidPriority);
    }
    if self.contains(thread) {
      return Err(Error::DuplicateThread);
    }
    let target = self.choose_target(affinity)?;
    let running_priority = self.running[target].map(|entry| entry.priority);
    self.queues[target].push(Entry { thread, priority })?;
    Ok(WakeResult {
      target_hart: target,
      request_reschedule: running_priority.is_none_or(|running| priority > running),
    })
  }

  pub fn peek_next(&self, hart: usize) -> Result<Option<ObjectId>, Error> {
    self.require_online(hart)?;
    if self.running[hart].is_some() {
      return Err(Error::HartBusy);
    }
    Ok(self.queues[hart].peek_next().map(|entry| entry.thread))
  }

  pub fn dispatch_expected(&mut self, hart: usize, expected: ObjectId) -> Result<(), Error> {
    self.require_online(hart)?;
    if self.running[hart].is_some() {
      return Err(Error::HartBusy);
    }
    let entry = self.queues[hart].pop_expected(expected)?;
    self.running[hart] = Some(entry);
    Ok(())
  }

  pub fn preempt(&mut self, hart: usize, affinity: u64, priority: u8) -> Result<WakeResult, Error> {
    self.require_online(hart)?;
    let running = self.running[hart].ok_or(Error::NoRunningThread)?;
    if running.priority != priority {
      return Err(Error::CandidateChanged);
    }
    let target = self.choose_target_inner(affinity)?;
    if self.queues[target].len() == MAX_RUNNABLE_PER_HART {
      return Err(Error::QueueFull);
    }
    self.running[hart] = None;
    self.queues[target].push(running)?;
    Ok(WakeResult {
      target_hart: target,
      request_reschedule: true,
    })
  }

  pub fn block_running(&mut self, hart: usize) -> Result<ObjectId, Error> {
    self.require_online(hart)?;
    self.running[hart]
      .take()
      .map(|entry| entry.thread)
      .ok_or(Error::NoRunningThread)
  }

  pub fn remove_queued(&mut self, thread: ObjectId) -> Result<(), Error> {
    for queue in &mut self.queues {
      if queue.contains(thread) {
        queue.remove(thread)?;
        return Ok(());
      }
    }
    Err(Error::ThreadNotQueued)
  }

  pub fn stop_running(&mut self, hart: usize) -> Result<ObjectId, Error> {
    self.block_running(hart)
  }

  fn choose_target(&self, affinity: u64) -> Result<usize, Error> {
    self.choose_target_inner(affinity)
  }

  fn choose_target_inner(&self, affinity: u64) -> Result<usize, Error> {
    let allowed = affinity & self.online_mask;
    if allowed == 0 {
      return Err(Error::NoAllowedHart);
    }
    let mut selected = None;
    for hart in 0..MAX_HARTS {
      if allowed & (1u64 << hart) == 0 {
        continue;
      }
      let len = self.queues[hart].len();
      if len == MAX_RUNNABLE_PER_HART {
        continue;
      }
      if selected.is_none_or(|current: usize| len < self.queues[current].len()) {
        selected = Some(hart);
      }
    }
    selected.ok_or(Error::QueueFull)
  }

  fn contains(&self, thread: ObjectId) -> bool {
    self.queues.iter().any(|queue| queue.contains(thread))
      || self
        .running
        .iter()
        .flatten()
        .any(|entry| entry.thread == thread)
  }

  fn require_online(&self, hart: usize) -> Result<(), Error> {
    if hart >= MAX_HARTS {
      return Err(Error::InvalidHartIndex);
    }
    if self.online_mask & (1u64 << hart) == 0 {
      return Err(Error::HartOffline);
    }
    Ok(())
  }
}

const fn valid_hart_mask() -> u64 {
  u64::MAX >> (u64::BITS as usize - MAX_HARTS)
}

#[cfg(test)]
mod tests {
  use super::*;

  const fn thread(slot: u16) -> ObjectId {
    ObjectId::from_raw_parts(slot, 1)
  }

  fn scheduler() -> SchedulerState {
    let mut scheduler = SchedulerState::uninitialized();
    scheduler.initialize(0b1111).unwrap();
    scheduler
  }

  #[test]
  fn sched_001_each_online_hart_has_an_independent_queue() {
    let mut scheduler = scheduler();
    let first = scheduler.enqueue(thread(1), 0b0001, 1).unwrap();
    let second = scheduler.enqueue(thread(2), 0b0010, 1).unwrap();
    assert_eq!(first.target_hart, 0);
    assert_eq!(second.target_hart, 1);
    assert_eq!(scheduler.queue_len(0), Ok(1));
    assert_eq!(scheduler.queue_len(1), Ok(1));
  }

  #[test]
  fn sched_002_same_priority_is_round_robin() {
    let mut scheduler = scheduler();
    scheduler.enqueue(thread(1), 1, 5).unwrap();
    scheduler.enqueue(thread(2), 1, 5).unwrap();
    let first = scheduler.peek_next(0).unwrap().unwrap();
    scheduler.dispatch_expected(0, first).unwrap();
    scheduler.preempt(0, 1, 5).unwrap();
    let second = scheduler.peek_next(0).unwrap().unwrap();
    assert_ne!(first, second);
  }

  #[test]
  fn sched_004_affinity_and_online_mask_are_both_required() {
    let mut scheduler = SchedulerState::uninitialized();
    scheduler.initialize(0b0011).unwrap();
    assert_eq!(
      scheduler.enqueue(thread(1), 0b0100, 1),
      Err(Error::NoAllowedHart)
    );
    assert_eq!(scheduler.total_runnable(), 0);
  }

  #[test]
  fn sched_006_thread_cannot_exist_in_two_locations() {
    let mut scheduler = scheduler();
    scheduler.enqueue(thread(1), 0b1111, 1).unwrap();
    assert_eq!(
      scheduler.enqueue(thread(1), 0b1111, 1),
      Err(Error::DuplicateThread)
    );
    let target = (0..4)
      .find(|hart| scheduler.queue_len(*hart) == Ok(1))
      .unwrap();
    let next = scheduler.peek_next(target).unwrap().unwrap();
    scheduler.dispatch_expected(target, next).unwrap();
    assert_eq!(
      scheduler.enqueue(thread(1), 0b1111, 1),
      Err(Error::DuplicateThread)
    );
  }

  #[test]
  fn sched_007_higher_priority_runs_first() {
    let mut scheduler = scheduler();
    scheduler.enqueue(thread(1), 1, 2).unwrap();
    scheduler.enqueue(thread(2), 1, 20).unwrap();
    assert_eq!(scheduler.peek_next(0), Ok(Some(thread(2))));
  }

  #[test]
  fn sched_010_load_placement_uses_shortest_allowed_queue() {
    let mut scheduler = scheduler();
    assert_eq!(
      scheduler.enqueue(thread(1), 0b0011, 1).unwrap().target_hart,
      0
    );
    assert_eq!(
      scheduler.enqueue(thread(2), 0b0011, 1).unwrap().target_hart,
      1
    );
  }

  #[test]
  fn sched_011_block_removes_running_membership() {
    let mut scheduler = scheduler();
    scheduler.enqueue(thread(1), 1, 1).unwrap();
    scheduler.dispatch_expected(0, thread(1)).unwrap();
    assert_eq!(scheduler.block_running(0), Ok(thread(1)));
    assert_eq!(scheduler.running_thread(0), Ok(None));
    assert_eq!(scheduler.total_runnable(), 0);
  }

  #[test]
  fn sched_012_wake_requests_reschedule_for_idle_or_lower_priority_hart() {
    let mut scheduler = scheduler();
    let first = scheduler.enqueue(thread(1), 1, 4).unwrap();
    assert!(first.request_reschedule);
    scheduler.dispatch_expected(0, thread(1)).unwrap();
    let lower = scheduler.enqueue(thread(2), 1, 3).unwrap();
    assert!(!lower.request_reschedule);
    let higher = scheduler.enqueue(thread(3), 1, 5).unwrap();
    assert!(higher.request_reschedule);
  }
}
