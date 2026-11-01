#[cfg(target_arch = "riscv64")]
use core::arch::asm;
#[cfg(target_arch = "riscv64")]
use core::hint::spin_loop;
#[cfg(target_arch = "riscv64")]
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::hart;
use crate::arch::riscv64::{ipi, paging};
#[cfg(target_arch = "riscv64")]
use crate::memory::address_space::{AddressSpace, Asid};
#[cfg(target_arch = "riscv64")]
use crate::platform::MAX_HARTS;
#[cfg(target_arch = "riscv64")]
use crate::sync::SpinLock;

const KIND_PAGE: usize = 1;
const KIND_ADDRESS_SPACE: usize = 2;
const KIND_GLOBAL: usize = 3;
#[cfg(target_arch = "riscv64")]
const WAIT_LIMIT: usize = 10_000_000;

#[cfg(target_arch = "riscv64")]
static COORDINATOR: SpinLock<()> = SpinLock::new(());
#[cfg(target_arch = "riscv64")]
static DESCRIPTOR: Descriptor = Descriptor::new();

#[cfg(target_arch = "riscv64")]
struct Descriptor {
  sequence: AtomicU64,
  asid: AtomicU64,
  generation: AtomicU64,
  virtual_address: AtomicU64,
  kind: AtomicUsize,
  targets: AtomicU64,
  acknowledgments: AtomicU64,
  poisoned: AtomicBool,
}

#[cfg(target_arch = "riscv64")]
impl Descriptor {
  const fn new() -> Self {
    Self {
      sequence: AtomicU64::new(0),
      asid: AtomicU64::new(0),
      generation: AtomicU64::new(0),
      virtual_address: AtomicU64::new(0),
      kind: AtomicUsize::new(0),
      targets: AtomicU64::new(0),
      acknowledgments: AtomicU64::new(0),
      poisoned: AtomicBool::new(false),
    }
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
  Page { virtual_address: u64 },
  AddressSpace,
  Global,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  InvalidAddress,
  InvalidHartMask,
  InvalidScope,
  SequenceOverflow,
  GenerationMismatch,
  Ipi(ipi::Error),
  Timeout,
  Poisoned,
}

#[cfg(target_arch = "riscv64")]
pub fn shootdown(space: &AddressSpace, scope: Scope) -> Result<u64, Error> {
  let _coordinator = COORDINATOR.lock();
  if DESCRIPTOR.poisoned.load(Ordering::Acquire) {
    return Err(Error::Poisoned);
  }

  validate_scope(scope)?;
  let targets = space.resident_harts() & hart::online_mask();
  run(space.asid(), scope, targets)?;
  Ok(targets)
}

#[cfg(target_arch = "riscv64")]
pub fn shootdown_global() -> Result<u64, Error> {
  let _coordinator = COORDINATOR.lock();
  if DESCRIPTOR.poisoned.load(Ordering::Acquire) {
    return Err(Error::Poisoned);
  }
  let targets = hart::online_mask();
  run(Asid::kernel(), Scope::Global, targets)?;
  Ok(targets)
}

#[cfg(target_arch = "riscv64")]
fn run(asid: Asid, scope: Scope, targets: u64) -> Result<(), Error> {
  let origin = hart::current_index();
  let origin_bit = 1u64
    .checked_shl(origin as u32)
    .ok_or(Error::InvalidHartMask)?;
  let remote_targets = targets & !origin_bit;
  let next_sequence = DESCRIPTOR
    .sequence
    .load(Ordering::Relaxed)
    .checked_add(1)
    .ok_or(Error::SequenceOverflow)?;

  let (kind, address) = encode_scope(scope);
  DESCRIPTOR
    .asid
    .store(u64::from(asid.value()), Ordering::Relaxed);
  DESCRIPTOR
    .generation
    .store(asid.generation(), Ordering::Relaxed);
  DESCRIPTOR.virtual_address.store(address, Ordering::Relaxed);
  DESCRIPTOR.kind.store(kind, Ordering::Relaxed);
  DESCRIPTOR.targets.store(remote_targets, Ordering::Relaxed);
  DESCRIPTOR.acknowledgments.store(0, Ordering::Relaxed);

  // Make Page Table stores and descriptor fields visible before notification.
  unsafe { asm!("fence w, w", options(nostack)) };
  DESCRIPTOR.sequence.store(next_sequence, Ordering::Release);

  if targets & origin_bit != 0 {
    execute(scope, asid.value());
  }

  for index in 0..MAX_HARTS {
    if remote_targets & (1u64 << index) != 0
      && let Err(error) = ipi::send(index, ipi::Request::TlbShootdown)
    {
      DESCRIPTOR.poisoned.store(true, Ordering::Release);
      return Err(Error::Ipi(error));
    }
  }

  for _ in 0..WAIT_LIMIT {
    if acknowledgments_complete(
      DESCRIPTOR.acknowledgments.load(Ordering::Acquire),
      remote_targets,
    ) {
      return Ok(());
    }
    spin_loop();
  }

  DESCRIPTOR.poisoned.store(true, Ordering::Release);
  Err(Error::Timeout)
}

#[cfg(target_arch = "riscv64")]
pub fn handle_local() {
  let index = hart::current_index();
  let bit = 1u64 << index;
  let sequence = DESCRIPTOR.sequence.load(Ordering::Acquire);
  let targets = DESCRIPTOR.targets.load(Ordering::Relaxed);
  if sequence == 0 || targets & bit == 0 {
    DESCRIPTOR.poisoned.store(true, Ordering::Release);
    return;
  }

  let asid = DESCRIPTOR.asid.load(Ordering::Relaxed) as u16;
  let scope = match decode_scope(
    DESCRIPTOR.kind.load(Ordering::Relaxed),
    DESCRIPTOR.virtual_address.load(Ordering::Relaxed),
  ) {
    Ok(scope) => scope,
    Err(_) => {
      DESCRIPTOR.poisoned.store(true, Ordering::Release);
      return;
    }
  };

  execute(scope, asid);
  DESCRIPTOR.acknowledgments.fetch_or(bit, Ordering::Release);
}

fn validate_scope(scope: Scope) -> Result<(), Error> {
  match scope {
    Scope::Page { virtual_address } if !paging_address_is_valid(virtual_address) => {
      Err(Error::InvalidAddress)
    }
    _ => Ok(()),
  }
}

fn paging_address_is_valid(virtual_address: u64) -> bool {
  paging::is_valid_page_address(virtual_address)
}

fn encode_scope(scope: Scope) -> (usize, u64) {
  match scope {
    Scope::Page { virtual_address } => (KIND_PAGE, virtual_address),
    Scope::AddressSpace => (KIND_ADDRESS_SPACE, 0),
    Scope::Global => (KIND_GLOBAL, 0),
  }
}

fn decode_scope(kind: usize, virtual_address: u64) -> Result<Scope, Error> {
  match kind {
    KIND_PAGE => {
      let scope = Scope::Page { virtual_address };
      validate_scope(scope)?;
      Ok(scope)
    }
    KIND_ADDRESS_SPACE if virtual_address == 0 => Ok(Scope::AddressSpace),
    KIND_GLOBAL if virtual_address == 0 => Ok(Scope::Global),
    _ => Err(Error::InvalidScope),
  }
}

fn acknowledgments_complete(acknowledgments: u64, targets: u64) -> bool {
  acknowledgments & targets == targets
}

#[cfg(target_arch = "riscv64")]
fn execute(scope: Scope, asid: u16) {
  match scope {
    Scope::Page { virtual_address } => paging::fence_page(virtual_address, asid),
    Scope::AddressSpace => paging::fence_asid(asid),
    Scope::Global => paging::fence_global(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tlb_001_page_scope_round_trips() {
    let scope = Scope::Page {
      virtual_address: 0x8020_0000,
    };
    let (kind, address) = encode_scope(scope);
    assert_eq!(decode_scope(kind, address), Ok(scope));
  }

  #[test]
  fn tlb_002_address_space_scope_round_trips() {
    let (kind, address) = encode_scope(Scope::AddressSpace);
    assert_eq!(decode_scope(kind, address), Ok(Scope::AddressSpace));
  }

  #[test]
  fn tlb_003_global_scope_round_trips() {
    let (kind, address) = encode_scope(Scope::Global);
    assert_eq!(decode_scope(kind, address), Ok(Scope::Global));
  }

  #[test]
  fn tlb_005_completion_requires_every_target() {
    assert!(!acknowledgments_complete(0b0011, 0b0111));
    assert!(acknowledgments_complete(0b0111, 0b0111));
    assert!(acknowledgments_complete(0b1111, 0b0111));
  }

  #[test]
  fn tlb_007_invalid_scope_encoding_is_rejected() {
    assert_eq!(decode_scope(usize::MAX, 0), Err(Error::InvalidScope));
    assert_eq!(
      validate_scope(Scope::Page {
        virtual_address: 0x8020_0001,
      }),
      Err(Error::InvalidAddress)
    );
  }
}
