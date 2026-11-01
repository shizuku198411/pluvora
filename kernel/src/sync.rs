use core::cell::UnsafeCell;
use core::hint::spin_loop;
#[cfg(target_arch = "riscv64")]
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::hart::{InterruptGuard, PreemptionGuard};

pub struct SpinLock<T> {
  locked: AtomicBool,
  value: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
  pub const fn new(value: T) -> Self {
    Self {
      locked: AtomicBool::new(false),
      value: UnsafeCell::new(value),
    }
  }

  #[cfg(target_arch = "riscv64")]
  pub fn lock(&self) -> SpinLockGuard<'_, T> {
    let preemption = PreemptionGuard::disable();
    self.acquire();
    SpinLockGuard {
      lock: self,
      _preemption: preemption,
    }
  }

  #[cfg(target_arch = "riscv64")]
  pub fn lock_irqsave(&self) -> IrqSpinLockGuard<'_, T> {
    let interrupt = InterruptGuard::disable();
    let preemption = PreemptionGuard::disable();
    self.acquire();
    IrqSpinLockGuard {
      lock: self,
      _preemption: preemption,
      _interrupt: interrupt,
    }
  }

  fn acquire(&self) {
    while !self.try_acquire() {
      while self.locked.load(Ordering::Relaxed) {
        spin_loop();
      }
    }
  }

  fn try_acquire(&self) -> bool {
    self
      .locked
      .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
      .is_ok()
  }

  fn release(&self) {
    self.locked.store(false, Ordering::Release);
  }
}

#[cfg(target_arch = "riscv64")]
pub struct SpinLockGuard<'a, T> {
  lock: &'a SpinLock<T>,
  _preemption: PreemptionGuard,
}

#[cfg(target_arch = "riscv64")]
pub struct IrqSpinLockGuard<'a, T> {
  lock: &'a SpinLock<T>,
  _preemption: PreemptionGuard,
  _interrupt: InterruptGuard,
}

#[cfg(target_arch = "riscv64")]
macro_rules! impl_guard {
  ($guard:ident) => {
    impl<T> Deref for $guard<'_, T> {
      type Target = T;

      fn deref(&self) -> &Self::Target {
        // SAFETY: the guard exists only after exclusive lock acquisition.
        unsafe { &*self.lock.value.get() }
      }
    }

    impl<T> DerefMut for $guard<'_, T> {
      fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: the lock permits exactly on live mutable guard.
        unsafe { &mut *self.lock.value.get() }
      }
    }

    impl<T> Drop for $guard<'_, T> {
      fn drop(&mut self) {
        self.lock.release();
      }
    }
  };
}

#[cfg(target_arch = "riscv64")]
impl_guard!(SpinLockGuard);
#[cfg(target_arch = "riscv64")]
impl_guard!(IrqSpinLockGuard);

#[cfg(test)]
mod tests {
  use std::sync::Arc;
  use std::thread;

  use super::*;

  fn increment(lock: &SpinLock<usize>) {
    lock.acquire();
    // SAFETY: the test holds the same exclusive lock used by production guards.
    unsafe { *lock.value.get() += 1 };
    lock.release();
  }

  #[test]
  fn sync_001_unlocked_state_can_be_acquired_once() {
    let lock = SpinLock::new(0usize);
    assert!(lock.try_acquire());
    assert!(!lock.try_acquire());
    lock.release();
  }

  #[test]
  fn sync_002_held_lock_rejects_try_acquire() {
    let lock = SpinLock::new(0usize);
    lock.acquire();
    assert!(!lock.try_acquire());
    lock.release();
    assert!(lock.try_acquire());
    lock.release();
  }

  #[test]
  fn sync_003_release_makes_lock_available_again() {
    let lock = SpinLock::new(0usize);
    lock.acquire();
    lock.release();
    assert!(lock.try_acquire());
    lock.release();
  }

  #[test]
  fn sync_004_contending_threads_do_not_lose_updates() {
    const THREADS: usize = 4;
    const INCREMENTS: usize = 2_000;
    let lock = Arc::new(SpinLock::new(0usize));
    let mut workers = std::vec::Vec::new();

    for _ in 0..THREADS {
      let lock = Arc::clone(&lock);
      workers.push(thread::spawn(move || {
        for _ in 0..INCREMENTS {
          increment(&lock);
        }
      }));
    }
    for worker in workers {
      worker.join().expect("spinlock test worker panicked");
    }

    lock.acquire();
    // SAFETY: all workers are joined and the test holds the lock.
    let value = unsafe { *lock.value.get() };
    lock.release();
    assert_eq!(value, THREADS * INCREMENTS);
  }
}
