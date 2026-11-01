use core::{ptr, slice, str};

// FDT header
//
// | Offset | Field | Meaning |
// | ---: | --- | --- |
// | `0x00` | `magic` | `0xd00dfeed` |
// | `0x04` | `totalsize` | total number of bytes in the DTB |
// | `0x08` | `off_dt_struct` | offset of the structure block |
// | `0x0c` | `off_dt_strings` | offset of the strings block |
// | `0x10` | `off_mem_rsvmap` | offset of the memory reservation |
// | `0x14` | `version` | FDT format version |
// | `0x18` | `last_comp_version` | backward-compatible version |
// | `0x1c` | `boot_cpuid_phys` | physical ID of the boot CPU |
// | `0x20` | `size_dt_strings` | number of bytes in the strings block |
// | `0x24` | `size_dt_struct` | number of bytes in the structure block |

const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_HEADER_SIZE: usize = 40;
const FDT_SUPPORTED_VERSION: u32 = 17;

// Structure block tokens
//
// | Token | Value | Extra data |
// | --- | ---: | --- |
// | `FDT_BEGIN_NODE` | `1` | null-terminated node nameとpadding |
// | `FDT_END_NODE` | `2` | none |
// | `FDT_PROP` | `3` | value length、name offset、value、padding |
// | `FDT_NOP` | `4` | none |
// | `FDT_END` | `9` | none |

const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

// A corrupted header must not make the kernel construct an arbitrarily large slice.
// This limit is intentionally conservative for the bootstrap parser.
const MAX_DTB_SIZE: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  NullAddress,
  UnalignedAddress,
  BlobTooSmall,
  BlobTooLarge,
  BadMagic,
  UnsupportedVersion,
  InvalidAlignment,
  InvalidRange,
  InvalidUtf8,
  MissingNullTerminator,
  UnknownToken,
  UnexpectedEndNode,
  MissingEndToken,
  MissingReservationTerminator,
  TrailingStructureData,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::NullAddress => "null DTB address",
      Self::UnalignedAddress => "unaligned DTB address",
      Self::BlobTooSmall => "DTB is smaller than its header",
      Self::BlobTooLarge => "DTB exceeds the bootstrap size limit",
      Self::BadMagic => "invalid DTB magic",
      Self::UnsupportedVersion => "unsupported DTB version",
      Self::InvalidAlignment => "invalid DTB block alignment",
      Self::InvalidRange => "DTB range is outside the blob",
      Self::InvalidUtf8 => "DTB name is not valid UTF-8",
      Self::MissingNullTerminator => "DTB string has no terminator",
      Self::MissingReservationTerminator => "missing FDT reservation terminator",
      Self::UnknownToken => "unknown DTB structure token",
      Self::UnexpectedEndNode => "unexpected FDT_END_NODE",
      Self::MissingEndToken => "missing FDT_END token",
      Self::TrailingStructureData => "data exists after FDT_END",
    }
  }
}

#[derive(Clone, Copy, Debug)]
pub struct Header {
  pub total_size: usize,
  pub version: u32,
  pub boot_cpu_id: u32,
}

pub struct Dtb<'a> {
  header: Header,
  structure: &'a [u8],
  strings: &'a [u8],
  reservations: &'a [u8],
}

impl<'a> Dtb<'a> {
  /// Creates a bounded DTB view from the physical address supplied by OpenSBI.
  ///
  /// # Safety
  ///
  /// `address` must point to readable physical memory containing at least
  /// an FDT header. That memory must remain readable and immutable for `'a`.
  /// Before paging is enabled, Pluvora directly addresses this physical memory.
  pub unsafe fn from_ptr(address: usize) -> Result<Self, Error> {
    if address == 0 {
      return Err(Error::NullAddress);
    }

    // DTSpec requires the DTB to be placed at an 8 byte aligned address.
    if !address.is_multiple_of(8) {
      return Err(Error::UnalignedAddress);
    }

    // Read only the fixed header first.
    let mut raw_header = [0u8; FDT_HEADER_SIZE];
    for (offset, byte) in raw_header.iter_mut().enumerate() {
      // SAFETY: the caller guarantees that at least
      // the fixed size FDT header is readable at `address`
      *byte = unsafe { ptr::read_volatile((address as *const u8).add(offset)) };
    }

    let total_size = read_be_u32(&raw_header, 4)? as usize;
    if total_size < FDT_HEADER_SIZE {
      return Err(Error::BlobTooSmall);
    }
    if total_size > MAX_DTB_SIZE {
      return Err(Error::BlobTooLarge);
    }

    // SAFETY: the caller guarantees that `address` refers to the DTB.
    let blob = unsafe { slice::from_raw_parts(address as *const u8, total_size) };
    Self::from_bytes(blob)
  }

  pub fn from_bytes(blob: &'a [u8]) -> Result<Self, Error> {
    if blob.len() < FDT_HEADER_SIZE {
      return Err(Error::BlobTooSmall);
    }
    if blob.len() > MAX_DTB_SIZE {
      return Err(Error::BlobTooLarge);
    }

    let magic = read_be_u32(blob, 0)?;
    if magic != FDT_MAGIC {
      return Err(Error::BadMagic);
    }

    let total_size = read_be_u32(blob, 4)? as usize;
    let structure_offset = read_be_u32(blob, 8)? as usize;
    let strings_offset = read_be_u32(blob, 12)? as usize;
    let reservation_offset = read_be_u32(blob, 16)? as usize;
    let version = read_be_u32(blob, 20)?;
    let last_compatible_version = read_be_u32(blob, 24)?;
    let boot_cpu_id = read_be_u32(blob, 28)?;
    let strings_size = read_be_u32(blob, 32)? as usize;
    let structure_size = read_be_u32(blob, 36)? as usize;

    if total_size != blob.len() {
      return Err(Error::InvalidRange);
    }

    if version < FDT_SUPPORTED_VERSION || last_compatible_version > FDT_SUPPORTED_VERSION {
      return Err(Error::UnsupportedVersion);
    }

    if !structure_offset.is_multiple_of(4) || !reservation_offset.is_multiple_of(8) {
      return Err(Error::InvalidAlignment);
    }

    let structure = checked_range(blob, structure_offset, structure_size)?;
    let strings = checked_range(blob, strings_offset, strings_size)?;
    let reservations = blob.get(reservation_offset..).ok_or(Error::InvalidRange)?;

    if reservations.len() < 16 {
      return Err(Error::InvalidRange);
    }

    let header = Header {
      total_size,
      version,
      boot_cpu_id,
    };

    let dtb = Self {
      header,
      structure,
      strings,
      reservations,
    };

    // Validate that a bounded terminator exists during construction.
    for reservation in dtb.reservations() {
      reservation?;
    }

    Ok(dtb)
  }

  pub const fn header(&self) -> &Header {
    &self.header
  }

  pub fn events(&self) -> Events<'a> {
    Events {
      structure: self.structure,
      strings: self.strings,
      cursor: 0,
      depth: 0,
      saw_root: false,
      root_closed: false,
      finished: false,
    }
  }
}

#[derive(Clone, Copy, Debug)]
pub enum Event<'a> {
  BeginNode {
    depth: usize,
    name: &'a str,
  },
  Property {
    depth: usize,
    name: &'a str,
    value: &'a [u8],
  },
  EndNode {
    depth: usize,
  },
  End,
}

pub struct Events<'a> {
  structure: &'a [u8],
  strings: &'a [u8],
  cursor: usize,
  depth: usize,
  saw_root: bool,
  root_closed: bool,
  finished: bool,
}

impl<'a> Events<'a> {
  fn parse_next(&mut self) -> Result<Event<'a>, Error> {
    loop {
      if self.cursor == self.structure.len() {
        return Err(Error::MissingEndToken);
      }

      let token = take_be_u32(self.structure, &mut self.cursor)?;
      match token {
        FDT_BEGIN_NODE => {
          if self.depth == 0 {
            if self.saw_root || self.root_closed {
              return Err(Error::InvalidRange);
            }
            self.saw_root = true;
          }

          let name_start = self.cursor;
          let name_bytes = take_c_string(self.structure, &mut self.cursor)?;
          self.cursor = align_up_4(self.cursor)?;
          if self.cursor > self.structure.len() {
            return Err(Error::InvalidRange);
          }

          let name = str::from_utf8(name_bytes).map_err(|_| Error::InvalidUtf8)?;
          let depth = self.depth;
          self.depth = self.depth.checked_add(1).ok_or(Error::InvalidRange)?;

          // A BEGIN_NODE name must come from the structure block at
          // the cursor position just consumed. Keep this assertion as
          // a local parser invariant without trusting external data.
          if name_start >= self.structure.len() {
            return Err(Error::InvalidRange);
          }

          return Ok(Event::BeginNode { depth, name });
        }
        FDT_END_NODE => {
          self.depth = self.depth.checked_sub(1).ok_or(Error::UnexpectedEndNode)?;
          if self.depth == 0 {
            self.root_closed = true;
          }
          return Ok(Event::EndNode { depth: self.depth });
        }
        FDT_PROP => {
          if self.depth == 0 {
            return Err(Error::InvalidRange);
          }

          let value_size = take_be_u32(self.structure, &mut self.cursor)? as usize;
          let name_offset = take_be_u32(self.structure, &mut self.cursor)? as usize;

          let value_end = self
            .cursor
            .checked_add(value_size)
            .ok_or(Error::InvalidRange)?;
          let value = self
            .structure
            .get(self.cursor..value_end)
            .ok_or(Error::InvalidRange)?;
          self.cursor = align_up_4(value_end)?;
          if self.cursor > self.structure.len() {
            return Err(Error::InvalidRange);
          }

          let mut name_cursor = name_offset;
          let name_bytes = take_c_string(self.strings, &mut name_cursor)?;
          let name = str::from_utf8(name_bytes).map_err(|_| Error::InvalidUtf8)?;

          return Ok(Event::Property {
            depth: self.depth - 1,
            name,
            value,
          });
        }
        FDT_NOP => continue,
        FDT_END => {
          if self.depth != 0 || !self.root_closed {
            return Err(Error::MissingEndToken);
          }
          if self.cursor != self.structure.len() {
            return Err(Error::TrailingStructureData);
          }
          return Ok(Event::End);
        }
        _ => return Err(Error::UnknownToken),
      }
    }
  }
}

impl<'a> Iterator for Events<'a> {
  type Item = Result<Event<'a>, Error>;

  fn next(&mut self) -> Option<Self::Item> {
    if self.finished {
      return None;
    }

    let event = self.parse_next();
    if matches!(&event, Ok(Event::End) | Err(_)) {
      self.finished = true;
    }
    Some(event)
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryReservation {
  pub address: u64,
  pub size: u64,
}

pub struct Reservations<'a> {
  bytes: &'a [u8],
  cursor: usize,
  finished: bool,
}

impl Dtb<'_> {
  pub fn reservations(&self) -> Reservations<'_> {
    Reservations {
      bytes: self.reservations,
      cursor: 0,
      finished: false,
    }
  }
}

impl Iterator for Reservations<'_> {
  type Item = Result<MemoryReservation, Error>;

  fn next(&mut self) -> Option<Self::Item> {
    if self.finished {
      return None;
    }

    let end = match self.cursor.checked_add(16) {
      Some(end) if end <= self.bytes.len() => end,
      _ => {
        self.finished = true;
        return Some(Err(Error::MissingReservationTerminator));
      }
    };

    let address = match read_be_u64(self.bytes, self.cursor) {
      Ok(address) => address,
      Err(error) => {
        self.finished = true;
        return Some(Err(error));
      }
    };
    let size = match read_be_u64(self.bytes, self.cursor + 8) {
      Ok(size) => size,
      Err(error) => {
        self.finished = true;
        return Some(Err(error));
      }
    };
    self.cursor = end;

    if address == 0 && size == 0 {
      self.finished = true;
      return None;
    }

    if size == 0 || address.checked_add(size).is_none() {
      self.finished = true;
      return Some(Err(Error::InvalidRange));
    }

    Some(Ok(MemoryReservation { address, size }))
  }
}

fn read_be_u64(bytes: &[u8], offset: usize) -> Result<u64, Error> {
  let end = offset.checked_add(8).ok_or(Error::InvalidRange)?;
  let raw = bytes.get(offset..end).ok_or(Error::InvalidRange)?;
  Ok(u64::from_be_bytes([
    raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
  ]))
}

fn read_be_u32(bytes: &[u8], offset: usize) -> Result<u32, Error> {
  let end = offset.checked_add(4).ok_or(Error::InvalidRange)?;
  let raw = bytes.get(offset..end).ok_or(Error::InvalidRange)?;
  Ok(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn take_be_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, Error> {
  let value = read_be_u32(bytes, *cursor)?;
  *cursor = cursor.checked_add(4).ok_or(Error::InvalidRange)?;
  Ok(value)
}

fn take_c_string<'a>(bytes: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], Error> {
  let rest = bytes.get(*cursor..).ok_or(Error::InvalidRange)?;
  let length = rest
    .iter()
    .position(|byte| *byte == 0)
    .ok_or(Error::MissingNullTerminator)?;
  let end = cursor.checked_add(length).ok_or(Error::InvalidRange)?;
  let value = bytes.get(*cursor..end).ok_or(Error::InvalidRange)?;
  *cursor = end.checked_add(1).ok_or(Error::InvalidRange)?;
  Ok(value)
}

fn checked_range(bytes: &[u8], offset: usize, size: usize) -> Result<&[u8], Error> {
  let end = offset.checked_add(size).ok_or(Error::InvalidRange)?;
  bytes.get(offset..end).ok_or(Error::InvalidRange)
}

fn align_up_4(value: usize) -> Result<usize, Error> {
  value
    .checked_add(3)
    .map(|value| value & !3)
    .ok_or(Error::InvalidRange)
}

#[cfg(test)]
mod tests {
  use std::vec::Vec;

  use super::*;

  const HEADER_SIZE: usize = 40;

  fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
  }

  fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
  }

  fn minimal_dtb(reservations: &[(u64, u64)]) -> Vec<u8> {
    let mut reservation_block = Vec::new();
    for (address, size) in reservations {
      push_u64(&mut reservation_block, *address);
      push_u64(&mut reservation_block, *size);
    }
    push_u64(&mut reservation_block, 0);
    push_u64(&mut reservation_block, 0);

    let mut structure = Vec::new();
    push_u32(&mut structure, FDT_BEGIN_NODE);
    structure.extend_from_slice(&[0, 0, 0, 0]);
    push_u32(&mut structure, FDT_END_NODE);
    push_u32(&mut structure, FDT_END);

    let reservation_offset = HEADER_SIZE;
    let structure_offset = reservation_offset + reservation_block.len();
    let strings_offset = structure_offset + structure.len();
    let total_size = strings_offset;

    let mut blob = Vec::with_capacity(total_size);
    push_u32(&mut blob, FDT_MAGIC);
    push_u32(&mut blob, total_size as u32);
    push_u32(&mut blob, structure_offset as u32);
    push_u32(&mut blob, strings_offset as u32);
    push_u32(&mut blob, reservation_offset as u32);
    push_u32(&mut blob, FDT_SUPPORTED_VERSION);
    push_u32(&mut blob, 16);
    push_u32(&mut blob, 0);
    push_u32(&mut blob, 0);
    push_u32(&mut blob, structure.len() as u32);
    blob.extend_from_slice(&reservation_block);
    blob.extend_from_slice(&structure);
    blob
  }

  #[test]
  fn dtb_001_valid_minimal_blob_is_accepted() {
    let blob = minimal_dtb(&[]);
    let dtb = Dtb::from_bytes(&blob).unwrap();
    assert_eq!(dtb.header().total_size, blob.len());
    assert_eq!(dtb.header().version, FDT_SUPPORTED_VERSION);

    let events = dtb.events().collect::<Result<Vec<_>, _>>().unwrap();
    assert!(matches!(
      events.as_slice(),
      [
        Event::BeginNode { depth: 0, name: "" },
        Event::EndNode { depth: 0 },
        Event::End
      ]
    ));
  }

  #[test]
  fn dtb_002_bad_magic_is_rejected() {
    let mut blob = minimal_dtb(&[]);
    blob[0] = 0;
    assert!(matches!(Dtb::from_bytes(&blob), Err(Error::BadMagic)));
  }

  #[test]
  fn dtb_003_unsupported_version_is_rejected() {
    let mut blob = minimal_dtb(&[]);
    blob[20..24].copy_from_slice(&16u32.to_be_bytes());
    assert!(matches!(
      Dtb::from_bytes(&blob),
      Err(Error::UnsupportedVersion)
    ));
  }

  #[test]
  fn dtb_004_short_blob_is_rejected() {
    assert!(matches!(
      Dtb::from_bytes(&[0; FDT_HEADER_SIZE - 1]),
      Err(Error::BlobTooSmall)
    ));
  }

  #[test]
  fn dtb_005_total_size_must_match_the_bounded_blob() {
    let mut blob = minimal_dtb(&[]);
    let incorrect = (blob.len() as u32 + 1).to_be_bytes();
    blob[4..8].copy_from_slice(&incorrect);
    assert!(matches!(Dtb::from_bytes(&blob), Err(Error::InvalidRange)));
  }

  #[test]
  fn dtb_009_unaligned_structure_offset_is_rejected() {
    let mut blob = minimal_dtb(&[]);
    blob[8..12].copy_from_slice(&57u32.to_be_bytes());
    assert!(matches!(
      Dtb::from_bytes(&blob),
      Err(Error::InvalidAlignment)
    ));
  }

  #[test]
  fn dtb_013_unknown_structure_token_is_rejected() {
    let mut blob = minimal_dtb(&[]);
    let structure_offset = read_be_u32(&blob, 8).unwrap() as usize;
    blob[structure_offset..structure_offset + 4].copy_from_slice(&0xffu32.to_be_bytes());
    let dtb = Dtb::from_bytes(&blob).unwrap();
    assert!(matches!(
      dtb.events().next(),
      Some(Err(Error::UnknownToken))
    ));
  }

  #[test]
  fn dtb_018_missing_reservation_terminator_is_rejected() {
    let mut blob = minimal_dtb(&[(0x8000_0000, 0x1000)]);
    blob[56..64].copy_from_slice(&1u64.to_be_bytes());
    blob[64..72].copy_from_slice(&1u64.to_be_bytes());
    assert!(matches!(
      Dtb::from_bytes(&blob),
      Err(Error::MissingReservationTerminator) | Err(Error::InvalidRange)
    ));
  }

  #[test]
  fn dtb_019_reservation_terminator_is_not_returned_as_data() {
    let blob = minimal_dtb(&[(0x8000_0000, 0x2000)]);
    let dtb = Dtb::from_bytes(&blob).unwrap();
    let reservations = dtb.reservations().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(
      reservations,
      [MemoryReservation {
        address: 0x8000_0000,
        size: 0x2000,
      }]
    );
  }

  #[test]
  fn dtb_020_iterator_stops_after_an_error() {
    let mut blob = minimal_dtb(&[]);
    let structure_offset = read_be_u32(&blob, 8).unwrap() as usize;
    blob[structure_offset..structure_offset + 4].copy_from_slice(&0xffu32.to_be_bytes());
    let dtb = Dtb::from_bytes(&blob).unwrap();
    let mut events = dtb.events();
    assert!(events.next().unwrap().is_err());
    assert!(events.next().is_none());
  }
}
