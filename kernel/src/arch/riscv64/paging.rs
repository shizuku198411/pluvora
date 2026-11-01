use core::mem::size_of;
use core::ptr;

#[cfg(target_arch = "riscv64")]
use core::arch::asm;

use crate::memory::{EarlyFrameAllocator, Error as MemoryError, PAGE_SIZE};
use crate::platform::MemoryRegion;

const ENTRIES_PER_TABLE: usize = 512;
const VPN_MASK: u64 = 0x1ff;
const PTE_PPN_SHIFT: u64 = 10;
const PAGE_SHIFT: u64 = 12;

const PTE_VALID: u64 = 1 << 0;
const PTE_READ: u64 = 1 << 1;
const PTE_WRITE: u64 = 1 << 2;
const PTE_EXECUTE: u64 = 1 << 3;
const PTE_ACCESSED: u64 = 1 << 6;
const PTE_DIRTY: u64 = 1 << 7;

const SATP_MODE_SV39: u64 = 8;
const SATP_MODE_SHIFT: u64 = 60;
const SATP_PPN_MASK: u64 = (1 << 44) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  Memory(MemoryError),
  OutOfMemory,
  InvalidRegion,
  InvalidVirtualAddress,
  InvalidPhysicalAddress,
  AlreadyMapped,
  LeafWhereTableExpected,
  Sv39NotSupported,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Memory(error) => error.as_str(),
      Self::OutOfMemory => "no frame available for page table",
      Self::InvalidRegion => "invalid paging region",
      Self::InvalidVirtualAddress => "non-canonical Sv39 virtual address",
      Self::InvalidPhysicalAddress => "physical address does not fit Sv39 PTE",
      Self::AlreadyMapped => "virtual page is already mapped",
      Self::LeafWhereTableExpected => "leaf PTE found above level zero",
      Self::Sv39NotSupported => "hart did not accept Sv39 in satp",
    }
  }
}

impl From<MemoryError> for Error {
  fn from(error: MemoryError) -> Self {
    Self::Memory(error)
  }
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permissions {
  ReadOnly,
  ReadWrite,
  ReadExecute,
}

impl Permissions {
  const fn pte_flags(self) -> u64 {
    match self {
      Self::ReadOnly => PTE_READ | PTE_ACCESSED,
      Self::ReadWrite => PTE_READ | PTE_WRITE | PTE_ACCESSED | PTE_DIRTY,
      Self::ReadExecute => PTE_READ | PTE_EXECUTE | PTE_ACCESSED,
    }
  }
}

#[derive(Clone, Copy, Debug)]
pub struct KernelLayout {
  pub text: MemoryRegion,
  pub rodata: MemoryRegion,
  pub data: Option<MemoryRegion>,
  pub bss: MemoryRegion,
}

#[derive(Clone, Copy, Debug)]
pub struct PagingInfo {
  root_frame: u64,
  mapped_pages: usize,
  page_table_frames: usize,
}

impl PagingInfo {
  pub const fn root_frame(self) -> u64 {
    self.root_frame
  }

  pub const fn mapped_pages(self) -> usize {
    self.mapped_pages
  }

  pub const fn page_table_frames(self) -> usize {
    self.page_table_frames
  }

  pub fn satp_value(self) -> Result<u64, Error> {
    let root_ppn = self.root_frame >> PAGE_SHIFT;
    if root_ppn > SATP_PPN_MASK {
      return Err(Error::InvalidPhysicalAddress);
    }
    Ok((SATP_MODE_SV39 << SATP_MODE_SHIFT) | root_ppn)
  }
}

pub fn build(
  allocator: &mut EarlyFrameAllocator<'_>,
  layout: KernelLayout,
  dtb: MemoryRegion,
) -> Result<PagingInfo, Error> {
  let mut mapper = Mapper::new(allocator)?;

  mapper.map_identity(layout.text, Permissions::ReadExecute)?;
  mapper.map_identity(layout.rodata, Permissions::ReadOnly)?;
  if let Some(data) = layout.data {
    mapper.map_identity(data, Permissions::ReadWrite)?;
  }
  mapper.map_identity(layout.bss, Permissions::ReadWrite)?;
  mapper.map_identity(dtb, Permissions::ReadOnly)?;

  Ok(PagingInfo {
    root_frame: mapper.root_frame,
    mapped_pages: mapper.mapped_pages,
    page_table_frames: mapper.page_table_frames,
  })
}

/// Activates an identity-mapped Sv39 address space.
///
/// # Safety
///
/// The caller must ensure that the current instruction address, stack, all subsequently
/// accessed static data, and the SBI console call path are valid in `paging`
/// before this function writes `satp`.
#[cfg(target_arch = "riscv64")]
pub unsafe fn activate(paging: PagingInfo) -> Result<(), Error> {
  let satp = paging.satp_value()?;

  // SAFETY: The caller guarantees that execution can continue using
  // the new identity-mapped address spaces.
  // ASID 0 is used during early boot.
  unsafe {
    asm!(
      "csrw satp, {satp}",
      "sfence.vma zero, zero",
      satp = in(reg) satp,
      options(nostack),
    );
  }

  let active_satp: u64;
  // SAFETY: Reading satp is permitted in S-mode.
  unsafe { asm!("csrr {value}, satp", value = out(reg) active_satp, options(nostack)) };

  if active_satp != satp {
    return Err(Error::Sv39NotSupported);
  }

  Ok(())
}

struct Mapper<'allocator, 'memory> {
  allocator: &'allocator mut EarlyFrameAllocator<'memory>,
  root_frame: u64,
  mapped_pages: usize,
  page_table_frames: usize,
}

impl<'allocator, 'memory> Mapper<'allocator, 'memory> {
  fn new(allocator: &'allocator mut EarlyFrameAllocator<'memory>) -> Result<Self, Error> {
    let root_frame = allocate_table_frame(allocator)?;
    Ok(Self {
      allocator,
      root_frame,
      mapped_pages: 0,
      page_table_frames: 1,
    })
  }

  fn map_identity(&mut self, region: MemoryRegion, permissions: Permissions) -> Result<(), Error> {
    let end = region.end().ok_or(Error::InvalidRegion)?;
    if region.size == 0 {
      return Err(Error::InvalidRegion);
    }

    let mut address = align_down(region.base);
    let aligned_end = align_up(end)?;
    while address < aligned_end {
      self.map_page(address, address, permissions)?;
      address = address.checked_add(PAGE_SIZE).ok_or(Error::InvalidRegion)?;
    }
    Ok(())
  }

  fn map_page(
    &mut self,
    virtual_address: u64,
    physical_address: u64,
    permissions: Permissions,
  ) -> Result<(), Error> {
    if !virtual_address.is_multiple_of(PAGE_SIZE) || !physical_address.is_multiple_of(PAGE_SIZE) {
      return Err(Error::InvalidRegion);
    }
    if !is_sv39_canonical(virtual_address) {
      return Err(Error::InvalidVirtualAddress);
    }
    if physical_address >> PAGE_SHIFT > SATP_PPN_MASK {
      return Err(Error::InvalidPhysicalAddress);
    }

    let indices = vpn_indices(virtual_address);
    let mut table_frame = self.root_frame;

    for level in (1..=2).rev() {
      let index = indices[level];
      let entry = read_entry(table_frame, index)?;

      table_frame = if entry & PTE_VALID == 0 {
        let child = allocate_table_frame(self.allocator)?;
        self.page_table_frames += 1;
        write_entry(table_frame, index, table_pointer(child))?;
        child
      } else {
        if is_leaf(entry) {
          return Err(Error::LeafWhereTableExpected);
        }
        pte_physical_address(entry)
      };
    }

    let leaf_index = indices[0];
    if read_entry(table_frame, leaf_index)? & PTE_VALID != 0 {
      return Err(Error::AlreadyMapped);
    }

    let leaf = table_pointer(physical_address) | permissions.pte_flags();
    write_entry(table_frame, leaf_index, leaf)?;
    self.mapped_pages += 1;
    Ok(())
  }
}

fn allocate_table_frame(allocator: &mut EarlyFrameAllocator<'_>) -> Result<u64, Error> {
  allocator
    .allocate_zeroed()?
    .map(|frame| frame.start())
    .ok_or(Error::OutOfMemory)
}

fn table_pointer(physical_address: u64) -> u64 {
  ((physical_address >> PAGE_SHIFT) << PTE_PPN_SHIFT) | PTE_VALID
}

fn pte_physical_address(entry: u64) -> u64 {
  ((entry >> PTE_PPN_SHIFT) & SATP_PPN_MASK) << PAGE_SHIFT
}

fn is_leaf(entry: u64) -> bool {
  entry & (PTE_READ | PTE_WRITE | PTE_EXECUTE) != 0
}

fn read_entry(table_frame: u64, index: usize) -> Result<u64, Error> {
  let address = entry_address(table_frame, index)?;
  // SAFETY: `table_frame` was allocated as a zeroed, aligned Page Table
  // and is accessed only by this Mapper while paging remains disabled.
  Ok(unsafe { ptr::read(address as *const u64) })
}

fn write_entry(table_frame: u64, index: usize, value: u64) -> Result<(), Error> {
  let address = entry_address(table_frame, index)?;
  // SAFETY: the entry is aligned, belongs to a Page Table exclusively owned
  // by this Mapper, and is written as one 64-bit value.
  unsafe { ptr::write(address as *mut u64, value) };
  Ok(())
}

fn entry_address(table_frame: u64, index: usize) -> Result<usize, Error> {
  if index >= ENTRIES_PER_TABLE || !table_frame.is_multiple_of(PAGE_SIZE) {
    return Err(Error::InvalidPhysicalAddress);
  }
  let byte_offset = (index as u64)
    .checked_mul(size_of::<u64>() as u64)
    .ok_or(Error::InvalidPhysicalAddress)?;
  let address = table_frame
    .checked_add(byte_offset)
    .ok_or(Error::InvalidPhysicalAddress)?;
  usize::try_from(address).map_err(|_| Error::InvalidPhysicalAddress)
}

fn vpn_indices(virtual_address: u64) -> [usize; 3] {
  [
    ((virtual_address >> 12) & VPN_MASK) as usize,
    ((virtual_address >> 21) & VPN_MASK) as usize,
    ((virtual_address >> 30) & VPN_MASK) as usize,
  ]
}

fn is_sv39_canonical(address: u64) -> bool {
  let sign = (address >> 38) & 1;
  let upper = address >> 39;
  upper == if sign == 0 { 0 } else { (1 << 25) - 1 }
}

fn align_down(value: u64) -> u64 {
  value & !(PAGE_SIZE - 1)
}

fn align_up(value: u64) -> Result<u64, Error> {
  value
    .checked_add(PAGE_SIZE - 1)
    .map(align_down)
    .ok_or(Error::InvalidRegion)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn vm_001_lower_sv39_address_is_canonical() {
    assert!(is_sv39_canonical(0x0000_003f_ffff_ffff));
  }

  #[test]
  fn vm_002_upper_sv39_address_is_canonical() {
    assert!(is_sv39_canonical(0xffff_ffc0_0000_0000));
  }

  #[test]
  fn vm_003_invalid_sign_extension_is_rejected() {
    assert!(!is_sv39_canonical(0x0000_0040_0000_0000));
    assert!(!is_sv39_canonical(0xffff_ffbf_ffff_ffff));
  }

  #[test]
  fn vm_004_vpn_indices_are_extracted_from_the_expected_bits() {
    let address = (0x12au64 << 30) | (0x155u64 << 21) | (0x0aau64 << 12);
    assert_eq!(vpn_indices(address), [0x0aa, 0x155, 0x12a]);
  }

  #[test]
  fn vm_005_pte_ppn_round_trips() {
    let physical_address = 0x0000_1234_5678_9000;
    let entry = table_pointer(physical_address);
    assert_eq!(pte_physical_address(entry), physical_address);
    assert_eq!(entry & PTE_VALID, PTE_VALID);
  }

  #[test]
  fn vm_007_non_leaf_pointer_only_sets_valid_flag() {
    let entry = table_pointer(0x8020_0000);
    assert_eq!(entry & 0xff, PTE_VALID);
    assert!(!is_leaf(entry));
  }

  #[test]
  fn vm_009_permission_sets_never_create_write_execute_leaves() {
    for permissions in [
      Permissions::ReadOnly,
      Permissions::ReadWrite,
      Permissions::ReadExecute,
    ] {
      let flags = permissions.pte_flags();
      assert_ne!(flags & PTE_READ, 0);
      assert_ne!(flags & (PTE_WRITE | PTE_EXECUTE), PTE_WRITE | PTE_EXECUTE);
    }
  }

  #[test]
  fn vm_015_alignment_covers_partial_pages() {
    assert_eq!(align_down(0x1234), 0x1000);
    assert_eq!(align_up(0x1234), Ok(0x2000));
    assert_eq!(align_up(0x2000), Ok(0x2000));
  }

  #[test]
  fn vm_015_alignment_overflow_is_rejected() {
    assert_eq!(align_up(u64::MAX), Err(Error::InvalidRegion));
  }

  #[test]
  fn vm_021_satp_encodes_sv39_mode_and_root_ppn() {
    let paging = PagingInfo {
      root_frame: 0x0000_0000_8020_0000,
      mapped_pages: 0,
      page_table_frames: 0,
    };
    assert_eq!(
      paging.satp_value(),
      Ok((SATP_MODE_SV39 << SATP_MODE_SHIFT) | 0x80200)
    );
  }
}
