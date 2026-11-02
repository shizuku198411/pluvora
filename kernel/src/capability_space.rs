use crate::capability::CapabilitySpace;

pub const MAX_CAPABILITY_SPACES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilitySpaceId {
  slot: u16,
  generation: u32,
}

impl CapabilitySpaceId {
  const fn bootstrap() -> Self {
    Self {
      slot: 0,
      generation: 1,
    }
  }

  const fn slot(self) -> usize {
    self.slot as usize
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  TableFull,
  InvalidSpace,
  SameSpace,
}

struct CapabilitySpaceSlot {
  generation: u32,
  space: Option<CapabilitySpace>,
}

impl CapabilitySpaceSlot {
  const fn empty() -> Self {
    Self {
      generation: 1,
      space: None,
    }
  }

  const fn bootstrap() -> Self {
    Self {
      generation: 1,
      space: Some(CapabilitySpace::new()),
    }
  }
}

pub struct CapabilitySpaceTable {
  slots: [CapabilitySpaceSlot; MAX_CAPABILITY_SPACES],
  live: usize,
}

impl CapabilitySpaceTable {
  pub const fn new() -> Self {
    let mut slots = [const { CapabilitySpaceSlot::empty() }; MAX_CAPABILITY_SPACES];
    slots[0] = CapabilitySpaceSlot::bootstrap();
    Self { slots, live: 1 }
  }

  pub(crate) const fn bootstrap_id(&self) -> CapabilitySpaceId {
    CapabilitySpaceId::bootstrap()
  }

  pub(crate) fn create(&mut self) -> Result<CapabilitySpaceId, Error> {
    let (index, slot) = self
      .slots
      .iter_mut()
      .enumerate()
      .find(|(_, slot)| slot.space.is_none())
      .ok_or(Error::TableFull)?;
    slot.space = Some(CapabilitySpace::new());
    self.live = self.live.checked_add(1).ok_or(Error::TableFull)?;
    Ok(CapabilitySpaceId {
      slot: index as u16,
      generation: slot.generation,
    })
  }

  pub(crate) fn get(&self, id: CapabilitySpaceId) -> Result<&CapabilitySpace, Error> {
    let slot = self.slots.get(id.slot()).ok_or(Error::InvalidSpace)?;
    if slot.generation != id.generation {
      return Err(Error::InvalidSpace);
    }
    slot.space.as_ref().ok_or(Error::InvalidSpace)
  }

  pub(crate) fn get_mut(&mut self, id: CapabilitySpaceId) -> Result<&mut CapabilitySpace, Error> {
    let slot = self.slots.get_mut(id.slot()).ok_or(Error::InvalidSpace)?;
    if slot.generation != id.generation {
      return Err(Error::InvalidSpace);
    }
    slot.space.as_mut().ok_or(Error::InvalidSpace)
  }

  pub(crate) fn pair_mut(
    &mut self,
    left: CapabilitySpaceId,
    right: CapabilitySpaceId,
  ) -> Result<(&mut CapabilitySpace, &mut CapabilitySpace), Error> {
    self.get(left)?;
    self.get(right)?;
    if left.slot == right.slot {
      return Err(Error::SameSpace);
    }

    let left_index = left.slot();
    let right_index = right.slot();
    if left_index < right_index {
      let (lower, upper) = self.slots.split_at_mut(right_index);
      let left_space = lower[left_index]
        .space
        .as_mut()
        .ok_or(Error::InvalidSpace)?;
      let right_space = upper[0].space.as_mut().ok_or(Error::InvalidSpace)?;
      Ok((left_space, right_space))
    } else {
      let (lower, upper) = self.slots.split_at_mut(left_index);
      let right_space = lower[right_index]
        .space
        .as_mut()
        .ok_or(Error::InvalidSpace)?;
      let left_space = upper[0].space.as_mut().ok_or(Error::InvalidSpace)?;
      Ok((left_space, right_space))
    }
  }

  pub const fn live_count(&self) -> usize {
    self.live
  }
}

impl Default for CapabilitySpaceTable {
  fn default() -> Self {
    Self::new()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn cspace_001_bootstrap_space_is_live() {
    let table = CapabilitySpaceTable::new();
    assert!(table.get(table.bootstrap_id()).is_ok());
    assert_eq!(table.live_count(), 1);
  }

  #[test]
  fn cspace_002_each_created_space_has_distinct_identity() {
    let mut table = CapabilitySpaceTable::new();
    let first = table.create().unwrap();
    let second = table.create().unwrap();
    assert_ne!(first, second);
    assert!(table.get(first).is_ok());
    assert!(table.get(second).is_ok());
  }

  #[test]
  fn cspace_003_generation_mismatch_is_rejected() {
    let table = CapabilitySpaceTable::new();
    let stale = CapabilitySpaceId {
      slot: 0,
      generation: 2,
    };
    assert!(matches!(table.get(stale), Err(Error::InvalidSpace)));
  }

  #[test]
  fn cspace_004_pair_mut_rejects_the_same_space() {
    let mut table = CapabilitySpaceTable::new();
    let bootstrap = table.bootstrap_id();
    assert!(matches!(
      table.pair_mut(bootstrap, bootstrap),
      Err(Error::SameSpace)
    ));
  }

  #[test]
  fn cspace_005_capacity_is_bounded() {
    let mut table = CapabilitySpaceTable::new();
    for _ in 1..MAX_CAPABILITY_SPACES {
      table.create().unwrap();
    }
    assert!(matches!(table.create(), Err(Error::TableFull)));
  }
}
