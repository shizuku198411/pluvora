use crate::object::{ObjectId, ObjectType};

pub const MAX_CAPABILITY_SLOTS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rights(u16);

impl Rights {
  pub const READ: Self = Self(1 << 0);
  pub const WRITE: Self = Self(1 << 1);
  pub const EXECUTE: Self = Self(1 << 2);
  pub const MAP: Self = Self(1 << 3);
  pub const SHARE: Self = Self(1 << 4);
  pub const TRANSFER: Self = Self(1 << 5);
  pub const GRANT: Self = Self(1 << 6);
  pub const REVOKE: Self = Self(1 << 7);
  pub const CREATE: Self = Self(1 << 8);
  pub const DESTROY: Self = Self(1 << 9);

  pub const fn empty() -> Self {
    Self(0)
  }

  pub const fn union(self, other: Self) -> Self {
    Self(self.0 | other.0)
  }

  pub const fn contains(self, required: Self) -> bool {
    self.0 & required.0 == required.0
  }

  pub const fn is_empty(self) -> bool {
    self.0 == 0
  }

  pub const fn is_subset_of(self, parent: Self) -> bool {
    parent.contains(self)
  }

  pub const fn bits(self) -> u16 {
    self.0
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityHandle {
  slot: u16,
  generation: u32,
}

impl CapabilityHandle {
  pub const fn from_raw_parts(slot: u16, generation: u32) -> Self {
    Self { slot, generation }
  }

  pub const fn slot(self) -> u16 {
    self.slot
  }

  pub const fn generation(self) -> u32 {
    self.generation
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CapabilityNodeId(u64);

impl CapabilityNodeId {
  pub(crate) const fn new(value: u64) -> Option<Self> {
    if value == 0 { None } else { Some(Self(value)) }
  }
}

#[derive(Clone, Copy)]
pub(crate) struct Capability {
  object: ObjectId,
  object_type: ObjectType,
  rights: Rights,
  node: CapabilityNodeId,
  parent: Option<CapabilityNodeId>,
}

impl Capability {
  pub(crate) fn initial(
    object: ObjectId,
    object_type: ObjectType,
    rights: Rights,
    node: CapabilityNodeId,
  ) -> Result<Self, Error> {
    validate_rights(object_type, rights)?;
    Ok(Self {
      object,
      object_type,
      rights,
      node,
      parent: None,
    })
  }

  pub(crate) fn derived(
    parent: Self,
    rights: Rights,
    node: CapabilityNodeId,
  ) -> Result<Self, Error> {
    validate_rights(parent.object_type, rights)?;
    if !parent.rights.contains(Rights::GRANT) || !rights.is_subset_of(parent.rights) {
      return Err(Error::RightsDenied);
    }
    Ok(Self {
      object: parent.object,
      object_type: parent.object_type,
      rights,
      node,
      parent: Some(parent.node),
    })
  }

  pub(crate) const fn object(self) -> ObjectId {
    self.object
  }

  pub(crate) const fn rights(self) -> Rights {
    self.rights
  }

  pub(crate) const fn node(self) -> CapabilityNodeId {
    self.node
  }

  pub(crate) const fn parent(self) -> Option<CapabilityNodeId> {
    self.parent
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  SpaceFull,
  InvalidHandle,
  StaleHandle,
  InvalidRights,
  RightsDenied,
  WrongObjectType,
  GenerationExhausted,
}

#[derive(Clone, Copy)]
struct CapabilitySlot {
  generation: u32,
  capability: Option<Capability>,
  quarantined: bool,
}

impl CapabilitySlot {
  const fn empty() -> Self {
    Self {
      generation: 1,
      capability: None,
      quarantined: false,
    }
  }
}

pub struct CapabilitySpace {
  slots: [CapabilitySlot; MAX_CAPABILITY_SLOTS],
  live: usize,
}

impl CapabilitySpace {
  pub const fn new() -> Self {
    Self {
      slots: [const { CapabilitySlot::empty() }; MAX_CAPABILITY_SLOTS],
      live: 0,
    }
  }

  pub(crate) fn insert_initial(
    &mut self,
    object: ObjectId,
    object_type: ObjectType,
    rights: Rights,
    node: CapabilityNodeId,
  ) -> Result<CapabilityHandle, Error> {
    self.insert_existing(Capability::initial(object, object_type, rights, node)?)
  }

  pub(crate) fn lookup(
    &self,
    handle: CapabilityHandle,
    expected_type: ObjectType,
    required: Rights,
  ) -> Result<ObjectId, Error> {
    let capability = self.capability(handle)?;
    if capability.object_type != expected_type {
      return Err(Error::WrongObjectType);
    }
    if !capability.rights.contains(required) {
      return Err(Error::RightsDenied);
    }
    Ok(capability.object)
  }

  pub(crate) fn identity(&self, handle: CapabilityHandle) -> Result<(ObjectId, ObjectType), Error> {
    let capability = self.capability(handle)?;
    Ok((capability.object, capability.object_type))
  }

  pub(crate) fn snapshot(&self, handle: CapabilityHandle) -> Result<Capability, Error> {
    self.capability(handle).copied()
  }

  pub(crate) fn has_free_slot(&self) -> bool {
    self
      .slots
      .iter()
      .any(|slot| slot.capability.is_none() && !slot.quarantined)
  }

  pub(crate) fn insert_existing(
    &mut self,
    capability: Capability,
  ) -> Result<CapabilityHandle, Error> {
    validate_rights(capability.object_type, capability.rights)?;
    self.insert(capability)
  }

  pub(crate) fn remove_full(&mut self, handle: CapabilityHandle) -> Result<Capability, Error> {
    let index = handle.slot as usize;
    let slot = self.slots.get_mut(index).ok_or(Error::InvalidHandle)?;
    if slot.quarantined || slot.generation != handle.generation {
      return Err(Error::StaleHandle);
    }
    let capability = slot.capability.take().ok_or(Error::InvalidHandle)?;
    self.live = self.live.checked_sub(1).ok_or(Error::InvalidHandle)?;

    match slot.generation.checked_add(1) {
      Some(generation) => slot.generation = generation,
      None => slot.quarantined = true,
    }
    Ok(capability)
  }

  pub(crate) fn derive_local(
    &mut self,
    parent: CapabilityHandle,
    rights: Rights,
    node: CapabilityNodeId,
  ) -> Result<(CapabilityHandle, ObjectId), Error> {
    let parent = *self.capability(parent)?;
    let derived = Capability::derived(parent, rights, node)?;
    let handle = self.insert_existing(derived)?;
    Ok((handle, parent.object))
  }

  pub(crate) fn remove(&mut self, handle: CapabilityHandle) -> Result<ObjectId, Error> {
    self.remove_full(handle).map(Capability::object)
  }

  pub const fn live_count(&self) -> usize {
    self.live
  }

  fn insert(&mut self, capability: Capability) -> Result<CapabilityHandle, Error> {
    let (index, slot) = self
      .slots
      .iter_mut()
      .enumerate()
      .find(|(_, slot)| slot.capability.is_none() && !slot.quarantined)
      .ok_or(Error::SpaceFull)?;
    slot.capability = Some(capability);
    self.live = self.live.checked_add(1).ok_or(Error::SpaceFull)?;
    Ok(CapabilityHandle {
      slot: index as u16,
      generation: slot.generation,
    })
  }

  fn capability(&self, handle: CapabilityHandle) -> Result<&Capability, Error> {
    let slot = self
      .slots
      .get(handle.slot as usize)
      .ok_or(Error::InvalidHandle)?;
    if slot.quarantined || slot.generation != handle.generation {
      return Err(Error::StaleHandle);
    }
    slot.capability.as_ref().ok_or(Error::InvalidHandle)
  }
}

impl Default for CapabilitySpace {
  fn default() -> Self {
    Self::new()
  }
}

pub const fn valid_rights(object_type: ObjectType) -> Rights {
  match object_type {
    ObjectType::Memory => Rights::READ
      .union(Rights::WRITE)
      .union(Rights::EXECUTE)
      .union(Rights::MAP)
      .union(Rights::SHARE)
      .union(Rights::TRANSFER)
      .union(Rights::GRANT)
      .union(Rights::REVOKE)
      .union(Rights::DESTROY),
    ObjectType::AddressSpace => Rights::MAP
      .union(Rights::SHARE)
      .union(Rights::TRANSFER)
      .union(Rights::GRANT)
      .union(Rights::REVOKE)
      .union(Rights::DESTROY),
    ObjectType::Process | ObjectType::Thread => Rights::READ
      .union(Rights::WRITE)
      .union(Rights::TRANSFER)
      .union(Rights::GRANT)
      .union(Rights::REVOKE)
      .union(Rights::DESTROY),
  }
}

fn validate_rights(object_type: ObjectType, rights: Rights) -> Result<(), Error> {
  if rights.is_empty() || !rights.is_subset_of(valid_rights(object_type)) {
    return Err(Error::InvalidRights);
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  const OBJECT: ObjectId = ObjectId::from_raw_parts(7, 3);

  fn node(value: u64) -> CapabilityNodeId {
    CapabilityNodeId::new(value).unwrap()
  }

  fn memory_space(rights: Rights) -> (CapabilitySpace, CapabilityHandle) {
    let mut space = CapabilitySpace::new();
    let handle = space
      .insert_initial(OBJECT, ObjectType::Memory, rights, node(1))
      .unwrap();
    (space, handle)
  }

  #[test]
  fn cap_001_lookup_requires_matching_generation() {
    let (space, handle) = memory_space(Rights::READ);
    let stale = CapabilityHandle::from_raw_parts(handle.slot(), handle.generation() + 1);
    assert_eq!(
      space.lookup(stale, ObjectType::Memory, Rights::READ),
      Err(Error::StaleHandle)
    );
  }

  #[test]
  fn cap_002_lookup_requires_object_type_and_rights() {
    let (space, handle) = memory_space(Rights::READ);
    assert_eq!(
      space.lookup(handle, ObjectType::AddressSpace, Rights::READ),
      Err(Error::WrongObjectType)
    );
    assert_eq!(
      space.lookup(handle, ObjectType::Memory, Rights::WRITE),
      Err(Error::RightsDenied)
    );
  }

  #[test]
  fn cap_003_derivation_cannot_amplify_rights() {
    let parent_rights = Rights::READ.union(Rights::GRANT);
    let (mut space, parent) = memory_space(parent_rights);
    assert_eq!(
      space.derive_local(parent, Rights::READ.union(Rights::WRITE), node(2)),
      Err(Error::RightsDenied)
    );
  }

  #[test]
  fn cap_004_derivation_requires_grant_authority() {
    let (mut space, parent) = memory_space(Rights::READ);
    assert_eq!(
      space.derive_local(parent, Rights::READ, node(2)),
      Err(Error::RightsDenied)
    );
  }

  #[test]
  fn cap_005_derived_capability_may_reduce_rights() {
    let parent_rights = Rights::READ.union(Rights::WRITE).union(Rights::GRANT);
    let (mut space, parent) = memory_space(parent_rights);
    let (derived, object) = space.derive_local(parent, Rights::READ, node(2)).unwrap();
    assert_eq!(object, OBJECT);
    assert_eq!(
      space.lookup(derived, ObjectType::Memory, Rights::READ),
      Ok(OBJECT)
    );
  }

  #[test]
  fn cap_006_removed_handle_never_refers_to_reused_slot() {
    let (mut space, old) = memory_space(Rights::READ);
    assert_eq!(space.remove(old), Ok(OBJECT));
    let new = space
      .insert_initial(OBJECT, ObjectType::Memory, Rights::READ, node(2))
      .unwrap();
    assert_eq!(old.slot(), new.slot());
    assert_ne!(old.generation(), new.generation());
    assert_eq!(
      space.lookup(old, ObjectType::Memory, Rights::READ),
      Err(Error::StaleHandle)
    );
  }

  #[test]
  fn cap_007_object_specific_rights_are_enforced() {
    let mut space = CapabilitySpace::new();
    assert_eq!(
      space.insert_initial(OBJECT, ObjectType::AddressSpace, Rights::READ, node(1),),
      Err(Error::InvalidRights)
    );
    assert_eq!(
      space.insert_initial(OBJECT, ObjectType::Memory, Rights::CREATE, node(1)),
      Err(Error::InvalidRights)
    );
  }

  #[test]
  fn cap_008_out_of_range_handle_is_rejected_without_panic() {
    let space = CapabilitySpace::new();
    let handle = CapabilityHandle::from_raw_parts(u16::MAX, 1);
    assert_eq!(
      space.lookup(handle, ObjectType::Memory, Rights::READ),
      Err(Error::InvalidHandle)
    );
  }

  #[test]
  fn cap_009_empty_rights_are_rejected() {
    let mut space = CapabilitySpace::new();
    assert_eq!(
      space.insert_initial(OBJECT, ObjectType::Memory, Rights::empty(), node(1)),
      Err(Error::InvalidRights)
    );
  }

  #[test]
  fn cap_010_failed_derivation_does_not_consume_a_slot() {
    let (mut space, parent) = memory_space(Rights::READ.union(Rights::GRANT));
    let stale = CapabilityHandle::from_raw_parts(parent.slot(), parent.generation() + 1);
    assert_eq!(
      space.derive_local(stale, Rights::READ, node(2)),
      Err(Error::StaleHandle)
    );
    assert_eq!(space.live_count(), 1);
  }

  #[test]
  fn cap_011_full_space_rejects_another_capability() {
    let mut space = CapabilitySpace::new();
    for index in 0..MAX_CAPABILITY_SLOTS {
      space
        .insert_initial(
          OBJECT,
          ObjectType::Memory,
          Rights::READ,
          node(index as u64 + 1),
        )
        .unwrap();
    }
    assert_eq!(
      space.insert_initial(
        OBJECT,
        ObjectType::Memory,
        Rights::READ,
        node(MAX_CAPABILITY_SLOTS as u64 + 1),
      ),
      Err(Error::SpaceFull)
    );
  }

  #[test]
  fn cap_012_capability_cannot_be_removed_twice() {
    let (mut space, handle) = memory_space(Rights::READ);
    assert_eq!(space.remove(handle), Ok(OBJECT));
    assert_eq!(space.remove(handle), Err(Error::StaleHandle));
    assert_eq!(space.live_count(), 0);
  }

  #[test]
  fn cap_013_derivation_records_parent_node() {
    let parent_rights = Rights::READ.union(Rights::GRANT);
    let (mut space, parent) = memory_space(parent_rights);
    let (derived, _) = space.derive_local(parent, Rights::READ, node(2)).unwrap();
    let parent_capability = space.snapshot(parent).unwrap();
    let derived_capability = space.snapshot(derived).unwrap();
    assert_eq!(derived_capability.parent(), Some(parent_capability.node()));
    assert_ne!(derived_capability.node(), parent_capability.node());
  }

  #[test]
  fn cap_014_process_and_thread_rights_reject_memory_operations() {
    for object_type in [ObjectType::Process, ObjectType::Thread] {
      assert!(!valid_rights(object_type).contains(Rights::MAP));
      assert!(!valid_rights(object_type).contains(Rights::EXECUTE));
      assert!(valid_rights(object_type).contains(Rights::READ));
      assert!(valid_rights(object_type).contains(Rights::WRITE));
    }
  }
}
