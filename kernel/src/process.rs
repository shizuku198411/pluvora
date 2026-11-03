use crate::capability::MAX_CAPABILITY_SLOTS;
use crate::capability_space::CapabilitySpaceId;
use crate::object::ObjectId;

pub const MAX_THREADS_PER_PROCESS: u16 = 64;
pub const MAX_PAGES_PER_PROCESS: u32 = 65_536;
pub const MAX_IPC_QUEUE_CAPACITY_PER_PROCESS: u32 = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessState {
  Created,
  Alive,
  Stopping,
  Dead,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationReason {
  Exit {
    status: usize,
  },
  UnknownSyscall {
    number: usize,
  },
  IllegalInstruction {
    sepc: usize,
    stval: usize,
  },
  Breakpoint {
    sepc: usize,
  },
  InstructionPageFault {
    sepc: usize,
    stval: usize,
  },
  LoadPageFault {
    sepc: usize,
    stval: usize,
  },
  StorePageFault {
    sepc: usize,
    stval: usize,
  },
  UnknownException {
    code: usize,
    sepc: usize,
    stval: usize,
  },
}

impl TerminationReason {
  pub const fn name(self) -> &'static str {
    match self {
      Self::Exit { .. } => "exit",
      Self::UnknownSyscall { .. } => "unknown syscall",
      Self::IllegalInstruction { .. } => "illegal instruction",
      Self::Breakpoint { .. } => "breakpoint",
      Self::InstructionPageFault { .. } => "instruction page fault",
      Self::LoadPageFault { .. } => "load page fault",
      Self::StorePageFault { .. } => "store page fault",
      Self::UnknownException { .. } => "unknown user exception",
    }
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationProgress {
  Pending,
  Quiescent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeathNotificationState {
  Pending,
  Delivered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminationRecord {
  reason: TerminationReason,
  progress: TerminationProgress,
  notification: DeathNotificationState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BeginTermination {
  pub record: TerminationRecord,
  pub leader: bool,
}

impl TerminationRecord {
  pub const fn reason(self) -> TerminationReason {
    self.reason
  }
  pub const fn progress(self) -> TerminationProgress {
    self.progress
  }
  pub const fn notification(self) -> DeathNotificationState {
    self.notification
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  InvalidLimit,
  InvalidState,
  ResourceLimit,
  ResourceUnderflow,
  ResourcesStillLive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessLimits {
  max_threads: u16,
  max_pages: u32,
  max_capability_slots: u16,
  max_ipc_queue_capacity: u32,
}

impl ProcessLimits {
  pub fn new(
    max_threads: u16,
    max_pages: u32,
    max_capability_slots: u16,
    max_ipc_queue_capacity: u32,
  ) -> Result<Self, Error> {
    if max_threads == 0
      || max_threads > MAX_THREADS_PER_PROCESS
      || max_pages == 0
      || max_pages > MAX_PAGES_PER_PROCESS
      || max_capability_slots == 0
      || usize::from(max_capability_slots) > MAX_CAPABILITY_SLOTS
      || max_ipc_queue_capacity == 0
      || max_ipc_queue_capacity > MAX_IPC_QUEUE_CAPACITY_PER_PROCESS
    {
      return Err(Error::InvalidLimit);
    }
    Ok(Self {
      max_threads,
      max_pages,
      max_capability_slots,
      max_ipc_queue_capacity,
    })
  }

  pub const fn max_threads(self) -> u16 {
    self.max_threads
  }

  pub const fn max_pages(self) -> u32 {
    self.max_pages
  }

  pub const fn max_capability_slots(self) -> u16 {
    self.max_capability_slots
  }

  pub const fn max_ipc_queue_capacity(self) -> u32 {
    self.max_ipc_queue_capacity
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceUsage {
  threads: u16,
  pages: u32,
  capability_slots: u16,
  ipc_queue_capacity: u32,
}

impl ResourceUsage {
  pub const fn threads(self) -> u16 {
    self.threads
  }

  pub const fn pages(self) -> u32 {
    self.pages
  }

  pub const fn capability_slots(self) -> u16 {
    self.capability_slots
  }

  pub const fn ipc_queue_capacity(self) -> u32 {
    self.ipc_queue_capacity
  }

  pub const fn is_empty(self) -> bool {
    self.threads == 0
      && self.pages == 0
      && self.capability_slots == 0
      && self.ipc_queue_capacity == 0
  }

  fn charge(&mut self, limits: ProcessLimits, charge: ResourceCharge) -> Result<(), Error> {
    let threads = self
      .threads
      .checked_add(charge.threads)
      .ok_or(Error::ResourceLimit)?;
    let pages = self
      .pages
      .checked_add(charge.pages)
      .ok_or(Error::ResourceLimit)?;
    let capability_slots = self
      .capability_slots
      .checked_add(charge.capability_slots)
      .ok_or(Error::ResourceLimit)?;
    let ipc_queue_capacity = self
      .ipc_queue_capacity
      .checked_add(charge.ipc_queue_capacity)
      .ok_or(Error::ResourceLimit)?;

    if threads > limits.max_threads
      || pages > limits.max_pages
      || capability_slots > limits.max_capability_slots
      || ipc_queue_capacity > limits.max_ipc_queue_capacity
    {
      return Err(Error::ResourceLimit);
    }

    self.threads = threads;
    self.pages = pages;
    self.capability_slots = capability_slots;
    self.ipc_queue_capacity = ipc_queue_capacity;
    Ok(())
  }

  fn release(&mut self, release: ResourceCharge) -> Result<(), Error> {
    let threads = self
      .threads
      .checked_sub(release.threads)
      .ok_or(Error::ResourceUnderflow)?;
    let pages = self
      .pages
      .checked_sub(release.pages)
      .ok_or(Error::ResourceUnderflow)?;
    let capability_slots = self
      .capability_slots
      .checked_sub(release.capability_slots)
      .ok_or(Error::ResourceUnderflow)?;
    let ipc_queue_capacity = self
      .ipc_queue_capacity
      .checked_sub(release.ipc_queue_capacity)
      .ok_or(Error::ResourceUnderflow)?;

    self.threads = threads;
    self.pages = pages;
    self.capability_slots = capability_slots;
    self.ipc_queue_capacity = ipc_queue_capacity;
    Ok(())
  }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceCharge {
  pub threads: u16,
  pub pages: u32,
  pub capability_slots: u16,
  pub ipc_queue_capacity: u32,
}

pub struct Process {
  address_space: ObjectId,
  capability_space: CapabilitySpaceId,
  limits: ProcessLimits,
  usage: ResourceUsage,
  state: ProcessState,
  termination: Option<TerminationRecord>,
}

impl Process {
  pub const fn new(
    address_space: ObjectId,
    capability_space: CapabilitySpaceId,
    limits: ProcessLimits,
  ) -> Self {
    Self {
      address_space,
      capability_space,
      limits,
      usage: ResourceUsage {
        threads: 0,
        pages: 0,
        capability_slots: 0,
        ipc_queue_capacity: 0,
      },
      state: ProcessState::Created,
      termination: None,
    }
  }

  pub const fn address_space(&self) -> ObjectId {
    self.address_space
  }

  pub const fn capability_space(&self) -> CapabilitySpaceId {
    self.capability_space
  }

  pub const fn limits(&self) -> ProcessLimits {
    self.limits
  }

  pub const fn usage(&self) -> ResourceUsage {
    self.usage
  }

  pub const fn state(&self) -> ProcessState {
    self.state
  }

  pub const fn termination(&self) -> Option<TerminationRecord> {
    self.termination
  }

  pub fn begin_termination_once(
    &mut self,
    reason: TerminationReason,
  ) -> Result<BeginTermination, Error> {
    match (self.state, self.termination) {
      (ProcessState::Alive, None) => {
        let record = TerminationRecord {
          reason,
          progress: TerminationProgress::Pending,
          notification: DeathNotificationState::Pending,
        };
        self.state = ProcessState::Stopping;
        self.termination = Some(record);
        Ok(BeginTermination {
          record,
          leader: true,
        })
      }
      (ProcessState::Stopping, Some(record)) => Ok(BeginTermination {
        record,
        leader: false,
      }),
      _ => Err(Error::InvalidState),
    }
  }

  pub fn mark_termination_quiescent(&mut self) -> Result<(), Error> {
    if self.state != ProcessState::Stopping {
      return Err(Error::InvalidState);
    }
    let record = self.termination.as_mut().ok_or(Error::InvalidState)?;
    record.progress = TerminationProgress::Quiescent;
    Ok(())
  }

  pub fn replace_address_space(&mut self, address_space: ObjectId) -> Result<ObjectId, Error> {
    if self.state != ProcessState::Alive || self.usage.threads != 0 {
      return Err(Error::InvalidState);
    }
    let previous = self.address_space;
    self.address_space = address_space;
    Ok(previous)
  }

  pub fn start(&mut self) -> Result<(), Error> {
    if self.state != ProcessState::Created {
      return Err(Error::InvalidState);
    }
    self.state = ProcessState::Alive;
    Ok(())
  }

  pub fn charge(&mut self, charge: ResourceCharge) -> Result<(), Error> {
    if !matches!(self.state, ProcessState::Created | ProcessState::Alive) {
      return Err(Error::InvalidState);
    }
    self.usage.charge(self.limits, charge)
  }

  pub fn can_charge(&self, charge: ResourceCharge) -> Result<(), Error> {
    if !matches!(self.state, ProcessState::Created | ProcessState::Alive) {
      return Err(Error::InvalidState);
    }
    let mut usage = self.usage;
    usage.charge(self.limits, charge)
  }

  pub fn release(&mut self, release: ResourceCharge) -> Result<(), Error> {
    if self.state == ProcessState::Dead {
      return Err(Error::InvalidState);
    }
    self.usage.release(release)
  }

  pub fn begin_stop(&mut self) -> Result<(), Error> {
    if self.state != ProcessState::Alive {
      return Err(Error::InvalidState);
    }
    self.state = ProcessState::Stopping;
    Ok(())
  }

  pub fn finish_stop(&mut self) -> Result<(), Error> {
    if self.state != ProcessState::Stopping {
      return Err(Error::InvalidState);
    }
    if !self.usage.is_empty() {
      return Err(Error::ResourcesStillLive);
    }
    self.state = ProcessState::Dead;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const ADDRESS_SPACE: ObjectId = ObjectId::from_raw_parts(1, 1);

  fn capability_space() -> CapabilitySpaceId {
    CapabilitySpaceId::from_raw_parts_for_test(1, 1)
  }

  fn limits() -> ProcessLimits {
    ProcessLimits::new(2, 4, 8, 16).unwrap()
  }

  fn alive_process() -> Process {
    let mut process = Process::new(ADDRESS_SPACE, capability_space(), limits());
    process.start().unwrap();
    process
  }

  #[test]
  fn proc_010_process_starts_with_distinct_resource_identities() {
    let process = Process::new(ADDRESS_SPACE, capability_space(), limits());
    assert_eq!(process.address_space(), ADDRESS_SPACE);
    assert_eq!(process.capability_space(), capability_space());
    assert_eq!(process.state(), ProcessState::Created);
    assert!(process.usage().is_empty());
  }

  #[test]
  fn proc_011_lifecycle_is_one_way() {
    let mut process = alive_process();
    process.begin_stop().unwrap();
    process.finish_stop().unwrap();
    assert_eq!(process.state(), ProcessState::Dead);
    assert_eq!(process.start(), Err(Error::InvalidState));
    assert_eq!(process.begin_stop(), Err(Error::InvalidState));
  }

  #[test]
  fn limit_003_thread_limit_is_enforced_without_partial_charge() {
    let mut process = alive_process();
    process
      .charge(ResourceCharge {
        threads: 2,
        ..ResourceCharge::default()
      })
      .unwrap();
    let before = process.usage();
    assert_eq!(
      process.charge(ResourceCharge {
        threads: 1,
        ..ResourceCharge::default()
      }),
      Err(Error::ResourceLimit)
    );
    assert_eq!(process.usage(), before);
  }

  #[test]
  fn limit_001_page_limit_is_enforced() {
    let mut process = alive_process();
    assert_eq!(
      process.charge(ResourceCharge {
        pages: limits().max_pages() + 1,
        ..ResourceCharge::default()
      }),
      Err(Error::ResourceLimit)
    );
    assert!(process.usage().is_empty());
  }

  #[test]
  fn limit_002_capability_slot_limit_is_enforced() {
    let mut process = alive_process();
    assert_eq!(
      process.charge(ResourceCharge {
        capability_slots: limits().max_capability_slots() + 1,
        ..ResourceCharge::default()
      }),
      Err(Error::ResourceLimit)
    );
    assert!(process.usage().is_empty());
  }

  #[test]
  fn limit_004_ipc_queue_capacity_is_enforced() {
    let mut process = alive_process();
    assert_eq!(
      process.charge(ResourceCharge {
        ipc_queue_capacity: limits().max_ipc_queue_capacity() + 1,
        ..ResourceCharge::default()
      }),
      Err(Error::ResourceLimit)
    );
    assert!(process.usage().is_empty());
  }

  #[test]
  fn limit_005_multi_resource_failure_is_atomic() {
    let mut process = alive_process();
    let before = process.usage();
    assert_eq!(
      process.charge(ResourceCharge {
        threads: 1,
        pages: 5,
        capability_slots: 1,
        ipc_queue_capacity: 1,
      }),
      Err(Error::ResourceLimit)
    );
    assert_eq!(process.usage(), before);
  }

  #[test]
  fn limit_006_release_returns_quota_and_rejects_underflow() {
    let mut process = alive_process();
    let charge = ResourceCharge {
      threads: 1,
      pages: 2,
      capability_slots: 3,
      ipc_queue_capacity: 4,
    };
    process.charge(charge).unwrap();
    process.release(charge).unwrap();
    assert!(process.usage().is_empty());
    assert_eq!(process.release(charge), Err(Error::ResourceUnderflow));
  }

  #[test]
  fn limit_007_charge_preflight_does_not_mutate_usage() {
    let process = alive_process();
    let before = process.usage();
    assert_eq!(
      process.can_charge(ResourceCharge {
        threads: 1,
        ..ResourceCharge::default()
      }),
      Ok(())
    );
    assert_eq!(process.usage(), before);
  }

  #[test]
  fn proc_012_stopping_process_rejects_new_resource_charge() {
    let mut process = alive_process();
    process.begin_stop().unwrap();
    assert_eq!(
      process.charge(ResourceCharge {
        threads: 1,
        ..ResourceCharge::default()
      }),
      Err(Error::InvalidState)
    );
  }

  #[test]
  fn proc_013_live_resources_block_dead_transition() {
    let mut process = alive_process();
    let charge = ResourceCharge {
      threads: 1,
      ..ResourceCharge::default()
    };
    process.charge(charge).unwrap();
    process.begin_stop().unwrap();
    assert_eq!(process.finish_stop(), Err(Error::ResourcesStillLive));
    assert_eq!(process.state(), ProcessState::Stopping);
    process.release(charge).unwrap();
    process.finish_stop().unwrap();
    assert_eq!(process.state(), ProcessState::Dead);
  }

  #[test]
  fn proc_014_invalid_limits_are_rejected() {
    assert_eq!(ProcessLimits::new(0, 1, 1, 1), Err(Error::InvalidLimit));
    assert_eq!(
      ProcessLimits::new(MAX_THREADS_PER_PROCESS + 1, 1, 1, 1),
      Err(Error::InvalidLimit)
    );
    assert_eq!(
      ProcessLimits::new(1, 1, MAX_CAPABILITY_SLOTS as u16 + 1, 1),
      Err(Error::InvalidLimit)
    );
  }

  #[test]
  fn proc_070_alive_process_without_threads_replaces_address_space() {
    let replacement = ObjectId::from_raw_parts(2, 1);
    let mut process = alive_process();
    assert_eq!(
      process.replace_address_space(replacement),
      Ok(ADDRESS_SPACE)
    );
    assert_eq!(process.address_space(), replacement);
  }

  #[test]
  fn proc_071_process_with_thread_rejects_address_space_replacement() {
    let replacement = ObjectId::from_raw_parts(2, 1);
    let mut process = alive_process();
    process
      .charge(ResourceCharge {
        threads: 1,
        ..ResourceCharge::default()
      })
      .unwrap();
    assert_eq!(
      process.replace_address_space(replacement),
      Err(Error::InvalidState)
    );
    assert_eq!(process.address_space(), ADDRESS_SPACE);
  }

  #[test]
  fn proc_080_termination_reason_is_first_wins() {
    let mut process = alive_process();
    let first = process
      .begin_termination_once(TerminationReason::Exit { status: 7 })
      .unwrap();
    let follower = process
      .begin_termination_once(TerminationReason::UnknownSyscall { number: 99 })
      .unwrap();
    assert!(first.leader);
    assert!(!follower.leader);
    assert_eq!(follower.record, first.record);
    assert_eq!(process.state(), ProcessState::Stopping);
  }

  #[test]
  fn proc_081_termination_progress_reaches_quiescent() {
    let mut process = alive_process();
    process
      .begin_termination_once(TerminationReason::Exit { status: 0 })
      .unwrap();
    process.mark_termination_quiescent().unwrap();
    assert_eq!(
      process.termination().unwrap().progress(),
      TerminationProgress::Quiescent
    );
  }

  #[test]
  fn proc_082_quiescent_does_not_bypass_resource_reclamation() {
    let mut process = alive_process();
    process
      .charge(ResourceCharge {
        threads: 1,
        ..ResourceCharge::default()
      })
      .unwrap();
    process
      .begin_termination_once(TerminationReason::Exit { status: 0 })
      .unwrap();
    process.mark_termination_quiescent().unwrap();
    assert_eq!(process.finish_stop(), Err(Error::ResourcesStillLive));
    assert_eq!(process.state(), ProcessState::Stopping);
  }
}
