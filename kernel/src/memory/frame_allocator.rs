use core::ptr;

use super::{EarlyFrameAllocator, PAGE_SIZE};

#[cfg(target_arch = "riscv64")]
use crate::sync::SpinLock;

pub const MAX_RUNTIME_FRAMES: usize = 65_536;

const STATE_UNUSED: u8 = 0;
const STATE_FREE: u8 = 1;
const STATE_ALLOCATED: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  AlreadyInitialized,
  Uninitialized,
  TooManyFrames,
  InvalidFrameCount,
  OutOfMemory,
  AddressNotRepresentable,
  ForeignBlock,
  DoubleFree,
  Memory(super::Error),
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::AlreadyInitialized => "runtime frame allocator is already initialized",
      Self::Uninitialized => "runtime frame allocator is not initialized",
      Self::TooManyFrames => "physical memory exceeds the runtime frame tracking limit",
      Self::InvalidFrameCount => "runtime frame count is invalid",
      Self::OutOfMemory => "no contiguous runtime frame block is available",
      Self::AddressNotRepresentable => "physical frame address does not fit usize",
      Self::ForeignBlock => "frame block is not managed by the runtime allocator",
      Self::DoubleFree => "frame block is not currently allocated",
      Self::Memory(error) => error.as_str(),
    }
  }
}

impl From<super::Error> for Error {
  fn from(error: super::Error) -> Self {
    Self::Memory(error)
  }
}

#[derive(Debug, Eq, PartialEq)]
pub struct FrameBlock {
  start: u64,
  frames: usize,
}

impl FrameBlock {
  pub const fn start(&self) -> u64 {
    self.start
  }

  pub const fn frames(&self) -> usize {
    self.frames
  }

  pub fn bytes(&self) -> Option<u64> {
    (self.frames as u64).checked_mul(PAGE_SIZE)
  }
}

#[derive(Clone, Copy)]
struct FrameSlot {
  address: u64,
  state: u8,
}

impl FrameSlot {
  const fn unused() -> Self {
    Self {
      address: 0,
      state: STATE_UNUSED,
    }
  }
}

pub struct FrameAllocator<const N: usize> {
  slots: [FrameSlot; N],
  len: usize,
  free: usize,
  initialized: bool,
}

impl<const N: usize> FrameAllocator<N> {
  pub const fn uninitialized() -> Self {
    Self {
      slots: [FrameSlot::unused(); N],
      len: 0,
      free: 0,
      initialized: false,
    }
  }

  pub fn initialize(&mut self, early: &EarlyFrameAllocator<'_>) -> Result<(), Error> {
    if self.initialized {
      return Err(Error::AlreadyInitialized);
    }

    let mut len = 0usize;
    let mut free = 0usize;
    for region in early.memory_regions() {
      let end = region.end().ok_or(super::Error::InvalidRange)?;
      let mut address = align_up(region.base)?;
      while address
        .checked_add(PAGE_SIZE)
        .is_some_and(|frame_end| frame_end <= end)
      {
        if len == N {
          return Err(Error::TooManyFrames);
        }
        let state = if early.runtime_eligible(address)? {
          free = free.checked_add(1).ok_or(Error::TooManyFrames)?;
          STATE_FREE
        } else {
          STATE_UNUSED
        };
        self.slots[len] = FrameSlot { address, state };
        len += 1;
        address = address
          .checked_add(PAGE_SIZE)
          .ok_or(super::Error::InvalidRange)?;
      }
    }

    self.len = len;
    self.free = free;
    self.initialized = true;
    Ok(())
  }

  pub const fn free_frames(&self) -> Result<usize, Error> {
    if !self.initialized {
      return Err(Error::Uninitialized);
    }
    Ok(self.free)
  }

  pub fn allocate(&mut self, frames: usize) -> Result<FrameBlock, Error> {
    self.require_initialized()?;
    if frames == 0 || frames > N {
      return Err(Error::InvalidFrameCount);
    }

    let start_index = self.find_contiguous(frames).ok_or(Error::OutOfMemory)?;
    let start = self.slots[start_index].address;
    for slot in &mut self.slots[start_index..start_index + frames] {
      slot.state = STATE_ALLOCATED;
    }
    self.free = self.free.checked_sub(frames).ok_or(Error::OutOfMemory)?;

    if let Err(error) = zero_block(start, frames) {
      for slot in &mut self.slots[start_index..start_index + frames] {
        slot.state = STATE_FREE;
      }
      self.free += frames;
      return Err(error);
    }

    Ok(FrameBlock { start, frames })
  }

  pub fn release(&mut self, block: FrameBlock) -> Result<(), Error> {
    self.require_initialized()?;
    if block.frames == 0 {
      return Err(Error::InvalidFrameCount);
    }
    let Some(start_index) = self.slots[..self.len]
      .iter()
      .position(|slot| slot.address == block.start)
    else {
      return Err(Error::ForeignBlock);
    };
    let Some(end_index) = start_index.checked_add(block.frames) else {
      return Err(Error::ForeignBlock);
    };
    if end_index > self.len {
      return Err(Error::ForeignBlock);
    }

    for (offset, slot) in self.slots[start_index..end_index].iter().enumerate() {
      let expected = block
        .start
        .checked_add((offset as u64) * PAGE_SIZE)
        .ok_or(Error::ForeignBlock)?;
      if slot.address != expected {
        return Err(Error::ForeignBlock);
      }
      if slot.state != STATE_ALLOCATED {
        return Err(Error::DoubleFree);
      }
    }

    for slot in &mut self.slots[start_index..end_index] {
      slot.state = STATE_FREE;
    }
    self.free = self
      .free
      .checked_add(block.frames)
      .ok_or(Error::TooManyFrames)?;
    Ok(())
  }

  fn find_contiguous(&self, frames: usize) -> Option<usize> {
    let mut run_start = 0usize;
    let mut run_len = 0usize;
    let mut previous = None;

    for (index, slot) in self.slots[..self.len].iter().enumerate() {
      let contiguous =
        previous.is_some_and(|address: u64| address.checked_add(PAGE_SIZE) == Some(slot.address));
      if slot.state == STATE_FREE {
        if run_len == 0 || !contiguous {
          run_start = index;
          run_len = 1;
        } else {
          run_len += 1;
        }
        if run_len == frames {
          return Some(run_start);
        }
      } else {
        run_len = 0;
      }
      previous = Some(slot.address);
    }
    None
  }

  const fn require_initialized(&self) -> Result<(), Error> {
    if self.initialized {
      Ok(())
    } else {
      Err(Error::Uninitialized)
    }
  }
}

fn align_up(address: u64) -> Result<u64, Error> {
  address
    .checked_add(PAGE_SIZE - 1)
    .map(|value| value & !(PAGE_SIZE - 1))
    .ok_or(Error::Memory(super::Error::InvalidRange))
}

fn zero_block(start: u64, frames: usize) -> Result<(), Error> {
  let bytes = frames
    .checked_mul(PAGE_SIZE as usize)
    .ok_or(Error::InvalidFrameCount)?;
  let address = usize::try_from(start).map_err(|_| Error::AddressNotRepresentable)?;
  // SAFETY: the caller changed a complete managed contiguous block from Free
  // to Allocated. Usable RAM is kernel identity-mapped read/write before the
  // runtime allocator is initialized.
  unsafe { ptr::write_bytes(address as *mut u8, 0, bytes) };
  Ok(())
}

pub type RuntimeFrameAllocator = FrameAllocator<MAX_RUNTIME_FRAMES>;

#[cfg(target_arch = "riscv64")]
static RUNTIME_ALLOCATOR: SpinLock<RuntimeFrameAllocator> =
  SpinLock::new(RuntimeFrameAllocator::uninitialized());

#[cfg(target_arch = "riscv64")]
pub fn initialize(early: &EarlyFrameAllocator<'_>) -> Result<usize, Error> {
  let mut allocator = RUNTIME_ALLOCATOR.lock_irqsave();
  allocator.initialize(early)?;
  allocator.free_frames()
}

#[cfg(target_arch = "riscv64")]
pub fn allocate(frames: usize) -> Result<FrameBlock, Error> {
  RUNTIME_ALLOCATOR.lock_irqsave().allocate(frames)
}

#[cfg(target_arch = "riscv64")]
pub fn release(block: FrameBlock) -> Result<(), Error> {
  RUNTIME_ALLOCATOR.lock_irqsave().release(block)
}

#[cfg(target_arch = "riscv64")]
pub fn free_frames() -> Result<usize, Error> {
  RUNTIME_ALLOCATOR.lock_irqsave().free_frames()
}

#[cfg(test)]
pub(crate) const fn frame_block_for_test(start: u64, frames: usize) -> FrameBlock {
  FrameBlock { start, frames }
}

#[cfg(test)]
mod tests {
  use core::ptr;
  use std::alloc::{Layout, alloc, dealloc};

  use crate::platform::{MemoryRegion, PlatformInfo};

  use super::*;

  struct TestMemory {
    pointer: *mut u8,
    layout: Layout,
  }

  impl TestMemory {
    fn new(pages: usize) -> Self {
      let layout = Layout::from_size_align(pages * PAGE_SIZE as usize, PAGE_SIZE as usize).unwrap();
      let pointer = unsafe { alloc(layout) };
      assert!(!pointer.is_null());
      unsafe { ptr::write_bytes(pointer, 0xa5, layout.size()) };
      Self { pointer, layout }
    }

    fn region(&self) -> MemoryRegion {
      MemoryRegion {
        base: self.pointer as u64,
        size: self.layout.size() as u64,
      }
    }
  }

  impl Drop for TestMemory {
    fn drop(&mut self) {
      unsafe { dealloc(self.pointer, self.layout) };
    }
  }

  fn allocator(storage: &TestMemory) -> FrameAllocator<8> {
    let platform = PlatformInfo::for_test(&[storage.region()], &[]);
    let outside = MemoryRegion {
      base: 0x1000,
      size: 1,
    };
    let early = EarlyFrameAllocator::new(&platform, outside, outside).unwrap();
    let mut allocator = FrameAllocator::uninitialized();
    allocator.initialize(&early).unwrap();
    allocator
  }

  #[test]
  fn frame_001_contiguous_allocation_is_zeroed() {
    let storage = TestMemory::new(8);
    let mut allocator = allocator(&storage);
    let block = allocator.allocate(4).unwrap();
    assert_eq!(block.frames(), 4);
    let bytes = unsafe { core::slice::from_raw_parts(block.start() as *const u8, 4 * 4096) };
    assert!(bytes.iter().all(|byte| *byte == 0));
  }

  #[test]
  fn frame_002_release_reuses_and_rezeros_block() {
    let storage = TestMemory::new(8);
    let mut allocator = allocator(&storage);
    let block = allocator.allocate(4).unwrap();
    let start = block.start();
    unsafe { ptr::write_bytes(block.start() as *mut u8, 0x5a, 4 * 4096) };
    allocator.release(block).unwrap();
    let reused = allocator.allocate(4).unwrap();
    assert_eq!(reused.start(), start);
    let bytes = unsafe { core::slice::from_raw_parts(reused.start() as *const u8, 4 * 4096) };
    assert!(bytes.iter().all(|byte| *byte == 0));
  }

  #[test]
  fn frame_003_foreign_block_is_rejected() {
    let storage = TestMemory::new(8);
    let mut allocator = allocator(&storage);
    let foreign = frame_block_for_test(0x1000, 1);
    assert_eq!(allocator.release(foreign), Err(Error::ForeignBlock));
  }

  #[test]
  fn frame_004_zero_frame_request_is_rejected() {
    let storage = TestMemory::new(8);
    let mut allocator = allocator(&storage);
    assert_eq!(allocator.allocate(0), Err(Error::InvalidFrameCount));
  }

  #[test]
  fn frame_005_early_allocation_is_not_republished() {
    let storage = TestMemory::new(8);
    let platform = PlatformInfo::for_test(&[storage.region()], &[]);
    let outside = MemoryRegion {
      base: 0x1000,
      size: 1,
    };
    let mut early = EarlyFrameAllocator::new(&platform, outside, outside).unwrap();
    let consumed = early.allocate_zeroed().unwrap().unwrap();
    let mut allocator = FrameAllocator::<8>::uninitialized();
    allocator.initialize(&early).unwrap();
    assert_eq!(allocator.free_frames(), Ok(7));
    assert_ne!(allocator.allocate(1).unwrap().start(), consumed.start());
  }
}
