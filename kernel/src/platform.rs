use core::str;

use crate::dtb::{Dtb, Event};

const DEFAULT_ADDRESS_CELLS: u32 = 2;
const DEFAULT_SIZE_CELLS: u32 = 1;
const MAX_SUPPORTED_CELLS: u32 = 2;
const MAX_DTB_DEPTH: usize = 16;

pub const MAX_HARTS: usize = 64;
pub const MAX_MEMORY_REGIONS: usize = 8;
pub const MAX_RESERVED_REGIONS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  Dtb(crate::dtb::Error),
  InvalidDepth,
  InvalidProperty,
  UnsupportedCellCount,
  TooManyHarts,
  TooManyMemoryRegions,
  DuplicateHart,
  MissingBootHart,
  InvalidMemoryRegion,
  OverlappingMemoryRegions,
  TooManyReservedRegions,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Dtb(error) => error.as_str(),
      Self::InvalidDepth => "invalid DTB node depth",
      Self::InvalidProperty => "invalid platform property",
      Self::UnsupportedCellCount => "unsupported address or size cell count",
      Self::TooManyHarts => "too many harts",
      Self::TooManyMemoryRegions => "too many memory regions",
      Self::DuplicateHart => "duplicate hart ID",
      Self::MissingBootHart => "boot hart is missing or disabled",
      Self::InvalidMemoryRegion => "invalid physical memory region",
      Self::OverlappingMemoryRegions => "physical memory regions overlap",
      Self::TooManyReservedRegions => "too many reserved memory regions",
    }
  }
}

impl From<crate::dtb::Error> for Error {
  fn from(error: crate::dtb::Error) -> Self {
    Self::Dtb(error)
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryRegion {
  pub base: u64,
  pub size: u64,
}

impl MemoryRegion {
  const EMPTY: Self = Self { base: 0, size: 0 };

  pub fn end(self) -> Option<u64> {
    self.base.checked_add(self.size)
  }
}

pub struct PlatformInfo {
  harts: [usize; MAX_HARTS],
  hart_count: usize,
  memory_regions: [MemoryRegion; MAX_MEMORY_REGIONS],
  memory_region_count: usize,
  reserved_regions: [MemoryRegion; MAX_RESERVED_REGIONS],
  reserved_region_count: usize,
}

impl PlatformInfo {
  const fn new() -> Self {
    Self {
      harts: [0; MAX_HARTS],
      hart_count: 0,
      memory_regions: [MemoryRegion::EMPTY; MAX_MEMORY_REGIONS],
      memory_region_count: 0,
      reserved_regions: [MemoryRegion::EMPTY; MAX_RESERVED_REGIONS],
      reserved_region_count: 0,
    }
  }

  pub fn harts(&self) -> &[usize] {
    &self.harts[..self.hart_count]
  }

  pub fn memory_regions(&self) -> &[MemoryRegion] {
    &self.memory_regions[..self.memory_region_count]
  }

  pub fn hart_index(&self, hart_id: usize) -> Option<usize> {
    self
      .harts()
      .iter()
      .position(|candidate| *candidate == hart_id)
  }

  pub fn reserved_regions(&self) -> &[MemoryRegion] {
    &self.reserved_regions[..self.reserved_region_count]
  }

  #[cfg(test)]
  pub(crate) fn for_test(memory: &[MemoryRegion], reserved: &[MemoryRegion]) -> Self {
    assert!(memory.len() <= MAX_MEMORY_REGIONS);
    assert!(reserved.len() <= MAX_RESERVED_REGIONS);

    let mut info = Self::new();
    for region in memory {
      info.memory_regions[info.memory_region_count] = *region;
      info.memory_region_count += 1;
    }
    for region in reserved {
      info.reserved_regions[info.reserved_region_count] = *region;
      info.reserved_region_count += 1;
    }
    info
  }

  fn add_reserved_region(&mut self, region: MemoryRegion) -> Result<(), Error> {
    if region.size == 0 || region.end().is_none() {
      return Err(Error::InvalidMemoryRegion);
    }
    if self.reserved_region_count == MAX_RESERVED_REGIONS {
      return Err(Error::TooManyReservedRegions);
    }

    self.reserved_regions[self.reserved_region_count] = region;
    self.reserved_region_count += 1;
    Ok(())
  }

  fn add_hart(&mut self, hart_id: usize) -> Result<(), Error> {
    if self.hart_index(hart_id).is_some() {
      return Err(Error::DuplicateHart);
    }
    if self.hart_count == MAX_HARTS {
      return Err(Error::TooManyHarts);
    }

    self.harts[self.hart_count] = hart_id;
    self.hart_count += 1;
    Ok(())
  }

  fn add_memory_region(&mut self, region: MemoryRegion) -> Result<(), Error> {
    let region_end = region.end().ok_or(Error::InvalidMemoryRegion)?;
    if region.size == 0 {
      return Err(Error::InvalidMemoryRegion);
    }

    for existing in self.memory_regions() {
      let existing_end = existing.end().ok_or(Error::InvalidMemoryRegion)?;
      if region.base < existing_end && existing.base < region_end {
        return Err(Error::OverlappingMemoryRegions);
      }
    }

    if self.memory_region_count == MAX_MEMORY_REGIONS {
      return Err(Error::TooManyMemoryRegions);
    }

    self.memory_regions[self.memory_region_count] = region;
    self.memory_region_count += 1;
    Ok(())
  }
}

#[derive(Clone, Copy)]
struct NodeState<'a> {
  name: &'a str,
  parent_address_cells: u32,
  parent_size_cells: u32,
  child_address_cells: u32,
  child_size_cells: u32,
  device_type: Option<&'a str>,
  status: Option<&'a str>,
  reg: Option<&'a [u8]>,
}

impl<'a> NodeState<'a> {
  const EMPTY: Self = Self {
    name: "",
    parent_address_cells: DEFAULT_ADDRESS_CELLS,
    parent_size_cells: DEFAULT_SIZE_CELLS,
    child_address_cells: DEFAULT_ADDRESS_CELLS,
    child_size_cells: DEFAULT_SIZE_CELLS,
    device_type: None,
    status: None,
    reg: None,
  };

  const fn new(name: &'a str, parent_address_cells: u32, parent_size_cells: u32) -> Self {
    Self {
      name,
      parent_address_cells,
      parent_size_cells,
      child_address_cells: DEFAULT_ADDRESS_CELLS,
      child_size_cells: DEFAULT_SIZE_CELLS,
      device_type: None,
      status: None,
      reg: None,
    }
  }

  fn enabled(self) -> bool {
    matches!(self.status, None | Some("ok") | Some("okay"))
  }
}

pub fn discover(dtb: &Dtb<'_>, boot_hart: usize) -> Result<PlatformInfo, Error> {
  let mut info = PlatformInfo::new();
  let mut stack = [NodeState::EMPTY; MAX_DTB_DEPTH];
  let mut stack_len = 0usize;

  for event in dtb.events() {
    match event? {
      Event::BeginNode { depth, name } => {
        if depth != stack_len || depth == MAX_DTB_DEPTH {
          return Err(Error::InvalidDepth);
        }

        let (parent_address_cells, parent_size_cells) = if depth == 0 {
          (DEFAULT_ADDRESS_CELLS, DEFAULT_SIZE_CELLS)
        } else {
          let parent = stack[depth - 1];
          (parent.child_address_cells, parent.child_size_cells)
        };

        stack[depth] = NodeState::new(name, parent_address_cells, parent_size_cells);
        stack_len += 1;
      }
      Event::Property { depth, name, value } => {
        if depth >= stack_len || depth + 1 != stack_len {
          return Err(Error::InvalidDepth);
        }

        let node = &mut stack[depth];
        match name {
          "#address-cells" => {
            node.child_address_cells = read_single_cell(value)?;
          }
          "#size-cells" => {
            node.child_size_cells = read_single_cell(value)?;
          }
          "device_type" => node.device_type = Some(read_string(value)?),
          "status" => node.status = Some(read_string(value)?),
          "reg" => node.reg = Some(value),
          _ => {}
        }
      }
      Event::EndNode { depth } => {
        if stack_len != depth + 1 {
          return Err(Error::InvalidDepth);
        }

        let node = stack[depth];
        let parent_name = if depth == 0 {
          None
        } else {
          Some(stack[depth - 1].name)
        };

        finish_node(&mut info, depth, parent_name, node)?;
        stack[depth] = NodeState::EMPTY;
        stack_len -= 1;
      }
      Event::End => {
        if stack_len != 0 {
          return Err(Error::InvalidDepth);
        }
        break;
      }
    }
  }

  for reservation in dtb.reservations() {
    let reservation = reservation?;
    info.add_reserved_region(MemoryRegion {
      base: reservation.address,
      size: reservation.size,
    })?;
  }

  if info.hart_index(boot_hart).is_none() {
    return Err(Error::MissingBootHart);
  }

  if info.memory_regions().is_empty() {
    return Err(Error::InvalidMemoryRegion);
  }

  Ok(info)
}

fn finish_node(
  info: &mut PlatformInfo,
  depth: usize,
  parent_name: Option<&str>,
  node: NodeState<'_>,
) -> Result<(), Error> {
  let is_cpu = depth == 2 && parent_name == Some("cpus") && node.device_type == Some("cpu");
  if is_cpu && node.enabled() {
    let reg = node.reg.ok_or(Error::InvalidProperty)?;
    if node.parent_size_cells != 0 {
      return Err(Error::InvalidProperty);
    }

    let mut cursor = 0usize;
    let hart_id = read_cells(reg, &mut cursor, node.parent_address_cells)?;
    if cursor != reg.len() {
      return Err(Error::InvalidProperty);
    }

    let hart_id = usize::try_from(hart_id).map_err(|_| Error::InvalidProperty)?;

    info.add_hart(hart_id)?;
  }

  let is_memory = depth == 1 && node.device_type == Some("memory");
  if is_memory && node.enabled() {
    let reg = node.reg.ok_or(Error::InvalidProperty)?;
    parse_reg_regions(node, reg, |region| info.add_memory_region(region))?;
  }

  let is_reserved_memory = depth == 2 && parent_name == Some("reserved-memory");
  if is_reserved_memory && node.enabled() {
    let reg = node.reg.ok_or(Error::InvalidProperty)?;
    parse_reg_regions(node, reg, |region| info.add_reserved_region(region))?;
  }

  Ok(())
}

fn parse_reg_regions(
  node: NodeState<'_>,
  reg: &[u8],
  mut add: impl FnMut(MemoryRegion) -> Result<(), Error>,
) -> Result<(), Error> {
  validate_cell_count(node.parent_address_cells)?;
  validate_cell_count(node.parent_size_cells)?;

  let cells_per_entry = node
    .parent_address_cells
    .checked_add(node.parent_size_cells)
    .ok_or(Error::InvalidProperty)? as usize;
  let bytes_per_entry = cells_per_entry
    .checked_mul(4)
    .ok_or(Error::InvalidProperty)?;

  if bytes_per_entry == 0 || reg.is_empty() || !reg.len().is_multiple_of(bytes_per_entry) {
    return Err(Error::InvalidProperty);
  }

  let mut cursor = 0usize;
  while cursor < reg.len() {
    let base = read_cells(reg, &mut cursor, node.parent_address_cells)?;
    let size = read_cells(reg, &mut cursor, node.parent_size_cells)?;
    add(MemoryRegion { base, size })?;
  }

  Ok(())
}

fn validate_cell_count(cells: u32) -> Result<(), Error> {
  if cells > MAX_SUPPORTED_CELLS {
    return Err(Error::UnsupportedCellCount);
  }
  Ok(())
}

fn read_single_cell(value: &[u8]) -> Result<u32, Error> {
  if value.len() != 4 {
    return Err(Error::InvalidProperty);
  }
  Ok(u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_cells(value: &[u8], cursor: &mut usize, cells: u32) -> Result<u64, Error> {
  validate_cell_count(cells)?;

  let mut result = 0u64;
  for _ in 0..cells {
    let end = cursor.checked_add(4).ok_or(Error::InvalidProperty)?;
    let cell = value.get(*cursor..end).ok_or(Error::InvalidProperty)?;
    let cell = u32::from_be_bytes([cell[0], cell[1], cell[2], cell[3]]);
    result = (result << 32) | u64::from(cell);
    *cursor = end;
  }
  Ok(result)
}

fn read_string(value: &[u8]) -> Result<&str, Error> {
  let terminator = value
    .iter()
    .position(|byte| *byte == 0)
    .ok_or(Error::InvalidProperty)?;

  if terminator + 1 != value.len() {
    return Err(Error::InvalidProperty);
  }

  str::from_utf8(&value[..terminator]).map_err(|_| Error::InvalidProperty)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn plat_004_sparse_hart_ids_receive_dense_indices() {
    let mut info = PlatformInfo::new();
    info.add_hart(2).unwrap();
    info.add_hart(17).unwrap();
    assert_eq!(info.hart_index(2), Some(0));
    assert_eq!(info.hart_index(17), Some(1));
    assert_eq!(info.hart_index(3), None);
  }

  #[test]
  fn plat_005_duplicate_hart_ids_are_rejected() {
    let mut info = PlatformInfo::new();
    info.add_hart(4).unwrap();
    assert_eq!(info.add_hart(4), Err(Error::DuplicateHart));
  }

  #[test]
  fn plat_008_cells_are_read_as_big_endian() {
    let bytes = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0];
    let mut cursor = 0;
    assert_eq!(
      read_cells(&bytes, &mut cursor, 2),
      Ok(0x1234_5678_9abc_def0)
    );
    assert_eq!(cursor, bytes.len());
  }

  #[test]
  fn plat_009_unsupported_cell_count_is_rejected() {
    let mut cursor = 0;
    assert_eq!(
      read_cells(&[0; 12], &mut cursor, 3),
      Err(Error::UnsupportedCellCount)
    );
  }

  #[test]
  fn plat_011_zero_sized_memory_is_rejected() {
    let mut info = PlatformInfo::new();
    assert_eq!(
      info.add_memory_region(MemoryRegion {
        base: 0x8000_0000,
        size: 0,
      }),
      Err(Error::InvalidMemoryRegion)
    );
  }

  #[test]
  fn plat_012_memory_end_overflow_is_rejected() {
    let mut info = PlatformInfo::new();
    assert_eq!(
      info.add_memory_region(MemoryRegion {
        base: u64::MAX - 0xff,
        size: 0x100,
      }),
      Err(Error::InvalidMemoryRegion)
    );
  }

  #[test]
  fn plat_013_overlapping_memory_is_rejected() {
    let mut info = PlatformInfo::new();
    info
      .add_memory_region(MemoryRegion {
        base: 0x8000_0000,
        size: 0x2000,
      })
      .unwrap();
    assert_eq!(
      info.add_memory_region(MemoryRegion {
        base: 0x8000_1000,
        size: 0x2000,
      }),
      Err(Error::OverlappingMemoryRegions)
    );
  }

  #[test]
  fn plat_014_adjacent_memory_is_not_an_overlap() {
    let mut info = PlatformInfo::new();
    info
      .add_memory_region(MemoryRegion {
        base: 0x8000_0000,
        size: 0x1000,
      })
      .unwrap();
    assert!(
      info
        .add_memory_region(MemoryRegion {
          base: 0x8000_1000,
          size: 0x1000,
        })
        .is_ok()
    );
  }

  #[test]
  fn plat_003_enabled_status_values_follow_the_device_tree_contract() {
    for status in [None, Some("ok"), Some("okay")] {
      let node = NodeState {
        status,
        ..NodeState::EMPTY
      };
      assert!(node.enabled());
    }
    let disabled = NodeState {
      status: Some("disabled"),
      ..NodeState::EMPTY
    };
    assert!(!disabled.enabled());
  }
}
