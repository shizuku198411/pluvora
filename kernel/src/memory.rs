use core::ptr;

use crate::platform::{MemoryRegion, PlatformInfo};

pub const PAGE_SIZE: u64 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  InvalidRange,
  AddressNotRepresentable,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::InvalidRange => "invalid early allocator range",
      Self::AddressNotRepresentable => "physical address does not fit usize",
    }
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalFrame {
  start: u64,
}

impl PhysicalFrame {
  pub const fn start(self) -> u64 {
    self.start
  }
}

pub struct EarlyFrameAllocator<'a> {
  memory: &'a [MemoryRegion],
  reserved: &'a [MemoryRegion],
  kernel: MemoryRegion,
  dtb: MemoryRegion,
  region_index: usize,
  next_candidate: Option<u64>,
}

impl<'a> EarlyFrameAllocator<'a> {
  pub fn new(
    platform: &'a PlatformInfo,
    kernel: MemoryRegion,
    dtb: MemoryRegion,
  ) -> Result<Self, Error> {
    validate_region(kernel)?;
    validate_region(dtb)?;

    if platform.memory_regions().is_empty() {
      return Err(Error::InvalidRange);
    }

    Ok(Self {
      memory: platform.memory_regions(),
      reserved: platform.reserved_regions(),
      kernel,
      dtb,
      region_index: 0,
      next_candidate: None,
    })
  }

  pub fn allocate_zeroed(&mut self) -> Result<Option<PhysicalFrame>, Error> {
    loop {
      let Some(memory) = self.memory.get(self.region_index).copied() else {
        return Ok(None);
      };
      let memory_end = memory.end().ok_or(Error::InvalidRange)?;

      let candidate = match self.next_candidate {
        Some(candidate) => candidate,
        None => align_up(memory.base, PAGE_SIZE)?,
      };
      let candidate_end = candidate
        .checked_add(PAGE_SIZE)
        .ok_or(Error::InvalidRange)?;

      if candidate_end > memory_end {
        self.region_index += 1;
        self.next_candidate = None;
        continue;
      }

      let page = MemoryRegion {
        base: candidate,
        size: PAGE_SIZE,
      };

      if let Some(blocking_end) = self.blocking_reservation_end(page)? {
        self.next_candidate = Some(align_up(blocking_end, PAGE_SIZE)?);
        continue;
      }

      self.next_candidate = Some(candidate_end);
      let address = usize::try_from(candidate).map_err(|_| Error::AddressNotRepresentable)?;

      // SAFETY: `candidate` lies inside DTB-reported RAM and does not overlap
      // firmware, kernel, DTB, or another reserved region. Paging is not yet
      // enabled, so the physical address is directly accessible.
      unsafe { ptr::write_bytes(address as *mut u8, 0, PAGE_SIZE as usize) };

      return Ok(Some(PhysicalFrame { start: candidate }));
    }
  }

  fn blocking_reservation_end(&self, page: MemoryRegion) -> Result<Option<u64>, Error> {
    let mut blocking_end = None;
    for reservation in self.reserved.iter().copied().chain([self.kernel, self.dtb]) {
      validate_region(reservation)?;
      if overlaps(page, reservation)? {
        let end = reservation.end().ok_or(Error::InvalidRange)?;
        blocking_end = Some(blocking_end.map_or(end, |current: u64| current.max(end)));
      }
    }

    Ok(blocking_end)
  }
}

fn validate_region(region: MemoryRegion) -> Result<(), Error> {
  if region.size == 0 || region.end().is_none() {
    return Err(Error::InvalidRange);
  }
  Ok(())
}

fn overlaps(left: MemoryRegion, right: MemoryRegion) -> Result<bool, Error> {
  let left_end = left.end().ok_or(Error::InvalidRange)?;
  let right_end = right.end().ok_or(Error::InvalidRange)?;
  Ok(left.base < right_end && right.base < left_end)
}

fn align_up(value: u64, alignment: u64) -> Result<u64, Error> {
  debug_assert!(alignment.is_power_of_two());
  value
    .checked_add(alignment - 1)
    .map(|value| value & !(alignment - 1))
    .ok_or(Error::InvalidRange)
}
