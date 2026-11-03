use crate::memory::{PAGE_SIZE, address_space::AddressSpace, frame_allocator};
use crate::process::Process;
use crate::thread::Thread;

pub const MAX_OBJECTS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  TableFull,
  InvalidObject,
  WrongObjectType,
  InvalidMemoryFrame,
  DuplicateObject,
  ReferenceOverflow,
  ReferenceUnderflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectType {
  Memory,
  AddressSpace,
  Process,
  Thread,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectId {
  slot: u16,
  generation: u32,
}

impl ObjectId {
  #[cfg(test)]
  pub(crate) const fn from_raw_parts(slot: u16, generation: u32) -> Self {
    Self { slot, generation }
  }

  pub(crate) const fn slot(self) -> usize {
    self.slot as usize
  }

  pub(crate) const fn generation(self) -> u32 {
    self.generation
  }

  pub(crate) const fn into_raw(self) -> usize {
    ((self.generation as usize) << 16) | self.slot as usize
  }

  pub(crate) const fn from_raw(raw: usize) -> Option<Self> {
    let slot = raw & 0xffff;
    let generation = raw >> 16;
    if slot >= MAX_OBJECTS || generation == 0 || generation > u32::MAX as usize {
      return None;
    }
    Some(Self {
      slot: slot as u16,
      generation: generation as u32,
    })
  }
}

#[derive(Debug, Eq, PartialEq)]
pub struct MemoryObject {
  frame: frame_allocator::FrameBlock,
}

impl MemoryObject {
  pub fn new(frame: frame_allocator::FrameBlock) -> Result<Self, Error> {
    if frame.start() == 0 || !frame.start().is_multiple_of(PAGE_SIZE) || frame.frames() != 1 {
      return Err(Error::InvalidMemoryFrame);
    }
    Ok(Self { frame })
  }

  pub const fn frame_start(&self) -> u64 {
    self.frame.start()
  }

  pub fn into_frame(self) -> frame_allocator::FrameBlock {
    self.frame
  }
}

pub enum KernelObject {
  Memory(MemoryObject),
  AddressSpace(AddressSpace),
  Process(Process),
  Thread(Thread),
}

impl KernelObject {
  pub const fn object_type(&self) -> ObjectType {
    match self {
      Self::Memory(_) => ObjectType::Memory,
      Self::AddressSpace(_) => ObjectType::AddressSpace,
      Self::Process(_) => ObjectType::Process,
      Self::Thread(_) => ObjectType::Thread,
    }
  }

  pub const fn as_memory(&self) -> Option<&MemoryObject> {
    match self {
      Self::Memory(memory) => Some(memory),
      Self::AddressSpace(_) | Self::Process(_) | Self::Thread(_) => None,
    }
  }

  pub const fn as_address_space(&self) -> Option<&AddressSpace> {
    match self {
      Self::Memory(_) | Self::Process(_) | Self::Thread(_) => None,
      Self::AddressSpace(space) => Some(space),
    }
  }

  pub fn as_address_space_mut(&mut self) -> Option<&mut AddressSpace> {
    match self {
      Self::AddressSpace(space) => Some(space),
      Self::Memory(_) | Self::Process(_) | Self::Thread(_) => None,
    }
  }

  pub const fn as_process(&self) -> Option<&Process> {
    match self {
      Self::Process(process) => Some(process),
      Self::Memory(_) | Self::AddressSpace(_) | Self::Thread(_) => None,
    }
  }

  pub fn as_process_mut(&mut self) -> Option<&mut Process> {
    match self {
      Self::Process(process) => Some(process),
      Self::Memory(_) | Self::AddressSpace(_) | Self::Thread(_) => None,
    }
  }

  pub const fn as_thread(&self) -> Option<&Thread> {
    match self {
      Self::Thread(thread) => Some(thread),
      Self::Memory(_) | Self::AddressSpace(_) | Self::Process(_) => None,
    }
  }

  pub fn as_thread_mut(&mut self) -> Option<&mut Thread> {
    match self {
      Self::Thread(thread) => Some(thread),
      Self::Memory(_) | Self::AddressSpace(_) | Self::Process(_) => None,
    }
  }

  fn has_same_identity(&self, other: &Self) -> bool {
    match (self, other) {
      (Self::Memory(left), Self::Memory(right)) => left.frame_start() == right.frame_start(),
      (Self::AddressSpace(left), Self::AddressSpace(right)) => {
        left.root_frame() == right.root_frame() && left.asid() == right.asid()
      }
      _ => false,
    }
  }
}

pub struct ObjectSlot {
  generation: u32,
  object: Option<KernelObject>,
  capability_references: usize,
  mapping_references: usize,
}

impl ObjectSlot {
  const fn empty() -> Self {
    Self {
      generation: 1,
      object: None,
      capability_references: 0,
      mapping_references: 0,
    }
  }
}

pub struct ObjectTable {
  slots: [ObjectSlot; MAX_OBJECTS],
  live: usize,
}

impl ObjectTable {
  pub const fn new() -> Self {
    Self {
      slots: [const { ObjectSlot::empty() }; MAX_OBJECTS],
      live: 0,
    }
  }

  pub(crate) fn insert(&mut self, object: KernelObject) -> Result<ObjectId, Error> {
    if self
      .slots
      .iter()
      .filter_map(|slot| slot.object.as_ref())
      .any(|existing| existing.has_same_identity(&object))
    {
      return Err(Error::DuplicateObject);
    }

    let (index, slot) = self
      .slots
      .iter_mut()
      .enumerate()
      .find(|(_, slot)| slot.object.is_none())
      .ok_or(Error::TableFull)?;

    debug_assert_eq!(slot.capability_references, 0);
    debug_assert_eq!(slot.mapping_references, 0);
    slot.object = Some(object);
    self.live = self.live.checked_add(1).ok_or(Error::ReferenceOverflow)?;

    Ok(ObjectId {
      slot: index as u16,
      generation: slot.generation,
    })
  }

  pub(crate) fn get(&self, id: ObjectId) -> Result<&KernelObject, Error> {
    let slot = self.slots.get(id.slot()).ok_or(Error::InvalidObject)?;
    if slot.generation != id.generation() {
      return Err(Error::InvalidObject);
    }
    slot.object.as_ref().ok_or(Error::InvalidObject)
  }

  pub(crate) fn get_mut(&mut self, id: ObjectId) -> Result<&mut KernelObject, Error> {
    let slot = self.slot_mut(id)?;
    slot.object.as_mut().ok_or(Error::InvalidObject)
  }

  pub(crate) fn validate_type(&self, id: ObjectId, expected: ObjectType) -> Result<(), Error> {
    if self.get(id)?.object_type() != expected {
      return Err(Error::WrongObjectType);
    }
    Ok(())
  }

  pub(crate) fn add_capability_reference(&mut self, id: ObjectId) -> Result<(), Error> {
    let slot = self.slot_mut(id)?;
    slot.capability_references = slot
      .capability_references
      .checked_add(1)
      .ok_or(Error::ReferenceOverflow)?;
    Ok(())
  }

  pub(crate) fn can_add_capability_reference(&self, id: ObjectId) -> Result<(), Error> {
    self
      .capability_references(id)?
      .checked_add(1)
      .map(|_| ())
      .ok_or(Error::ReferenceOverflow)
  }

  pub(crate) fn remove_capability_reference(&mut self, id: ObjectId) -> Result<(), Error> {
    let slot = self.slot_mut(id)?;
    slot.capability_references = slot
      .capability_references
      .checked_sub(1)
      .ok_or(Error::ReferenceUnderflow)?;
    Ok(())
  }

  pub(crate) fn can_add_mapping_reference(&self, id: ObjectId) -> Result<(), Error> {
    self
      .mapping_references(id)?
      .checked_add(1)
      .map(|_| ())
      .ok_or(Error::ReferenceOverflow)
  }

  pub(crate) fn add_mapping_reference(&mut self, id: ObjectId) -> Result<(), Error> {
    let slot = self.slot_mut(id)?;
    slot.mapping_references = slot
      .mapping_references
      .checked_add(1)
      .ok_or(Error::ReferenceOverflow)?;
    Ok(())
  }

  #[cfg_attr(
    not(test),
    expect(dead_code, reason = "unmap will release mapping references")
  )]
  pub(crate) fn remove_mapping_reference(&mut self, id: ObjectId) -> Result<(), Error> {
    let slot = self.slot_mut(id)?;
    slot.mapping_references = slot
      .mapping_references
      .checked_sub(1)
      .ok_or(Error::ReferenceUnderflow)?;
    Ok(())
  }

  pub(crate) fn rollback_unreferenced_insert(&mut self, id: ObjectId) -> Result<(), Error> {
    let slot = self.slot_mut(id)?;
    if slot.capability_references != 0 || slot.mapping_references != 0 {
      return Err(Error::InvalidObject);
    }
    slot.object.take().ok_or(Error::InvalidObject)?;
    self.live = self.live.checked_sub(1).ok_or(Error::ReferenceUnderflow)?;
    Ok(())
  }

  pub(crate) fn capability_references(&self, id: ObjectId) -> Result<usize, Error> {
    let slot = self.slots.get(id.slot()).ok_or(Error::InvalidObject)?;
    if slot.generation != id.generation() || slot.object.is_none() {
      return Err(Error::InvalidObject);
    }
    Ok(slot.capability_references)
  }

  pub(crate) fn mapping_references(&self, id: ObjectId) -> Result<usize, Error> {
    let slot = self.slots.get(id.slot()).ok_or(Error::InvalidObject)?;
    if slot.generation != id.generation() || slot.object.is_none() {
      return Err(Error::InvalidObject);
    }
    Ok(slot.mapping_references)
  }

  pub const fn live_count(&self) -> usize {
    self.live
  }

  pub(crate) fn has_free_slot(&self) -> bool {
    self.slots.iter().any(|slot| slot.object.is_none())
  }

  fn slot_mut(&mut self, id: ObjectId) -> Result<&mut ObjectSlot, Error> {
    let slot = self.slots.get_mut(id.slot()).ok_or(Error::InvalidObject)?;
    if slot.generation != id.generation() || slot.object.is_none() {
      return Err(Error::InvalidObject);
    }
    Ok(slot)
  }
}

impl Default for ObjectTable {
  fn default() -> Self {
    Self::new()
  }
}

#[cfg(test)]
pub(crate) fn memory_object_for_test(frame_start: u64) -> MemoryObject {
  MemoryObject::new(frame_allocator::frame_block_for_test(frame_start, 1)).unwrap()
}

#[cfg(test)]
mod tests {
  use crate::capability_space::CapabilitySpaceId;
  use crate::memory::address_space::Asid;
  use crate::process::{Process, ProcessLimits};
  use crate::thread::Thread;

  use super::*;

  fn process_object() -> Process {
    Process::new(
      ObjectId::from_raw_parts(20, 1),
      CapabilitySpaceId::from_raw_parts_for_test(2, 1),
      ProcessLimits::new(4, 16, 16, 16).unwrap(),
    )
  }

  #[test]
  fn obj_001_empty_table_rejects_unknown_identity() {
    let table = ObjectTable::new();
    let id = ObjectId {
      slot: 0,
      generation: 1,
    };
    assert!(matches!(table.get(id), Err(Error::InvalidObject)));
  }

  #[test]
  fn obj_002_inserted_object_preserves_type_and_payload() {
    let mut table = ObjectTable::new();
    let memory = memory_object_for_test(0x8040_0000);
    let id = table.insert(KernelObject::Memory(memory)).unwrap();
    assert_eq!(table.get(id).unwrap().object_type(), ObjectType::Memory);
    assert_eq!(
      table.get(id).unwrap().as_memory().unwrap().frame_start(),
      0x8040_0000
    );
  }

  #[test]
  fn obj_003_wrong_object_type_is_rejected() {
    let mut table = ObjectTable::new();
    let space = AddressSpace::new(0x8060_0000, Asid::kernel()).unwrap();
    let id = table.insert(KernelObject::AddressSpace(space)).unwrap();
    assert_eq!(
      table.validate_type(id, ObjectType::Memory),
      Err(Error::WrongObjectType)
    );
  }

  #[test]
  fn obj_004_memory_object_requires_exactly_one_frame() {
    assert!(MemoryObject::new(frame_allocator::frame_block_for_test(0, 1)).is_err());
    assert!(MemoryObject::new(frame_allocator::frame_block_for_test(0x8040_0001, 1)).is_err());
    assert!(MemoryObject::new(frame_allocator::frame_block_for_test(0x8040_0000, 2)).is_err());
    assert!(MemoryObject::new(frame_allocator::frame_block_for_test(0x8040_0000, 1)).is_ok());
  }

  #[test]
  fn obj_005_reference_count_is_checked() {
    let mut table = ObjectTable::new();
    let id = table
      .insert(KernelObject::Memory(memory_object_for_test(0x8040_0000)))
      .unwrap();
    assert_eq!(table.capability_references(id), Ok(0));
    table.add_capability_reference(id).unwrap();
    assert_eq!(table.capability_references(id), Ok(1));
    table.remove_capability_reference(id).unwrap();
    assert_eq!(table.capability_references(id), Ok(0));
    assert_eq!(
      table.remove_capability_reference(id),
      Err(Error::ReferenceUnderflow)
    );
  }

  #[test]
  fn obj_008_mapping_reference_is_independent_from_capability_reference() {
    let mut table = ObjectTable::new();
    let id = table
      .insert(KernelObject::Memory(memory_object_for_test(0x8060_0000)))
      .unwrap();
    assert_eq!(table.can_add_mapping_reference(id), Ok(()));
    table.add_capability_reference(id).unwrap();
    table.add_mapping_reference(id).unwrap();
    assert_eq!(table.capability_references(id), Ok(1));
    assert_eq!(table.mapping_references(id), Ok(1));
    table.remove_capability_reference(id).unwrap();
    assert_eq!(table.capability_references(id), Ok(0));
    assert_eq!(table.mapping_references(id), Ok(1));
    assert_eq!(
      table.rollback_unreferenced_insert(id),
      Err(Error::InvalidObject)
    );
    table.remove_mapping_reference(id).unwrap();
    assert_eq!(table.mapping_references(id), Ok(0));
  }

  #[test]
  fn obj_006_same_physical_page_cannot_back_two_memory_objects() {
    let mut table = ObjectTable::new();
    table
      .insert(KernelObject::Memory(memory_object_for_test(0x8040_0000)))
      .unwrap();
    assert!(matches!(
      table.insert(KernelObject::Memory(memory_object_for_test(0x8040_0000))),
      Err(Error::DuplicateObject)
    ));
  }

  #[test]
  fn obj_007_full_table_rejects_another_object() {
    let mut table = ObjectTable::new();
    for index in 0..MAX_OBJECTS {
      let frame = 0x8100_0000 + (index as u64) * PAGE_SIZE;
      table
        .insert(KernelObject::Memory(memory_object_for_test(frame)))
        .unwrap();
    }
    assert!(matches!(
      table.insert(KernelObject::Memory(memory_object_for_test(0x9000_0000))),
      Err(Error::TableFull)
    ));
  }

  #[test]
  fn obj_010_process_and_thread_payloads_preserve_their_type() {
    let mut table = ObjectTable::new();
    let process = table
      .insert(KernelObject::Process(process_object()))
      .unwrap();
    let thread = Thread::new(process, 1, 8).unwrap();
    let thread = table.insert(KernelObject::Thread(thread)).unwrap();
    assert_eq!(
      table.get(process).unwrap().object_type(),
      ObjectType::Process
    );
    assert_eq!(table.get(thread).unwrap().object_type(), ObjectType::Thread);
  }

  #[test]
  fn obj_011_typed_accessors_do_not_cross_object_types() {
    let process = KernelObject::Process(process_object());
    assert!(process.as_process().is_some());
    assert!(process.as_thread().is_none());
    assert!(process.as_memory().is_none());
    assert!(process.as_address_space().is_none());

    let thread = KernelObject::Thread(Thread::new(ObjectId::from_raw_parts(21, 1), 1, 8).unwrap());
    assert!(thread.as_thread().is_some());
    assert!(thread.as_process().is_none());
    assert!(thread.as_memory().is_none());
    assert!(thread.as_address_space().is_none());
  }

  #[test]
  fn obj_012_object_id_raw_encoding_preserves_generation() {
    let id = ObjectId::from_raw_parts(7, 19);
    assert_eq!(ObjectId::from_raw(id.into_raw()), Some(id));
    assert_eq!(ObjectId::from_raw(0), None);
  }
}
