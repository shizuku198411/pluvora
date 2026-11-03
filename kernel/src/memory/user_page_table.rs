#[cfg(target_arch = "riscv64")]
use core::ptr;

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::paging::PTE_VALID;
use crate::arch::riscv64::paging::{self, ENTRIES_PER_TABLE, UserPermissions};
use crate::memory::{PAGE_SIZE, frame_allocator};

pub const USER_WINDOW_START: u64 = PAGE_SIZE;
pub const USER_WINDOW_END: u64 = 1 << 30;
pub const MAX_OWNED_PAGE_TABLE_FRAMES: usize = 3;

const USER_ROOT_INDEX: usize = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  Allocator(frame_allocator::Error),
  Paging(paging::Error),
  KernelUserWindowOccupied,
  InvalidVirtualAddress,
  InvalidPhysicalAddress,
  AlreadyMapped,
  LeafWhereTableExpected,
  TableFrameLimit,
  MappingLimit,
  KernelAddressSpace,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Allocator(error) => error.as_str(),
      Self::Paging(error) => error.as_str(),
      Self::KernelUserWindowOccupied => "kernel root occupies the user virtual window",
      Self::InvalidVirtualAddress => "virtual address is outside the user window",
      Self::InvalidPhysicalAddress => "physical user frame is invalid",
      Self::AlreadyMapped => "user virtual page is already mapped",
      Self::LeafWhereTableExpected => "user page-table walk found an upper-level leaf",
      Self::TableFrameLimit => "user page-table frame limit reached",
      Self::MappingLimit => "user Address Space mapping limit reached",
      Self::KernelAddressSpace => "kernel Address Space cannot receive a user mapping",
    }
  }
}

pub struct PreparedMappingFrames {
  level_one: frame_allocator::FrameBlock,
  level_zero: frame_allocator::FrameBlock,
}

impl PreparedMappingFrames {
  #[cfg(target_arch = "riscv64")]
  pub fn allocate() -> Result<Self, Error> {
    let level_one = frame_allocator::allocate(1)?;
    let level_zero = match frame_allocator::allocate(1) {
      Ok(frame) => frame,
      Err(error) => {
        frame_allocator::release(level_one)
          .unwrap_or_else(|_| panic!("failed prepared mapping rollback"));
        return Err(Error::Allocator(error));
      }
    };
    Ok(Self {
      level_one,
      level_zero,
    })
  }

  #[cfg(target_arch = "riscv64")]
  pub fn release(self) -> Result<(), Error> {
    frame_allocator::release(self.level_zero)?;
    frame_allocator::release(self.level_one)?;
    Ok(())
  }

  #[cfg(test)]
  pub(crate) fn for_test(level_one: u64, level_zero: u64) -> Self {
    Self {
      level_one: frame_allocator::frame_block_for_test(level_one, 1),
      level_zero: frame_allocator::frame_block_for_test(level_zero, 1),
    }
  }
}

pub struct MappingFailure {
  error: Error,
  frames: PreparedMappingFrames,
}

impl MappingFailure {
  pub(crate) const fn new(error: Error, frames: PreparedMappingFrames) -> Self {
    Self { error, frames }
  }

  pub const fn error(&self) -> Error {
    self.error
  }

  pub fn into_parts(self) -> (Error, PreparedMappingFrames) {
    (self.error, self.frames)
  }
}

impl From<frame_allocator::Error> for Error {
  fn from(error: frame_allocator::Error) -> Self {
    Self::Allocator(error)
  }
}

impl From<paging::Error> for Error {
  fn from(error: paging::Error) -> Self {
    Self::Paging(error)
  }
}

pub struct UserPageTable {
  root_frame: u64,
  owned: [Option<frame_allocator::FrameBlock>; MAX_OWNED_PAGE_TABLE_FRAMES],
  owned_len: usize,
  mapped_pages: usize,
}

impl UserPageTable {
  #[cfg(target_arch = "riscv64")]
  pub fn clone_kernel_root(kernel_root: u64) -> Result<Self, Error> {
    validate_table_frame(kernel_root)?;
    if read_entry(kernel_root, USER_ROOT_INDEX)? & PTE_VALID != 0 {
      return Err(Error::KernelUserWindowOccupied);
    }

    let root = frame_allocator::allocate(1)?;
    let root_frame = root.start();
    let root_address = usize::try_from(root_frame).map_err(|_| Error::InvalidPhysicalAddress)?;
    let kernel_address = usize::try_from(kernel_root).map_err(|_| Error::InvalidPhysicalAddress)?;
    // SAFETY: both ranges are aligned live Page Table frames. `root` is exclusively owned.
    unsafe {
      ptr::copy_nonoverlapping(
        kernel_address as *const u8,
        root_address as *mut u8,
        PAGE_SIZE as usize,
      );
    }

    let mut owned = [const { None }; MAX_OWNED_PAGE_TABLE_FRAMES];
    owned[0] = Some(root);
    Ok(Self {
      root_frame,
      owned,
      owned_len: 1,
      mapped_pages: 0,
    })
  }

  pub const fn root_frame(&self) -> u64 {
    self.root_frame
  }

  pub const fn owned_frames(&self) -> usize {
    self.owned_len
  }

  pub const fn mapped_pages(&self) -> usize {
    self.mapped_pages
  }

  #[cfg(target_arch = "riscv64")]
  pub fn map_user_page(
    &mut self,
    virtual_address: u64,
    physical_address: u64,
    permissions: UserPermissions,
    frames: PreparedMappingFrames,
  ) -> Result<(), MappingFailure> {
    if let Err(error) = validate_user_page(virtual_address) {
      return Err(MappingFailure::new(error, frames));
    }
    if let Err(error) = validate_physical_page(physical_address) {
      return Err(MappingFailure::new(error, frames));
    }
    if self.mapped_pages != 0 {
      return Err(MappingFailure::new(Error::MappingLimit, frames));
    }
    if self.owned_len + 2 > MAX_OWNED_PAGE_TABLE_FRAMES {
      return Err(MappingFailure::new(Error::TableFrameLimit, frames));
    }

    let indices = paging::vpn_indices(virtual_address);
    let root_entry = match read_entry(self.root_frame, indices[2]) {
      Ok(entry) => entry,
      Err(error) => return Err(MappingFailure::new(error, frames)),
    };
    if root_entry & PTE_VALID != 0 {
      let error = if paging::is_leaf(root_entry) {
        Error::LeafWhereTableExpected
      } else {
        Error::AlreadyMapped
      };
      return Err(MappingFailure::new(error, frames));
    }

    let PreparedMappingFrames {
      level_one,
      level_zero,
    } = frames;

    write_entry(
      level_one.start(),
      indices[1],
      paging::table_pointer(level_zero.start()),
    )
    .unwrap_or_else(|_| panic!("validated level-one Page Table write failed"));
    write_entry(
      level_zero.start(),
      indices[0],
      paging::table_pointer(physical_address) | permissions.pte_flags(),
    )
    .unwrap_or_else(|_| panic!("validated user leaf write failed"));
    write_entry(
      self.root_frame,
      indices[2],
      paging::table_pointer(level_one.start()),
    )
    .unwrap_or_else(|_| panic!("validated user root write failed"));

    self.owned[self.owned_len] = Some(level_one);
    self.owned[self.owned_len + 1] = Some(level_zero);
    self.owned_len += 2;
    self.mapped_pages += 1;
    Ok(())
  }

  #[cfg(target_arch = "riscv64")]
  pub fn release(mut self) -> Result<(), Error> {
    while self.owned_len != 0 {
      self.owned_len -= 1;
      let block = self.owned[self.owned_len]
        .take()
        .unwrap_or_else(|| panic!("owned Page Table frame disappeared"));
      frame_allocator::release(block)?;
    }
    Ok(())
  }
}

fn validate_user_page(address: u64) -> Result<(), Error> {
  if !(USER_WINDOW_START..USER_WINDOW_END).contains(&address)
    || !paging::is_valid_page_address(address)
  {
    return Err(Error::InvalidVirtualAddress);
  }
  Ok(())
}

fn validate_physical_page(address: u64) -> Result<(), Error> {
  if address == 0 || !address.is_multiple_of(PAGE_SIZE) || address >> 12 > paging::SATP_PPN_MASK {
    return Err(Error::InvalidPhysicalAddress);
  }
  Ok(())
}

#[cfg(target_arch = "riscv64")]
fn validate_table_frame(frame: u64) -> Result<(), Error> {
  validate_physical_page(frame)
}

#[cfg(target_arch = "riscv64")]
fn read_entry(table: u64, index: usize) -> Result<u64, Error> {
  let address = paging::entry_address(table, index)?;
  // SAFETY: caller supplies a live identity-mapped Page Table frame.
  Ok(unsafe { ptr::read(address as *const u64) })
}

#[cfg(target_arch = "riscv64")]
fn write_entry(table: u64, index: usize, value: u64) -> Result<(), Error> {
  let address = paging::entry_address(table, index)?;
  // SAFETY: caller exclusively owns the target user Page Table frame.
  unsafe { ptr::write(address as *mut u64, value) };
  Ok(())
}

const _: () = assert!(USER_ROOT_INDEX < ENTRIES_PER_TABLE);

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn uas_001_user_window_excludes_null_and_upper_boundary() {
    assert_eq!(validate_user_page(0), Err(Error::InvalidVirtualAddress));
    assert_eq!(
      validate_user_page(USER_WINDOW_END),
      Err(Error::InvalidVirtualAddress)
    );
    assert_eq!(validate_user_page(USER_WINDOW_START), Ok(()));
  }

  #[test]
  fn uas_002_user_page_requires_alignment() {
    assert_eq!(
      validate_user_page(USER_WINDOW_START + 1),
      Err(Error::InvalidVirtualAddress)
    );
  }

  #[test]
  fn uas_003_user_physical_page_rejects_zero_and_unaligned() {
    assert_eq!(
      validate_physical_page(0),
      Err(Error::InvalidPhysicalAddress)
    );
    assert_eq!(
      validate_physical_page(0x8000_0001),
      Err(Error::InvalidPhysicalAddress)
    );
    assert_eq!(validate_physical_page(0x8000_0000), Ok(()));
  }

  #[test]
  fn uas_004_user_window_stays_in_one_private_root_slot() {
    assert_eq!(paging::vpn_indices(USER_WINDOW_START)[2], USER_ROOT_INDEX);
    assert_eq!(
      paging::vpn_indices(USER_WINDOW_END - PAGE_SIZE)[2],
      USER_ROOT_INDEX
    );
  }

  #[test]
  fn uas_005_owned_frame_slots_match_the_published_count() {
    let root = frame_allocator::frame_block_for_test(0x8000_0000, 1);
    let root_frame = root.start();
    let mut owned = [const { None }; MAX_OWNED_PAGE_TABLE_FRAMES];
    owned[0] = Some(root);
    let table = UserPageTable {
      root_frame,
      owned,
      owned_len: 1,
      mapped_pages: 0,
    };
    assert_eq!(table.owned_frames(), 1);
    assert_eq!(
      table.owned[0]
        .as_ref()
        .map(frame_allocator::FrameBlock::start),
      Some(root_frame)
    );
  }

  #[test]
  fn uas_006_user_permissions_always_set_user_and_never_wx() {
    const WRITE: u64 = 1 << 2;
    const EXECUTE: u64 = 1 << 3;
    for permissions in [
      UserPermissions::ReadOnly,
      UserPermissions::ReadWrite,
      UserPermissions::ReadExecute,
    ] {
      let flags = permissions.pte_flags();
      assert_ne!(flags & paging::PTE_USER, 0);
      assert_ne!(flags & (WRITE | EXECUTE), WRITE | EXECUTE);
    }
  }

  #[test]
  fn uas_007_mapping_failure_returns_prepared_frame_ownership() {
    let frames = PreparedMappingFrames::for_test(0x8100_0000, 0x8100_1000);
    let failure = MappingFailure::new(Error::InvalidVirtualAddress, frames);
    let (error, frames) = failure.into_parts();
    assert_eq!(error, Error::InvalidVirtualAddress);
    assert_eq!(frames.level_one.start(), 0x8100_0000);
    assert_eq!(frames.level_zero.start(), 0x8100_1000);
  }
}
