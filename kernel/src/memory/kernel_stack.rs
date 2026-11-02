use super::{PAGE_SIZE, frame_allocator};

pub const KERNEL_STACK_PAGES: usize = 4;
pub const KERNEL_STACK_BYTES: usize = KERNEL_STACK_PAGES * PAGE_SIZE as usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
  Allocator(frame_allocator::Error),
  InvalidLayout,
}

impl Error {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Allocator(error) => error.as_str(),
      Self::InvalidLayout => "kernel stack layout is invalid",
    }
  }
}

impl From<frame_allocator::Error> for Error {
  fn from(error: frame_allocator::Error) -> Self {
    Self::Allocator(error)
  }
}

pub struct KernelStack {
  block: frame_allocator::FrameBlock,
}

impl KernelStack {
  #[cfg(target_arch = "riscv64")]
  pub fn allocate() -> Result<Self, Error> {
    let block = frame_allocator::allocate(KERNEL_STACK_PAGES)?;
    let stack = Self { block };
    if stack.top() == 0 || !stack.top().is_multiple_of(16) {
      frame_allocator::release(stack.block)?;
      return Err(Error::InvalidLayout);
    }
    Ok(stack)
  }

  pub const fn base(&self) -> u64 {
    self.block.start()
  }

  pub fn top(&self) -> usize {
    self
      .block
      .start()
      .checked_add(KERNEL_STACK_BYTES as u64)
      .and_then(|top| usize::try_from(top).ok())
      .unwrap_or(0)
  }

  pub const fn pages(&self) -> usize {
    KERNEL_STACK_PAGES
  }

  #[cfg(target_arch = "riscv64")]
  pub fn release(self) -> Result<(), Error> {
    frame_allocator::release(self.block)?;
    Ok(())
  }
}
