use crate::capability::{Capability, CapabilityHandle, CapabilityNodeId, Rights};
use crate::capability_space::{self, CapabilitySpaceId, CapabilitySpaceTable};
#[cfg(target_arch = "riscv64")]
use crate::memory::address_space::{AddressSpace, Asid};
use crate::object::{self, KernelObject, ObjectId, ObjectTable, ObjectType};
use crate::process::{Process, ProcessLimits, ProcessState, ResourceCharge};
use crate::scheduler::{self, SchedulerState, WakeResult};
use crate::thread::{self, Thread, ThreadState};

#[cfg(target_arch = "riscv64")]
use crate::sync::SpinLock;
#[cfg(target_arch = "riscv64")]
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_arch = "riscv64")]
static KERNEL_STATE: SpinLock<KernelState> = SpinLock::new(KernelState::new());
#[cfg(target_arch = "riscv64")]
static SCHEDULER_READY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  Object(object::Error),
  Capability(crate::capability::Error),
  CapabilitySpace(capability_space::Error),
  AddressSpace(crate::memory::address_space::Error),
  AlreadyInitialized,
  NodeExhausted,
  CommitInvariant,
  Scheduler(scheduler::Error),
  SchedulerUninitialized,
  Thread(thread::Error),
  ProcessNotAlive,
  Process(crate::process::Error),
  KernelStack(crate::memory::kernel_stack::Error),
  FrameAllocator(crate::memory::frame_allocator::Error),
  Hart(crate::arch::riscv64::hart::Error),
  KernelAddressSpaceUninitialized,
  RollbackFailed,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Object(_) => "kernel object operation failed",
      Self::Capability(_) => "capability operation failed",
      Self::CapabilitySpace(_) => "capability space operation failed",
      Self::AddressSpace(_) => "bootstrap address space is invalid",
      Self::AlreadyInitialized => "bootstrap kernel state is already initialized",
      Self::NodeExhausted => "capability node identity is exhausted",
      Self::CommitInvariant => "capability transaction commit invariant failed",
      Self::Scheduler(_) => "scheduler operation failed",
      Self::SchedulerUninitialized => "scheduler is not initialized",
      Self::Thread(_) => "thread state transition failed",
      Self::ProcessNotAlive => "owner process is not alive",
      Self::Process(_) => "process resource accounting failed",
      Self::KernelStack(_) => "kernel stack operation failed",
      Self::FrameAllocator(_) => "runtime frame allocation failed",
      Self::Hart(_) => "hart-local thread operation failed",
      Self::KernelAddressSpaceUninitialized => "kernel Address Space is not initialized",
      Self::RollbackFailed => "kernel state transaction rollback failed",
    }
  }
}

impl From<object::Error> for Error {
  fn from(error: object::Error) -> Self {
    Self::Object(error)
  }
}

impl From<crate::capability::Error> for Error {
  fn from(error: crate::capability::Error) -> Self {
    Self::Capability(error)
  }
}

impl From<capability_space::Error> for Error {
  fn from(error: capability_space::Error) -> Self {
    Self::CapabilitySpace(error)
  }
}

impl From<scheduler::Error> for Error {
  fn from(error: scheduler::Error) -> Self {
    Self::Scheduler(error)
  }
}

impl From<thread::Error> for Error {
  fn from(error: thread::Error) -> Self {
    Self::Thread(error)
  }
}

impl From<crate::memory::frame_allocator::Error> for Error {
  fn from(error: crate::memory::frame_allocator::Error) -> Self {
    Self::FrameAllocator(error)
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapProcess {
  pub object: ObjectId,
  pub authority: CapabilityHandle,
  pub space: CapabilitySpaceId,
}

pub struct KernelState {
  bootstrap_process: Option<BootstrapProcess>,
  objects: ObjectTable,
  spaces: CapabilitySpaceTable,
  next_capability_node: u64,
  kernel_address_space: Option<CapabilityHandle>,
  scheduler: SchedulerState,
}

#[derive(Clone, Copy)]
struct ThreadSnapshot {
  process: ObjectId,
  state: ThreadState,
  affinity: u64,
  priority: u8,
  running_hart: Option<u16>,
  has_execution: bool,
}

impl KernelState {
  pub const fn new() -> Self {
    Self {
      bootstrap_process: None,
      objects: ObjectTable::new(),
      spaces: CapabilitySpaceTable::new(),
      next_capability_node: 1,
      kernel_address_space: None,
      scheduler: SchedulerState::uninitialized(),
    }
  }

  pub const fn bootstrap_space(&self) -> CapabilitySpaceId {
    self.spaces.bootstrap_id()
  }

  pub fn create_capability_space(&mut self) -> Result<CapabilitySpaceId, Error> {
    Ok(self.spaces.create()?)
  }

  fn next_node(&self) -> Result<CapabilityNodeId, Error> {
    CapabilityNodeId::new(self.next_capability_node).ok_or(Error::NodeExhausted)
  }

  fn commit_node(&mut self) -> Result<(), Error> {
    self.next_capability_node = self
      .next_capability_node
      .checked_add(1)
      .ok_or(Error::NodeExhausted)?;
    Ok(())
  }

  pub const fn kernel_address_space_handle(&self) -> Option<CapabilityHandle> {
    self.kernel_address_space
  }

  pub fn capability_count(&self) -> usize {
    match self.spaces.get(self.bootstrap_space()) {
      Ok(space) => space.live_count(),
      Err(_) => 0,
    }
  }

  pub const fn capability_space_count(&self) -> usize {
    self.spaces.live_count()
  }

  pub const fn object_count(&self) -> usize {
    self.objects.live_count()
  }

  pub fn create_object_with_capability(
    &mut self,
    object: KernelObject,
    rights: Rights,
  ) -> Result<CapabilityHandle, Error> {
    self.create_object_in_space(self.bootstrap_space(), object, rights)
  }

  pub fn create_object_in_space(
    &mut self,
    space: CapabilitySpaceId,
    object: KernelObject,
    rights: Rights,
  ) -> Result<CapabilityHandle, Error> {
    self.spaces.get(space)?;
    let node = self.next_node()?;
    self
      .next_capability_node
      .checked_add(1)
      .ok_or(Error::NodeExhausted)?;

    let object_type = object.object_type();
    let id = self.objects.insert(object)?;
    let handle = match self
      .spaces
      .get_mut(space)?
      .insert_initial(id, object_type, rights, node)
    {
      Ok(handle) => handle,
      Err(error) => {
        self
          .objects
          .rollback_unreferenced_insert(id)
          .map_err(|_| Error::RollbackFailed)?;
        return Err(Error::Capability(error));
      }
    };

    if self.objects.add_capability_reference(id).is_err() {
      let removed = self
        .spaces
        .get_mut(space)?
        .remove(handle)
        .map_err(|_| Error::RollbackFailed)?;
      if removed != id || self.objects.rollback_unreferenced_insert(id).is_err() {
        return Err(Error::RollbackFailed);
      }
      return Err(Error::Object(object::Error::ReferenceOverflow));
    }
    self.commit_node()?;
    Ok(handle)
  }

  pub fn derive_capability(
    &mut self,
    parent: CapabilityHandle,
    rights: Rights,
  ) -> Result<CapabilityHandle, Error> {
    let space = self.bootstrap_space();
    let node = self.next_node()?;
    self
      .next_capability_node
      .checked_add(1)
      .ok_or(Error::NodeExhausted)?;
    let parent_object = self.spaces.get(space)?.identity(parent)?.0;
    self.objects.can_add_capability_reference(parent_object)?;

    let (derived, object) = self
      .spaces
      .get_mut(space)?
      .derive_local(parent, rights, node)?;
    if self.objects.add_capability_reference(object).is_err() {
      self
        .spaces
        .get_mut(space)?
        .remove(derived)
        .map_err(|_| Error::RollbackFailed)?;
      return Err(Error::CommitInvariant);
    }
    self.commit_node()?;
    Ok(derived)
  }

  pub fn remove_capability(&mut self, handle: CapabilityHandle) -> Result<(), Error> {
    let bootstrap = self.bootstrap_space();
    let (object, _) = self.spaces.get(bootstrap)?.identity(handle)?;
    self.objects.get(object)?;
    if self.objects.capability_references(object)? == 0 {
      return Err(Error::Object(object::Error::ReferenceUnderflow));
    }
    let removed = self.spaces.get_mut(bootstrap)?.remove(handle)?;
    debug_assert_eq!(removed, object);
    self.objects.remove_capability_reference(object)?;
    Ok(())
  }

  pub fn share_capability(
    &mut self,
    source_space: CapabilitySpaceId,
    source_handle: CapabilityHandle,
    destination_space: CapabilitySpaceId,
    rights: Rights,
  ) -> Result<CapabilityHandle, Error> {
    let source = self.spaces.get(source_space)?.snapshot(source_handle)?;
    let node = self.next_node()?;
    self
      .next_capability_node
      .checked_add(1)
      .ok_or(Error::NodeExhausted)?;
    let derived = Capability::derived(source, rights, node)?;
    self.objects.get(source.object())?;
    self.objects.can_add_capability_reference(source.object())?;
    if !self.spaces.get(destination_space)?.has_free_slot() {
      return Err(Error::Capability(crate::capability::Error::SpaceFull));
    }

    let handle = self
      .spaces
      .get_mut(destination_space)?
      .insert_existing(derived)
      .map_err(|_| Error::CommitInvariant)?;
    self
      .objects
      .add_capability_reference(source.object())
      .map_err(|_| Error::CommitInvariant)?;
    self.commit_node()?;
    Ok(handle)
  }

  pub fn transfer_capability(
    &mut self,
    source_space: CapabilitySpaceId,
    source_handle: CapabilityHandle,
    destination_space: CapabilitySpaceId,
  ) -> Result<CapabilityHandle, Error> {
    if source_space == destination_space {
      return Err(Error::CapabilitySpace(capability_space::Error::SameSpace));
    }
    let source = self.spaces.get(source_space)?.snapshot(source_handle)?;
    if !source.rights().contains(Rights::TRANSFER) {
      return Err(Error::Capability(crate::capability::Error::RightsDenied));
    }
    self.objects.get(source.object())?;
    if !self.spaces.get(destination_space)?.has_free_slot() {
      return Err(Error::Capability(crate::capability::Error::SpaceFull));
    }

    let (source_caps, destination_caps) = self.spaces.pair_mut(source_space, destination_space)?;
    let moved = source_caps
      .remove_full(source_handle)
      .map_err(|_| Error::CommitInvariant)?;
    debug_assert_eq!(moved.node(), source.node());
    debug_assert_eq!(moved.parent(), source.parent());
    destination_caps
      .insert_existing(moved)
      .map_err(|_| Error::CommitInvariant)
  }

  pub fn lookup(
    &self,
    handle: CapabilityHandle,
    expected_type: ObjectType,
    required: Rights,
  ) -> Result<&KernelObject, Error> {
    self.lookup_in_space(self.bootstrap_space(), handle, expected_type, required)
  }

  pub fn lookup_in_space(
    &self,
    space: CapabilitySpaceId,
    handle: CapabilityHandle,
    expected_type: ObjectType,
    required: Rights,
  ) -> Result<&KernelObject, Error> {
    let object = self
      .spaces
      .get(space)?
      .lookup(handle, expected_type, required)?;
    self.objects.validate_type(object, expected_type)?;
    Ok(self.objects.get(object)?)
  }

  pub fn initialize_scheduler(&mut self, online_mask: u64) -> Result<(), Error> {
    if self.scheduler.is_initialized() {
      return Err(Error::AlreadyInitialized);
    }
    self.scheduler.initialize(online_mask)?;
    Ok(())
  }

  pub fn scheduler_summary(&self) -> Result<(u64, usize, usize), Error> {
    let scheduler = self.scheduler()?;
    Ok((
      scheduler.online_mask(),
      scheduler.total_runnable(),
      scheduler.total_running(),
    ))
  }

  fn scheduler(&self) -> Result<&SchedulerState, Error> {
    self
      .scheduler
      .is_initialized()
      .then_some(&self.scheduler)
      .ok_or(Error::SchedulerUninitialized)
  }

  fn scheduler_mut(&mut self) -> Result<&mut SchedulerState, Error> {
    if !self.scheduler.is_initialized() {
      return Err(Error::SchedulerUninitialized);
    }
    Ok(&mut self.scheduler)
  }

  fn thread_snapshot(&self, id: ObjectId) -> Result<ThreadSnapshot, Error> {
    let thread = self
      .objects
      .get(id)?
      .as_thread()
      .ok_or(Error::Object(object::Error::WrongObjectType))?;
    Ok(ThreadSnapshot {
      process: thread.process(),
      state: thread.state(),
      affinity: thread.affinity(),
      priority: thread.effective_priority(),
      running_hart: thread.running_hart(),
      has_execution: thread.has_execution(),
    })
  }

  fn require_process_alive(&self, id: ObjectId) -> Result<(), Error> {
    let process = self
      .objects
      .get(id)?
      .as_process()
      .ok_or(Error::Object(object::Error::WrongObjectType))?;
    if process.state() != ProcessState::Alive {
      return Err(Error::ProcessNotAlive);
    }
    Ok(())
  }

  pub fn wake_thread(&mut self, thread_id: ObjectId) -> Result<WakeResult, Error> {
    let snapshot = self.thread_snapshot(thread_id)?;
    self.require_process_alive(snapshot.process)?;
    if !snapshot.has_execution {
      return Err(Error::Thread(thread::Error::ExecutionMissing));
    }
    if !matches!(snapshot.state, ThreadState::New | ThreadState::Blocked) {
      return Err(Error::Thread(thread::Error::InvalidState));
    }

    let result = self
      .scheduler_mut()?
      .enqueue(thread_id, snapshot.affinity, snapshot.priority)?;
    self
      .objects
      .get_mut(thread_id)?
      .as_thread_mut()
      .ok_or(Error::CommitInvariant)?
      .make_runnable()
      .map_err(|_| Error::CommitInvariant)?;
    Ok(result)
  }

  pub fn dispatch_next(&mut self, hart: usize) -> Result<Option<ObjectId>, Error> {
    let candidate = self.scheduler()?.peek_next(hart)?;
    let Some(thread_id) = candidate else {
      return Ok(None);
    };
    let snapshot = self.thread_snapshot(thread_id)?;
    let hart_bit = 1u64
      .checked_shl(hart as u32)
      .ok_or(Error::CommitInvariant)?;
    if snapshot.state != ThreadState::Runnable || snapshot.affinity & hart_bit == 0 {
      return Err(Error::CommitInvariant);
    }

    self.scheduler_mut()?.dispatch_expected(hart, thread_id)?;
    self
      .objects
      .get_mut(thread_id)?
      .as_thread_mut()
      .ok_or(Error::CommitInvariant)?
      .begin_running(hart)
      .map_err(|_| Error::CommitInvariant)?;
    Ok(Some(thread_id))
  }

  pub fn preempt_current(&mut self, hart: usize) -> Result<WakeResult, Error> {
    let thread_id = self
      .scheduler()?
      .running_thread(hart)?
      .ok_or(Error::Scheduler(scheduler::Error::NoRunningThread))?;
    let snapshot = self.thread_snapshot(thread_id)?;
    if snapshot.state != ThreadState::Running || snapshot.running_hart != u16::try_from(hart).ok() {
      return Err(Error::CommitInvariant);
    }
    let result = self
      .scheduler_mut()?
      .preempt(hart, snapshot.affinity, snapshot.priority)?;
    self
      .objects
      .get_mut(thread_id)?
      .as_thread_mut()
      .ok_or(Error::CommitInvariant)?
      .preempt()
      .map_err(|_| Error::CommitInvariant)?;
    Ok(result)
  }

  pub fn block_current(&mut self, hart: usize) -> Result<ObjectId, Error> {
    let thread_id = self
      .scheduler()?
      .running_thread(hart)?
      .ok_or(Error::Scheduler(scheduler::Error::NoRunningThread))?;
    let snapshot = self.thread_snapshot(thread_id)?;
    if snapshot.state != ThreadState::Running || snapshot.running_hart != u16::try_from(hart).ok() {
      return Err(Error::CommitInvariant);
    }
    let removed = self.scheduler_mut()?.block_running(hart)?;
    debug_assert_eq!(removed, thread_id);
    self
      .objects
      .get_mut(thread_id)?
      .as_thread_mut()
      .ok_or(Error::CommitInvariant)?
      .block()
      .map_err(|_| Error::CommitInvariant)?;
    Ok(thread_id)
  }

  pub fn create_thread_in_space(
    &mut self,
    authority_space: CapabilitySpaceId,
    process_handle: CapabilityHandle,
    affinity: u64,
    priority: u8,
    entry: usize,
    stack: crate::memory::kernel_stack::KernelStack,
  ) -> Result<CreatedThread, CreateThreadFailure> {
    match self.create_thread_inner(
      authority_space,
      process_handle,
      affinity,
      priority,
      entry,
      stack,
    ) {
      Ok(created) => Ok(created),
      Err((error, stack)) => Err(CreateThreadFailure { error, stack }),
    }
  }

  pub fn create_memory_in_space(
    &mut self,
    authority_space: CapabilitySpaceId,
    process_handle: CapabilityHandle,
    frame: crate::memory::frame_allocator::FrameBlock,
  ) -> Result<CreatedMemory, CreateMemoryFailure> {
    match self.create_memory_inner(authority_space, process_handle, frame) {
      Ok(created) => Ok(created),
      Err((error, frame)) => Err(CreateMemoryFailure { error, frame }),
    }
  }

  fn create_memory_inner(
    &mut self,
    authority_space: CapabilitySpaceId,
    process_handle: CapabilityHandle,
    frame: crate::memory::frame_allocator::FrameBlock,
  ) -> Result<CreatedMemory, (Error, crate::memory::frame_allocator::FrameBlock)> {
    let authority = match self.spaces.get(authority_space) {
      Ok(space) => space,
      Err(error) => return Err((Error::CapabilitySpace(error), frame)),
    };
    let process_id = match authority.lookup(process_handle, ObjectType::Process, Rights::CREATE) {
      Ok(id) => id,
      Err(error) => return Err((Error::Capability(error), frame)),
    };
    let process = match self.objects.get(process_id) {
      Ok(KernelObject::Process(process)) => process,
      Ok(_) => return Err((Error::Object(object::Error::WrongObjectType), frame)),
      Err(error) => return Err((Error::Object(error), frame)),
    };
    if process.state() != ProcessState::Alive {
      return Err((Error::ProcessNotAlive, frame));
    }
    if let Err(error) = process.can_charge(MEMORY_PAGE_CHARGE) {
      return Err((Error::Process(error), frame));
    }
    let owner_space = process.capability_space();

    if !self.objects.has_free_slot() {
      return Err((Error::Object(object::Error::TableFull), frame));
    }
    let owner_capabilities = match self.spaces.get(owner_space) {
      Ok(space) => space,
      Err(error) => return Err((Error::CapabilitySpace(error), frame)),
    };
    if !owner_capabilities.has_free_slot() {
      return Err((
        Error::Capability(crate::capability::Error::SpaceFull),
        frame,
      ));
    }
    let node = match self.next_node() {
      Ok(node) => node,
      Err(error) => return Err((error, frame)),
    };
    if self.next_capability_node.checked_add(1).is_none() {
      return Err((Error::NodeExhausted, frame));
    }
    if frame.start() == 0
      || !frame.start().is_multiple_of(crate::memory::PAGE_SIZE)
      || frame.frames() != 1
    {
      return Err((Error::Object(object::Error::InvalidMemoryFrame), frame));
    }
    let memory = crate::object::MemoryObject::new(frame)
      .unwrap_or_else(|_| panic!("preflighted runtime frame block became invalid"));

    self
      .objects
      .get_mut(process_id)
      .unwrap_or_else(|_| panic!("preflighted owner process disappeared"))
      .as_process_mut()
      .unwrap_or_else(|| panic!("preflighted owner object changed type"))
      .charge(MEMORY_PAGE_CHARGE)
      .unwrap_or_else(|_| panic!("preflighted page quota commit failed"));
    let memory_id = self
      .objects
      .insert(KernelObject::Memory(memory))
      .unwrap_or_else(|_| panic!("preflighted Memory Object insertion failed"));
    let handle = self
      .spaces
      .get_mut(owner_space)
      .unwrap_or_else(|_| panic!("preflighted owner Capability Space disappeared"))
      .insert_initial(memory_id, ObjectType::Memory, MEMORY_OWNER_RIGHTS, node)
      .unwrap_or_else(|_| panic!("preflighted Memory capability insertion failed"));
    self
      .objects
      .add_capability_reference(memory_id)
      .unwrap_or_else(|_| panic!("new Memory Object reference increment failed"));
    self
      .commit_node()
      .unwrap_or_else(|_| panic!("preflighted node commit failed"));

    Ok(CreatedMemory {
      object: memory_id,
      capability: handle,
    })
  }

  fn create_thread_inner(
    &mut self,
    authority_space: CapabilitySpaceId,
    process_handle: CapabilityHandle,
    affinity: u64,
    priority: u8,
    entry: usize,
    stack: crate::memory::kernel_stack::KernelStack,
  ) -> Result<CreatedThread, (Error, crate::memory::kernel_stack::KernelStack)> {
    let authority = match self.spaces.get(authority_space) {
      Ok(space) => space,
      Err(error) => return Err((Error::CapabilitySpace(error), stack)),
    };
    let process_id = match authority.lookup(process_handle, ObjectType::Process, Rights::CREATE) {
      Ok(id) => id,
      Err(error) => return Err((Error::Capability(error), stack)),
    };

    let process = match self.objects.get(process_id) {
      Ok(KernelObject::Process(process)) => process,
      Ok(_) => return Err((Error::Object(object::Error::WrongObjectType), stack)),
      Err(error) => return Err((Error::Object(error), stack)),
    };
    if process.state() != ProcessState::Alive {
      return Err((Error::ProcessNotAlive, stack));
    }
    if let Err(error) = process.can_charge(THREAD_CHARGE) {
      return Err((Error::Process(error), stack));
    }
    let owner_space = process.capability_space();

    if !self.objects.has_free_slot() {
      return Err((Error::Object(object::Error::TableFull), stack));
    }
    let owner_capabilities = match self.spaces.get(owner_space) {
      Ok(space) => space,
      Err(error) => return Err((Error::CapabilitySpace(error), stack)),
    };
    if !owner_capabilities.has_free_slot() {
      return Err((
        Error::Capability(crate::capability::Error::SpaceFull),
        stack,
      ));
    }
    let node = match self.next_node() {
      Ok(node) => node,
      Err(error) => return Err((error, stack)),
    };
    if self.next_capability_node.checked_add(1).is_none() {
      return Err((Error::NodeExhausted, stack));
    }

    let mut thread = match Thread::new(process_id, affinity, priority) {
      Ok(thread) => thread,
      Err(error) => return Err((Error::Thread(error), stack)),
    };
    if let Err(failuer) = thread.install_execution(stack, entry) {
      let (error, stack) = failuer.into_parts();
      return Err((Error::Thread(error), stack));
    }

    self
      .objects
      .get_mut(process_id)
      .unwrap_or_else(|_| panic!("preflighted owner process disappered"))
      .as_process_mut()
      .unwrap_or_else(|| panic!("preflighted owner object changed type"))
      .charge(THREAD_CHARGE)
      .unwrap_or_else(|_| panic!("preflighted thread quota commit failed"));

    let thread_id = self
      .objects
      .insert(KernelObject::Thread(thread))
      .unwrap_or_else(|_| panic!("preflighted thread object insertion failed"));
    let handle = self
      .spaces
      .get_mut(owner_space)
      .unwrap_or_else(|_| panic!("preflighted owner capability space disappeared"))
      .insert_initial(thread_id, ObjectType::Thread, THREAD_OWNER_RIGHTS, node)
      .unwrap_or_else(|_| panic!("preflighted thread capability insertion failed"));
    self
      .objects
      .add_capability_reference(thread_id)
      .unwrap_or_else(|_| panic!("new thread reference increment failed"));
    self
      .commit_node()
      .unwrap_or_else(|_| panic!("preflighted node commit failed"));

    Ok(CreatedThread {
      object: thread_id,
      capability: handle,
    })
  }

  fn thread_context_pointer(
    &mut self,
    thread_id: ObjectId,
  ) -> Result<*mut crate::arch::riscv64::context::SavedContext, Error> {
    let thread = self
      .objects
      .get_mut(thread_id)?
      .as_thread_mut()
      .ok_or(Error::Object(object::Error::WrongObjectType))?;
    let execution = thread
      .execution_mut()
      .ok_or(Error::Thread(thread::Error::ExecutionMissing))?;
    Ok(execution.context_mut() as *mut _)
  }

  fn create_bootstrap_process(&mut self) -> Result<BootstrapProcess, Error> {
    if self.bootstrap_process.is_some() {
      return Err(Error::AlreadyInitialized);
    }
    let address_space_handle = self
      .kernel_address_space
      .ok_or(Error::KernelAddressSpaceUninitialized)?;
    let address_space = self.spaces.get(self.bootstrap_space())?.lookup(
      address_space_handle,
      ObjectType::AddressSpace,
      Rights::MAP,
    )?;
    let process_space = self.create_capability_space()?;
    let limits = ProcessLimits::new(4, 16, 16, 16).map_err(Error::Process)?;
    let mut process = Process::new(address_space, process_space, limits);
    process.start().map_err(Error::Process)?;
    let authority = self
      .create_object_with_capability(KernelObject::Process(process), BOOTSTRAP_PROCESS_RIGHTS)?;
    let object = self.spaces.get(self.bootstrap_space())?.lookup(
      authority,
      ObjectType::Process,
      Rights::CREATE,
    )?;
    let created = BootstrapProcess {
      object,
      authority,
      space: process_space,
    };
    self.bootstrap_process = Some(created);
    Ok(created)
  }

  #[cfg(target_arch = "riscv64")]
  fn prepare_current_hart_dispatch(
    &mut self,
    hart: usize,
  ) -> Result<Option<(ObjectId, *const crate::arch::riscv64::context::SavedContext)>, Error> {
    let local = crate::arch::riscv64::hart::local_by_index(hart).ok_or(Error::Hart(
      crate::arch::riscv64::hart::Error::InvalidHartIndex,
    ))?;
    if local.current_thread().is_some() {
      return Err(Error::Hart(
        crate::arch::riscv64::hart::Error::CurrentThreadBusy,
      ));
    }
    let Some(thread_id) = self.dispatch_next(hart)? else {
      return Ok(None);
    };
    local
      .install_current_thread(thread_id)
      .unwrap_or_else(|_| panic!("preflighted current Thread publication failed"));
    let context = self
      .thread_context_pointer(thread_id)
      .unwrap_or_else(|_| panic!("dispatched Thread lost its execution context"));
    Ok(Some((thread_id, context.cast_const())))
  }

  #[cfg(target_arch = "riscv64")]
  fn prepare_current_thread_park(
    &mut self,
    hart: usize,
    thread_id: ObjectId,
  ) -> Result<*mut crate::arch::riscv64::context::SavedContext, Error> {
    if self.block_current(hart)? != thread_id {
      return Err(Error::CommitInvariant);
    }
    let local = crate::arch::riscv64::hart::local_by_index(hart).ok_or(Error::Hart(
      crate::arch::riscv64::hart::Error::InvalidHartIndex,
    ))?;
    local
      .clear_current_thread(thread_id)
      .unwrap_or_else(|_| panic!("running Thread identity changed during park"));
    Ok(
      self
        .thread_context_pointer(thread_id)
        .unwrap_or_else(|_| panic!("parked Thread lost its execution context")),
    )
  }

  #[cfg(target_arch = "riscv64")]
  fn prepare_current_thread_yield(
    &mut self,
    hart: usize,
    thread_id: ObjectId,
  ) -> Result<*mut crate::arch::riscv64::context::SavedContext, Error> {
    let local = crate::arch::riscv64::hart::local_by_index(hart).ok_or(Error::Hart(
      crate::arch::riscv64::hart::Error::InvalidHartIndex,
    ))?;
    if local.current_thread() != Some(thread_id) {
      return Err(Error::Hart(
        crate::arch::riscv64::hart::Error::CurrentThreadMismatch,
      ));
    }
    if local.pending_yield().is_some() {
      return Err(Error::Hart(
        crate::arch::riscv64::hart::Error::PendingYieldBusy,
      ));
    }
    let snapshot = self.thread_snapshot(thread_id)?;
    if snapshot.state != ThreadState::Running || snapshot.running_hart != u16::try_from(hart).ok() {
      return Err(Error::CommitInvariant);
    }
    if self.scheduler()?.running_thread(hart)? != Some(thread_id) {
      return Err(Error::CommitInvariant);
    }
    let outgoing = self.thread_context_pointer(thread_id)?;

    local
      .install_pending_yield(thread_id)
      .unwrap_or_else(|_| panic!("preflighted pending yield publication failed"));
    self
      .scheduler_mut()
      .unwrap_or_else(|_| panic!("initialized scheduler disappeared"))
      .suspend_running(hart, thread_id)
      .unwrap_or_else(|_| panic!("preflighted scheduler suspend failed"));
    self
      .objects
      .get_mut(thread_id)
      .unwrap_or_else(|_| panic!("yielding Thread disappeared"))
      .as_thread_mut()
      .unwrap_or_else(|| panic!("yielding Object changed type"))
      .begin_switch_out()
      .unwrap_or_else(|_| panic!("running Thread refused switch-out"));
    local
      .clear_current_thread(thread_id)
      .unwrap_or_else(|_| panic!("current Thread changed during yield"));
    Ok(outgoing)
  }

  #[cfg(target_arch = "riscv64")]
  fn prepare_timer_preemption(
    &mut self,
    hart: usize,
    thread_id: ObjectId,
    trap_frame: usize,
  ) -> Result<(), Error> {
    let local = crate::arch::riscv64::hart::local_by_index(hart).ok_or(Error::Hart(
      crate::arch::riscv64::hart::Error::InvalidHartIndex,
    ))?;
    if local.current_thread() != Some(thread_id) || local.pending_yield().is_some() {
      return Err(Error::CommitInvariant);
    }
    let snapshot = self.thread_snapshot(thread_id)?;
    if snapshot.state != ThreadState::Running || snapshot.running_hart != u16::try_from(hart).ok() {
      return Err(Error::CommitInvariant);
    }
    if self.scheduler()?.running_thread(hart)? != Some(thread_id) {
      return Err(Error::CommitInvariant);
    }

    let trap_context = crate::arch::riscv64::context::SavedContext::trap_resume(trap_frame)
      .map_err(thread::Error::Context)?;

    local
      .install_pending_yield(thread_id)
      .unwrap_or_else(|_| panic!("preflighted timer pending yield failed"));
    self
      .scheduler_mut()
      .unwrap_or_else(|_| panic!("initialized scheduler disappeared"))
      .suspend_running(hart, thread_id)
      .unwrap_or_else(|_| panic!("preflighted timer scheduler suspend failed"));
    let thread = self
      .objects
      .get_mut(thread_id)
      .unwrap_or_else(|_| panic!("preempted Thread disappeared"))
      .as_thread_mut()
      .unwrap_or_else(|| panic!("preempted Object changed type"));
    thread
      .begin_switch_out()
      .unwrap_or_else(|_| panic!("running Thread refused timer switch-out"));
    *thread
      .execution_mut()
      .unwrap_or_else(|| panic!("preempted Thread lost execution"))
      .context_mut() = trap_context;
    local
      .clear_current_thread(thread_id)
      .unwrap_or_else(|_| panic!("current Thread changed during timer preemption"));
    Ok(())
  }

  #[cfg(target_arch = "riscv64")]
  fn finalize_current_hart_yield(&mut self, hart: usize) -> Result<Option<ObjectId>, Error> {
    let local = crate::arch::riscv64::hart::local_by_index(hart).ok_or(Error::Hart(
      crate::arch::riscv64::hart::Error::InvalidHartIndex,
    ))?;
    let Some(thread_id) = local.pending_yield() else {
      return Ok(None);
    };
    let snapshot = self.thread_snapshot(thread_id)?;
    if snapshot.state != ThreadState::Switching || snapshot.running_hart.is_some() {
      return Err(Error::CommitInvariant);
    }
    let hart_bit = 1u64
      .checked_shl(hart as u32)
      .ok_or(Error::CommitInvariant)?;
    if snapshot.affinity & hart_bit == 0 {
      return Err(Error::CommitInvariant);
    }
    self.objects.get(thread_id)?;

    self
      .scheduler_mut()
      .unwrap_or_else(|_| panic!("initialized scheduler disappeared"))
      .resume_suspended(hart, thread_id, snapshot.priority)
      .unwrap_or_else(|_| panic!("saved Thread could not be republished"));
    self
      .objects
      .get_mut(thread_id)
      .unwrap_or_else(|_| panic!("switching Thread disappeared"))
      .as_thread_mut()
      .unwrap_or_else(|| panic!("switching Object changed type"))
      .finish_yield()
      .unwrap_or_else(|_| panic!("switching Thread refused yield completion"));
    local
      .clear_pending_yield(thread_id)
      .unwrap_or_else(|_| panic!("pending yield identity changed during finalize"));
    Ok(Some(thread_id))
  }

  #[cfg(test)]
  fn insert_test_object(&mut self, object: KernelObject) -> Result<ObjectId, Error> {
    Ok(self.objects.insert(object)?)
  }
}

impl Default for KernelState {
  fn default() -> Self {
    Self::new()
  }
}

const THREAD_OWNER_RIGHTS: Rights = Rights::READ
  .union(Rights::WRITE)
  .union(Rights::TRANSFER)
  .union(Rights::GRANT)
  .union(Rights::REVOKE)
  .union(Rights::DESTROY);

const MEMORY_OWNER_RIGHTS: Rights = Rights::READ
  .union(Rights::WRITE)
  .union(Rights::EXECUTE)
  .union(Rights::MAP)
  .union(Rights::SHARE)
  .union(Rights::TRANSFER)
  .union(Rights::GRANT)
  .union(Rights::REVOKE)
  .union(Rights::DESTROY);

const BOOTSTRAP_PROCESS_RIGHTS: Rights = Rights::READ
  .union(Rights::WRITE)
  .union(Rights::TRANSFER)
  .union(Rights::GRANT)
  .union(Rights::REVOKE)
  .union(Rights::CREATE)
  .union(Rights::DESTROY);

const THREAD_CHARGE: ResourceCharge = ResourceCharge {
  threads: 1,
  pages: 0,
  capability_slots: 1,
  ipc_queue_capacity: 0,
};

const MEMORY_PAGE_CHARGE: ResourceCharge = ResourceCharge {
  threads: 0,
  pages: 1,
  capability_slots: 1,
  ipc_queue_capacity: 0,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreatedMemory {
  pub object: ObjectId,
  pub capability: CapabilityHandle,
}

pub struct CreateMemoryFailure {
  error: Error,
  frame: crate::memory::frame_allocator::FrameBlock,
}

impl CreateMemoryFailure {
  pub const fn error(&self) -> Error {
    self.error
  }

  pub fn into_parts(self) -> (Error, crate::memory::frame_allocator::FrameBlock) {
    (self.error, self.frame)
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreatedThread {
  pub object: ObjectId,
  pub capability: CapabilityHandle,
}

pub struct CreateThreadFailure {
  error: Error,
  stack: crate::memory::kernel_stack::KernelStack,
}

impl CreateThreadFailure {
  pub const fn error(&self) -> Error {
    self.error
  }

  pub fn into_parts(self) -> (Error, crate::memory::kernel_stack::KernelStack) {
    (self.error, self.stack)
  }
}

#[cfg(target_arch = "riscv64")]
pub fn initialize_kernel_address_space(root_frame: u64) -> Result<CapabilityHandle, Error> {
  let mut state = KERNEL_STATE.lock();
  if state.kernel_address_space.is_some() {
    return Err(Error::AlreadyInitialized);
  }

  let address_space = AddressSpace::new(root_frame, Asid::kernel()).map_err(Error::AddressSpace)?;
  let rights = Rights::MAP.union(Rights::SHARE);
  let handle =
    state.create_object_with_capability(KernelObject::AddressSpace(address_space), rights)?;
  state.kernel_address_space = Some(handle);
  Ok(handle)
}

#[cfg(target_arch = "riscv64")]
pub fn counts() -> (usize, usize, usize) {
  let state = KERNEL_STATE.lock();
  (
    state.object_count(),
    state.capability_count(),
    state.capability_space_count(),
  )
}

#[cfg(target_arch = "riscv64")]
pub fn initialize_scheduler_state(online_mask: u64) -> Result<(u64, usize, usize), Error> {
  let summary = {
    let mut state = KERNEL_STATE.lock();
    state.initialize_scheduler(online_mask)?;
    state
      .scheduler_summary()
      .unwrap_or_else(|_| panic!("initialized scheduler has no summary"))
  };
  publish_scheduler_ready();

  let current = crate::arch::riscv64::hart::current_index();
  for target in 0..crate::platform::MAX_HARTS {
    let Some(bit) = 1u64.checked_shl(target as u32) else {
      continue;
    };
    if online_mask & bit == 0 || target == current {
      continue;
    }
    crate::arch::riscv64::ipi::send(target, crate::arch::riscv64::ipi::Request::Reschedule)
      .unwrap_or_else(|_| panic!("failed to wake scheduler hart"));
  }
  Ok(summary)
}

#[cfg(target_arch = "riscv64")]
pub(crate) fn scheduler_ready() -> bool {
  SCHEDULER_READY.load(Ordering::Acquire)
}

#[cfg(target_arch = "riscv64")]
fn publish_scheduler_ready() {
  if SCHEDULER_READY.swap(true, Ordering::Release) {
    panic!("scheduler readiness was published twice");
  }
}

#[cfg(target_arch = "riscv64")]
pub fn create_thread(
  authority_space: CapabilitySpaceId,
  process_handle: CapabilityHandle,
  affinity: u64,
  priority: u8,
  entry: usize,
) -> Result<CreatedThread, Error> {
  let stack = crate::memory::kernel_stack::KernelStack::allocate().map_err(Error::KernelStack)?;
  let result = {
    let mut state = KERNEL_STATE.lock();
    state.create_thread_in_space(
      authority_space,
      process_handle,
      affinity,
      priority,
      entry,
      stack,
    )
  };
  match result {
    Ok(created) => Ok(created),
    Err(failure) => {
      let (error, stack) = failure.into_parts();
      stack
        .release()
        .unwrap_or_else(|_| panic!("failed Thread creation stack release"));
      Err(error)
    }
  }
}

#[cfg(target_arch = "riscv64")]
pub fn create_memory(
  authority_space: CapabilitySpaceId,
  process_handle: CapabilityHandle,
) -> Result<CreatedMemory, Error> {
  let frame = crate::memory::frame_allocator::allocate(1)?;
  let result = {
    let mut state = KERNEL_STATE.lock();
    state.create_memory_in_space(authority_space, process_handle, frame)
  };
  match result {
    Ok(created) => Ok(created),
    Err(failure) => {
      let (error, frame) = failure.into_parts();
      crate::memory::frame_allocator::release(frame)
        .unwrap_or_else(|_| panic!("failed Memory creation frame release"));
      Err(error)
    }
  }
}

#[cfg(target_arch = "riscv64")]
pub fn initialize_bootstrap_process() -> Result<BootstrapProcess, Error> {
  KERNEL_STATE.lock().create_bootstrap_process()
}

#[cfg(target_arch = "riscv64")]
pub fn bootstrap_space() -> CapabilitySpaceId {
  KERNEL_STATE.lock().bootstrap_space()
}

#[cfg(target_arch = "riscv64")]
pub fn wake_thread(thread_id: ObjectId) -> Result<WakeResult, Error> {
  let result = {
    let mut state = KERNEL_STATE.lock();
    state.wake_thread(thread_id)?
  };
  if result.request_reschedule {
    let current = crate::arch::riscv64::hart::current_index();
    if result.target_hart == current {
      crate::arch::riscv64::hart::current().request_reschedule();
    } else {
      crate::arch::riscv64::ipi::send(
        result.target_hart,
        crate::arch::riscv64::ipi::Request::Reschedule,
      )
      .unwrap_or_else(|_| panic!("failed to notify woken Thread target"));
    }
  }
  Ok(result)
}

#[cfg(target_arch = "riscv64")]
pub fn global_scheduler_summary() -> Result<(u64, usize, usize), Error> {
  KERNEL_STATE.lock().scheduler_summary()
}

#[cfg(target_arch = "riscv64")]
pub(crate) fn try_preempt_current_thread(trap_frame: *mut crate::arch::riscv64::trap::TrapFrame) {
  let hart = crate::arch::riscv64::hart::current_index();
  let local = crate::arch::riscv64::hart::current();
  if trap_frame.is_null() {
    return;
  }
  if !local.reschedule_requested() {
    return;
  }
  let Some(thread_id) = local.current_thread() else {
    return;
  };
  if !local.can_preempt() {
    return;
  }
  let Some(mut state) = KERNEL_STATE.try_lock() else {
    return;
  };
  state
    .prepare_timer_preemption(hart, thread_id, trap_frame as usize)
    .unwrap_or_else(|_| panic!("failed timer preemption transaction"));
  if !local.take_reschedule_request() {
    panic!("timer preemption lost reschedule request");
  }
  drop(state);

  // SAFETY: Thread state and ownership were unpublished, and its TrapFrame is live.
  unsafe { crate::arch::riscv64::context::restore_scheduler_context(hart) }
}

#[cfg(target_arch = "riscv64")]
pub fn dispatch_current_hart_once() -> Result<Option<ObjectId>, Error> {
  let interrupt_window = crate::arch::riscv64::hart::ThreadInterruptWindow::mask_scheduler();
  let hart = crate::arch::riscv64::hart::current_index();
  let scheduler =
    crate::arch::riscv64::context::scheduler_context(hart).map_err(thread::Error::Context)?;
  let prepared = {
    let mut state = KERNEL_STATE.lock();
    state.prepare_current_hart_dispatch(hart)?
  };
  let Some((thread_id, incoming)) = prepared else {
    return Ok(None);
  };

  crate::arch::riscv64::timer::begin_quantum()
    .unwrap_or_else(|_| panic!("failed to arm Thread scheduling quantum"));
  // SAFETY: publication is complete, both contexts are live, and the state lock is dropped.
  unsafe { crate::arch::riscv64::context::switch_to_thread(scheduler, incoming) };
  interrupt_window.returned_to_scheduler();

  {
    let mut state = KERNEL_STATE.lock();
    state.finalize_current_hart_yield(hart)?;
  }

  let local = crate::arch::riscv64::hart::local_by_index(hart).ok_or(Error::Hart(
    crate::arch::riscv64::hart::Error::InvalidHartIndex,
  ))?;
  if local.current_thread().is_some() {
    return Err(Error::CommitInvariant);
  }
  Ok(Some(thread_id))
}

#[cfg(target_arch = "riscv64")]
pub fn run_scheduler_loop() -> ! {
  while !scheduler_ready() {
    crate::support::cpu::wait_for_interrupt();
  }

  loop {
    crate::arch::riscv64::hart::current().take_reschedule_request();
    match dispatch_current_hart_once() {
      Ok(Some(_)) => {}
      Ok(None) => crate::support::cpu::wait_for_interrupt(),
      Err(_) => panic!("per-hart scheduler loop failed"),
    }
  }
}

#[cfg(target_arch = "riscv64")]
pub fn park_current_thread() -> ! {
  let hart = crate::arch::riscv64::hart::current_index();
  let local = crate::arch::riscv64::hart::local_by_index(hart)
    .unwrap_or_else(|| panic!("current hart index is invalid"));
  let thread_id = local
    .current_thread()
    .unwrap_or_else(|| panic!("kernel Thread has no current identity"));
  let outgoing = {
    let mut state = KERNEL_STATE.lock();
    state
      .prepare_current_thread_park(hart, thread_id)
      .unwrap_or_else(|_| panic!("failed to park current kernel Thread"))
  };
  let scheduler = crate::arch::riscv64::context::scheduler_context(hart)
    .unwrap_or_else(|_| panic!("scheduler context is unavailable"));

  // SAFETY: the outgoing Thread is Blocked and unpublished before the state lock is dropped.
  unsafe { crate::arch::riscv64::context::switch_saved_context(outgoing, scheduler.cast_const()) };
  panic!("parked kernel Thread resumed without wake")
}

#[cfg(target_arch = "riscv64")]
pub fn yield_current_thread() {
  let hart = crate::arch::riscv64::hart::current_index();
  let local = crate::arch::riscv64::hart::local_by_index(hart)
    .unwrap_or_else(|| panic!("current hart index is invalid"));
  let thread_id = local
    .current_thread()
    .unwrap_or_else(|| panic!("yield requires a current Thread"));
  let outgoing = {
    let mut state = KERNEL_STATE.lock();
    state
      .prepare_current_thread_yield(hart, thread_id)
      .unwrap_or_else(|_| panic!("failed to prepare current Thread yield"))
  };
  let scheduler = crate::arch::riscv64::context::scheduler_context(hart)
    .unwrap_or_else(|_| panic!("scheduler context is unavailable"));

  // SAFETY: the Thread is Switching, unpublished, and exclusively owns outgoing.
  unsafe { crate::arch::riscv64::context::switch_saved_context(outgoing, scheduler.cast_const()) };

  if local.current_thread() != Some(thread_id) {
    panic!("yielded Thread resumed without current identity");
  }
}

#[cfg(test)]
mod tests {
  use crate::memory::address_space::{AddressSpace, Asid};
  use crate::object::memory_object_for_test;
  use crate::process::{Process, ProcessLimits};
  use crate::thread::{Thread, ThreadState};

  use super::*;

  fn memory() -> KernelObject {
    KernelObject::Memory(memory_object_for_test(0x8080_0000))
  }

  fn initialize_thread_creation_state(
    state: &mut KernelState,
    max_threads: u16,
  ) -> (CapabilityHandle, CapabilitySpaceId) {
    let owner_space = state.create_capability_space().unwrap();
    let mut process = Process::new(
      ObjectId::from_raw_parts(80, 1),
      owner_space,
      ProcessLimits::new(max_threads, 4, 8, 8).unwrap(),
    );
    process.start().unwrap();
    let handle = state
      .create_object_with_capability(KernelObject::Process(process), Rights::CREATE)
      .unwrap();
    (handle, owner_space)
  }

  fn initialize_memory_creation_state(
    state: &mut KernelState,
    max_pages: u32,
    used_pages: u32,
    rights: Rights,
  ) -> (CapabilityHandle, CapabilitySpaceId) {
    let owner_space = state.create_capability_space().unwrap();
    let mut process = Process::new(
      ObjectId::from_raw_parts(81, 1),
      owner_space,
      ProcessLimits::new(4, max_pages, 8, 8).unwrap(),
    );
    process.start().unwrap();
    for _ in 0..used_pages {
      process
        .charge(ResourceCharge {
          pages: 1,
          ..ResourceCharge::default()
        })
        .unwrap();
    }
    let handle = state
      .create_object_with_capability(KernelObject::Process(process), rights)
      .unwrap();
    (handle, owner_space)
  }

  fn scheduler_state(state: &mut KernelState) -> (ObjectId, ObjectId) {
    state.initialize_scheduler(0b1111).unwrap();
    let mut process = Process::new(
      ObjectId::from_raw_parts(90, 1),
      state.bootstrap_space(),
      ProcessLimits::new(4, 4, 8, 8).unwrap(),
    );
    process.start().unwrap();
    let process = state
      .insert_test_object(KernelObject::Process(process))
      .unwrap();
    let mut thread = Thread::new(process, 0b0101, 8).unwrap();
    thread
      .install_execution(
        crate::memory::kernel_stack::kernel_stack_for_test(0x9100_0000),
        0x8020_0000,
      )
      .unwrap_or_else(|_| panic!("test execution installation failed"));
    let thread = state
      .insert_test_object(KernelObject::Thread(thread))
      .unwrap();
    (process, thread)
  }

  #[test]
  fn dispatch_000_bootstrap_process_uses_kernel_address_space() {
    let mut state = KernelState::new();
    let address_space = AddressSpace::new(0x8020_0000, Asid::kernel()).unwrap();
    let handle = state
      .create_object_with_capability(
        KernelObject::AddressSpace(address_space),
        Rights::MAP.union(Rights::SHARE),
      )
      .unwrap();
    state.kernel_address_space = Some(handle);

    let process = state.create_bootstrap_process().unwrap();
    assert!(matches!(
      state.objects.get(process.object),
      Ok(KernelObject::Process(process)) if process.state() == ProcessState::Alive
    ));
    assert_eq!(
      state.create_bootstrap_process(),
      Err(Error::AlreadyInitialized)
    );
  }

  #[test]
  fn dispatch_001_context_pointer_requires_execution() {
    let mut state = KernelState::new();
    state.initialize_scheduler(0b1111).unwrap();
    let mut process = Process::new(
      ObjectId::from_raw_parts(90, 1),
      state.bootstrap_space(),
      ProcessLimits::new(4, 4, 8, 8).unwrap(),
    );
    process.start().unwrap();
    let process = state
      .insert_test_object(KernelObject::Process(process))
      .unwrap();
    let thread = state
      .insert_test_object(KernelObject::Thread(Thread::new(process, 1, 8).unwrap()))
      .unwrap();
    assert!(matches!(
      state.thread_context_pointer(thread),
      Err(Error::Thread(thread::Error::ExecutionMissing))
    ));
  }

  #[test]
  fn dispatch_002_dispatch_exposes_owned_context() {
    let mut state = KernelState::new();
    let (_, thread) = scheduler_state(&mut state);
    state.wake_thread(thread).unwrap();
    assert_eq!(state.dispatch_next(0), Ok(Some(thread)));
    assert!(!state.thread_context_pointer(thread).unwrap().is_null());
  }

  #[test]
  fn yield_001_switching_thread_is_absent_from_scheduler() {
    let mut state = KernelState::new();
    let (_, thread_id) = scheduler_state(&mut state);
    state.wake_thread(thread_id).unwrap();
    state.dispatch_next(0).unwrap();
    state.scheduler.suspend_running(0, thread_id).unwrap();
    state
      .objects
      .get_mut(thread_id)
      .unwrap()
      .as_thread_mut()
      .unwrap()
      .begin_switch_out()
      .unwrap();

    assert_eq!(state.scheduler.total_running(), 0);
    assert_eq!(state.scheduler.total_runnable(), 0);
    assert_eq!(
      state
        .objects
        .get(thread_id)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::Switching
    );
  }

  #[test]
  fn yield_002_finalize_republishes_saved_thread() {
    let mut state = KernelState::new();
    let (_, thread_id) = scheduler_state(&mut state);
    state.wake_thread(thread_id).unwrap();
    state.dispatch_next(0).unwrap();
    state.scheduler.suspend_running(0, thread_id).unwrap();
    let thread = state
      .objects
      .get_mut(thread_id)
      .unwrap()
      .as_thread_mut()
      .unwrap();
    let priority = thread.effective_priority();
    thread.begin_switch_out().unwrap();
    state
      .scheduler
      .resume_suspended(0, thread_id, priority)
      .unwrap();
    state
      .objects
      .get_mut(thread_id)
      .unwrap()
      .as_thread_mut()
      .unwrap()
      .finish_yield()
      .unwrap();

    assert_eq!(state.scheduler.queue_len(0), Ok(1));
    assert_eq!(
      state
        .objects
        .get(thread_id)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::Runnable
    );
  }

  #[test]
  fn kstate_001_create_publishes_object_and_capability_together() {
    let mut state = KernelState::new();
    let handle = state
      .create_object_with_capability(memory(), Rights::READ.union(Rights::SHARE))
      .unwrap();
    assert_eq!(state.object_count(), 1);
    assert_eq!(state.capability_count(), 1);
    assert!(matches!(
      state.lookup(handle, ObjectType::Memory, Rights::READ),
      Ok(KernelObject::Memory(_))
    ));
  }

  #[test]
  fn kstate_002_invalid_initial_rights_leave_no_partial_object() {
    let mut state = KernelState::new();
    assert!(matches!(
      state.create_object_with_capability(memory(), Rights::CREATE),
      Err(Error::Capability(_))
    ));
    assert_eq!(state.object_count(), 0);
    assert_eq!(state.capability_count(), 0);
  }

  #[test]
  fn kstate_003_derivation_updates_reference_count_transactionally() {
    let mut state = KernelState::new();
    let parent = state
      .create_object_with_capability(memory(), Rights::READ.union(Rights::GRANT))
      .unwrap();
    let derived = state.derive_capability(parent, Rights::READ).unwrap();
    assert_eq!(state.object_count(), 1);
    assert_eq!(state.capability_count(), 2);
    state.remove_capability(derived).unwrap();
    assert_eq!(state.object_count(), 1);
    assert_eq!(state.capability_count(), 1);
  }

  #[test]
  fn kstate_004_object_id_never_substitutes_for_authority() {
    let mut state = KernelState::new();
    let handle = state
      .create_object_with_capability(memory(), Rights::READ)
      .unwrap();
    assert!(matches!(
      state.lookup(handle, ObjectType::Memory, Rights::WRITE),
      Err(Error::Capability(crate::capability::Error::RightsDenied))
    ));
  }

  #[test]
  fn kstate_005_failed_derivation_leaves_counts_unchanged() {
    let mut state = KernelState::new();
    let parent = state
      .create_object_with_capability(memory(), Rights::READ)
      .unwrap();
    assert!(matches!(
      state.derive_capability(parent, Rights::READ),
      Err(Error::Capability(crate::capability::Error::RightsDenied))
    ));
    assert_eq!(state.object_count(), 1);
    assert_eq!(state.capability_count(), 1);
  }

  #[test]
  fn cspace_006_same_handle_in_another_space_has_no_authority() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let other = state.create_capability_space().unwrap();
    let handle = state
      .create_object_in_space(source, memory(), Rights::READ)
      .unwrap();
    assert!(matches!(
      state.lookup_in_space(other, handle, ObjectType::Memory, Rights::READ),
      Err(Error::Capability(crate::capability::Error::InvalidHandle))
    ));
  }

  #[test]
  fn cap_020_share_creates_attenuated_destination_capability() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(
        source,
        memory(),
        Rights::READ.union(Rights::WRITE).union(Rights::GRANT),
      )
      .unwrap();
    let shared = state
      .share_capability(source, parent, destination, Rights::READ)
      .unwrap();
    assert!(
      state
        .lookup_in_space(source, parent, ObjectType::Memory, Rights::WRITE)
        .is_ok()
    );
    assert!(
      state
        .lookup_in_space(destination, shared, ObjectType::Memory, Rights::READ)
        .is_ok()
    );
    assert!(matches!(
      state.lookup_in_space(destination, shared, ObjectType::Memory, Rights::WRITE),
      Err(Error::Capability(crate::capability::Error::RightsDenied))
    ));
  }

  #[test]
  fn cap_021_share_without_grant_is_rejected_without_partial_state() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(source, memory(), Rights::READ)
      .unwrap();
    let before = state.spaces.get(destination).unwrap().live_count();
    assert!(matches!(
      state.share_capability(source, parent, destination, Rights::READ),
      Err(Error::Capability(crate::capability::Error::RightsDenied))
    ));
    assert_eq!(state.spaces.get(destination).unwrap().live_count(), before);
  }

  #[test]
  fn cap_023_share_retains_source_and_records_lineage() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(source, memory(), Rights::READ.union(Rights::GRANT))
      .unwrap();
    let shared = state
      .share_capability(source, parent, destination, Rights::READ)
      .unwrap();
    let parent_capability = state.spaces.get(source).unwrap().snapshot(parent).unwrap();
    let child = state
      .spaces
      .get(destination)
      .unwrap()
      .snapshot(shared)
      .unwrap();
    assert_eq!(child.parent(), Some(parent_capability.node()));
    assert!(state.spaces.get(source).unwrap().snapshot(parent).is_ok());
  }

  #[test]
  fn cap_024_transfer_invalidates_source_and_preserves_node() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let handle = state
      .create_object_in_space(source, memory(), Rights::READ.union(Rights::TRANSFER))
      .unwrap();
    let node = state
      .spaces
      .get(source)
      .unwrap()
      .snapshot(handle)
      .unwrap()
      .node();
    let moved = state
      .transfer_capability(source, handle, destination)
      .unwrap();
    assert!(state.spaces.get(source).unwrap().snapshot(handle).is_err());
    assert_eq!(
      state
        .spaces
        .get(destination)
        .unwrap()
        .snapshot(moved)
        .unwrap()
        .node(),
      node
    );
  }

  #[test]
  fn cap_025_failed_transfer_changes_neither_space() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let handle = state
      .create_object_in_space(source, memory(), Rights::READ)
      .unwrap();
    assert!(matches!(
      state.transfer_capability(source, handle, destination),
      Err(Error::Capability(crate::capability::Error::RightsDenied))
    ));
    assert!(state.spaces.get(source).unwrap().snapshot(handle).is_ok());
    assert_eq!(state.spaces.get(destination).unwrap().live_count(), 0);
  }

  #[test]
  fn cap_026_full_destination_rolls_back_before_commit() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(
        source,
        memory(),
        Rights::READ.union(Rights::GRANT).union(Rights::TRANSFER),
      )
      .unwrap();
    for _ in 0..crate::capability::MAX_CAPABILITY_SLOTS {
      state
        .share_capability(source, parent, destination, Rights::READ)
        .unwrap();
    }
    let source_count = state.spaces.get(source).unwrap().live_count();
    let destination_count = state.spaces.get(destination).unwrap().live_count();
    assert!(matches!(
      state.share_capability(source, parent, destination, Rights::READ),
      Err(Error::Capability(crate::capability::Error::SpaceFull))
    ));
    assert!(state.spaces.get(source).unwrap().snapshot(parent).is_ok());
    assert_eq!(state.spaces.get(source).unwrap().live_count(), source_count);
    assert_eq!(
      state.spaces.get(destination).unwrap().live_count(),
      destination_count
    );
  }

  #[test]
  fn cap_030_share_increments_object_reference_count() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(source, memory(), Rights::READ.union(Rights::GRANT))
      .unwrap();
    let object = state
      .spaces
      .get(source)
      .unwrap()
      .snapshot(parent)
      .unwrap()
      .object();
    let before = state.objects.capability_references(object).unwrap();
    state
      .share_capability(source, parent, destination, Rights::READ)
      .unwrap();
    assert_eq!(
      state.objects.capability_references(object).unwrap(),
      before + 1
    );
  }

  #[test]
  fn cap_031_transfer_preserves_object_reference_count() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let handle = state
      .create_object_in_space(source, memory(), Rights::READ.union(Rights::TRANSFER))
      .unwrap();
    let object = state
      .spaces
      .get(source)
      .unwrap()
      .snapshot(handle)
      .unwrap()
      .object();
    let before = state.objects.capability_references(object).unwrap();
    state
      .transfer_capability(source, handle, destination)
      .unwrap();
    assert_eq!(state.objects.capability_references(object), Ok(before));
  }

  #[test]
  fn cap_032_same_space_transfer_is_rejected_without_change() {
    let mut state = KernelState::new();
    let space = state.bootstrap_space();
    let handle = state
      .create_object_in_space(space, memory(), Rights::READ.union(Rights::TRANSFER))
      .unwrap();
    assert!(matches!(
      state.transfer_capability(space, handle, space),
      Err(Error::CapabilitySpace(capability_space::Error::SameSpace))
    ));
    assert!(state.spaces.get(space).unwrap().snapshot(handle).is_ok());
  }

  #[test]
  fn cap_033_stale_source_does_not_consume_a_node_id() {
    let mut state = KernelState::new();
    let source = state.bootstrap_space();
    let destination = state.create_capability_space().unwrap();
    let parent = state
      .create_object_in_space(source, memory(), Rights::READ.union(Rights::GRANT))
      .unwrap();
    let stale = CapabilityHandle::from_raw_parts(parent.slot(), parent.generation() + 1);
    let next_node = state.next_capability_node;
    assert!(matches!(
      state.share_capability(source, stale, destination, Rights::READ),
      Err(Error::Capability(crate::capability::Error::StaleHandle))
    ));
    assert_eq!(state.next_capability_node, next_node);
  }

  #[test]
  fn sched_020_wake_updates_queue_and_thread_state_together() {
    let mut state = KernelState::new();
    let (_, thread) = scheduler_state(&mut state);
    let wake = state.wake_thread(thread).unwrap();
    assert!(matches!(wake.target_hart, 0 | 2));
    assert_eq!(
      state
        .objects
        .get(thread)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::Runnable
    );
    assert_eq!(state.scheduler.total_runnable(), 1);
  }

  #[test]
  fn sched_021_dispatch_updates_running_membership_and_thread_state() {
    let mut state = KernelState::new();
    let (_, thread) = scheduler_state(&mut state);
    let target = state.wake_thread(thread).unwrap().target_hart;
    assert_eq!(state.dispatch_next(target), Ok(Some(thread)));
    let object = state.objects.get(thread).unwrap().as_thread().unwrap();
    assert_eq!(object.state(), ThreadState::Running);
    assert_eq!(object.running_hart(), Some(target as u16));
  }

  #[test]
  fn sched_022_preempt_returns_thread_to_one_queue() {
    let mut state = KernelState::new();
    let (_, thread) = scheduler_state(&mut state);
    let target = state.wake_thread(thread).unwrap().target_hart;
    state.dispatch_next(target).unwrap();
    state.preempt_current(target).unwrap();
    assert_eq!(
      state
        .objects
        .get(thread)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::Runnable
    );
    assert_eq!(state.scheduler.total_running(), 0);
    assert_eq!(state.scheduler.total_runnable(), 1);
  }

  #[test]
  fn sched_023_block_removes_thread_from_scheduler() {
    let mut state = KernelState::new();
    let (_, thread) = scheduler_state(&mut state);
    let target = state.wake_thread(thread).unwrap().target_hart;
    state.dispatch_next(target).unwrap();
    assert_eq!(state.block_current(target), Ok(thread));
    assert_eq!(
      state
        .objects
        .get(thread)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::Blocked
    );
    assert_eq!(state.scheduler.total_running(), 0);
    assert_eq!(state.scheduler.total_runnable(), 0);
  }

  #[test]
  fn sched_024_stopping_process_cannot_wake_thread() {
    let mut state = KernelState::new();
    let (process, thread) = scheduler_state(&mut state);
    state
      .objects
      .get_mut(process)
      .unwrap()
      .as_process_mut()
      .unwrap()
      .begin_stop()
      .unwrap();
    assert_eq!(state.wake_thread(thread), Err(Error::ProcessNotAlive));
    assert_eq!(state.scheduler.total_runnable(), 0);
    assert_eq!(
      state
        .objects
        .get(thread)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::New
    );
  }

  #[test]
  fn memory_create_001_commits_quota_object_and_capability_together() {
    let mut state = KernelState::new();
    let bootstrap = state.bootstrap_space();
    let (process, owner_space) = initialize_memory_creation_state(&mut state, 4, 0, Rights::CREATE);
    let before_objects = state.object_count();
    let before_capabilities = state.spaces.get(owner_space).unwrap().live_count();
    let frame = crate::memory::frame_allocator::frame_block_for_test(0x9000_0000, 1);

    let created = state
      .create_memory_in_space(bootstrap, process, frame)
      .unwrap_or_else(|_| panic!("Memory Object creation failed"));

    assert_eq!(state.object_count(), before_objects + 1);
    assert_eq!(
      state.spaces.get(owner_space).unwrap().live_count(),
      before_capabilities + 1
    );
    assert!(matches!(
      state.lookup_in_space(
        owner_space,
        created.capability,
        ObjectType::Memory,
        Rights::MAP
      ),
      Ok(KernelObject::Memory(memory)) if memory.frame_start() == 0x9000_0000
    ));
  }

  #[test]
  fn memory_create_002_quota_failure_returns_frame_without_partial_state() {
    let mut state = KernelState::new();
    let bootstrap = state.bootstrap_space();
    let (process, owner_space) = initialize_memory_creation_state(&mut state, 1, 1, Rights::CREATE);
    let before = (
      state.object_count(),
      state.spaces.get(owner_space).unwrap().live_count(),
    );
    let frame = crate::memory::frame_allocator::frame_block_for_test(0x9000_0000, 1);

    let failure = state
      .create_memory_in_space(bootstrap, process, frame)
      .err()
      .unwrap_or_else(|| panic!("page quota accepted Memory Object creation"));
    let (error, frame) = failure.into_parts();

    assert_eq!(error, Error::Process(crate::process::Error::ResourceLimit));
    assert_eq!(frame.start(), 0x9000_0000);
    assert_eq!(
      (
        state.object_count(),
        state.spaces.get(owner_space).unwrap().live_count(),
      ),
      before
    );
  }

  #[test]
  fn memory_create_003_create_right_is_required() {
    let mut state = KernelState::new();
    let bootstrap = state.bootstrap_space();
    let (process_without_create, _) =
      initialize_memory_creation_state(&mut state, 4, 0, Rights::READ);
    let frame = crate::memory::frame_allocator::frame_block_for_test(0x9000_0000, 1);
    let failure = state
      .create_memory_in_space(bootstrap, process_without_create, frame)
      .err()
      .unwrap_or_else(|| panic!("missing CREATE accepted Memory Object creation"));
    assert!(matches!(failure.error(), Error::Capability(_)));
  }

  #[test]
  fn thread_create_001_commits_quota_object_and_capability_together() {
    let mut state = KernelState::new();
    let (process, owner_space) = initialize_thread_creation_state(&mut state, 2);
    let before_objects = state.object_count();
    let created = state
      .create_thread_in_space(
        state.bootstrap_space(),
        process,
        1,
        8,
        0x8020_0000,
        crate::memory::kernel_stack::kernel_stack_for_test(0x9200_0000),
      )
      .unwrap_or_else(|_| panic!("Thread creation failed"));

    assert_eq!(state.object_count(), before_objects + 1);
    assert!(
      state
        .lookup_in_space(
          owner_space,
          created.capability,
          ObjectType::Thread,
          Rights::READ,
        )
        .is_ok()
    );
    assert!(
      state
        .objects
        .get(created.object)
        .unwrap()
        .as_thread()
        .unwrap()
        .has_execution()
    );
  }

  #[test]
  fn thread_create_002_quota_failure_returns_stack_without_partial_state() {
    let mut state = KernelState::new();
    let (process, _) = initialize_thread_creation_state(&mut state, 1);
    let first = state
      .create_thread_in_space(
        state.bootstrap_space(),
        process,
        1,
        8,
        0x8020_0000,
        crate::memory::kernel_stack::kernel_stack_for_test(0x9200_0000),
      )
      .unwrap_or_else(|_| panic!("first Thread creation failed"));
    assert!(state.objects.get(first.object).is_ok());
    let before = state.object_count();

    let failure = state
      .create_thread_in_space(
        state.bootstrap_space(),
        process,
        1,
        8,
        0x8020_0000,
        crate::memory::kernel_stack::kernel_stack_for_test(0x9201_0000),
      )
      .expect_err("quota must reject second Thread");
    assert!(matches!(
      failure.error(),
      Error::Process(crate::process::Error::ResourceLimit)
    ));
    assert_eq!(failure.into_parts().1.base(), 0x9201_0000);
    assert_eq!(state.object_count(), before);
  }

  #[test]
  fn thread_create_003_create_right_is_required() {
    let mut state = KernelState::new();
    let owner_space = state.create_capability_space().unwrap();
    let mut process = Process::new(
      ObjectId::from_raw_parts(80, 1),
      owner_space,
      ProcessLimits::new(2, 4, 8, 8).unwrap(),
    );
    process.start().unwrap();
    let handle = state
      .create_object_with_capability(KernelObject::Process(process), Rights::READ)
      .unwrap();
    let failure = state
      .create_thread_in_space(
        state.bootstrap_space(),
        handle,
        1,
        8,
        0x8020_0000,
        crate::memory::kernel_stack::kernel_stack_for_test(0x9200_0000),
      )
      .expect_err("READ must not authorize Thread creation");
    assert!(matches!(failure.error(), Error::Capability(_)));
    assert_eq!(failure.into_parts().1.base(), 0x9200_0000);
  }

  #[test]
  fn sched_025_executionless_thread_cannot_wake() {
    let mut state = KernelState::new();
    state.initialize_scheduler(0b1111).unwrap();
    let mut process = Process::new(
      ObjectId::from_raw_parts(90, 1),
      state.bootstrap_space(),
      ProcessLimits::new(4, 4, 8, 8).unwrap(),
    );
    process.start().unwrap();
    let process = state
      .insert_test_object(KernelObject::Process(process))
      .unwrap();
    let thread = state
      .insert_test_object(KernelObject::Thread(
        Thread::new(process, 0b0101, 8).unwrap(),
      ))
      .unwrap();

    assert_eq!(
      state.wake_thread(thread),
      Err(Error::Thread(crate::thread::Error::ExecutionMissing))
    );
    assert_eq!(state.scheduler.total_runnable(), 0);
    assert_eq!(
      state
        .objects
        .get(thread)
        .unwrap()
        .as_thread()
        .unwrap()
        .state(),
      ThreadState::New
    );
  }
}
