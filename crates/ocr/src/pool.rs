//! Bounded pool of expensive per-thread resources (ORT session pairs). A caller checks one out,
//! runs without holding the lock and returns it through a RAII slot, so a panic inside the caller's
//! closure frees the slot instead of leaking it (the panicked resource is dropped, not reused).

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::OcrError;

pub(crate) struct Pool<T> {
    /// Idle items and the number of live items (idle + checked out).
    state: Mutex<(Vec<T>, usize)>,
    freed: Condvar,
    max: usize,
    wait: Duration,
}

/// Checked-out item; returned on drop, or forgotten (live count decremented) on unwind.
struct Slot<'a, T> {
    pool: &'a Pool<T>,
    item: Option<T>,
}

impl<T> Drop for Slot<'_, T> {
    fn drop(&mut self) {
        let mut g = self.pool.lock();
        match self.item.take() {
            Some(item) if !std::thread::panicking() => g.0.push(item),
            _ => g.1 -= 1,
        }
        drop(g);
        self.pool.freed.notify_one();
    }
}

impl<T> Pool<T> {
    pub(crate) fn new(max: usize, wait: Duration) -> Self {
        Self {
            state: Mutex::new((Vec::new(), 0)),
            freed: Condvar::new(),
            max: max.max(1),
            wait,
        }
    }

    /// The lock is only held for pop/push and never across user code, so poisoning cannot leave
    /// the state inconsistent; recover from it instead of propagating.
    fn lock(&self) -> MutexGuard<'_, (Vec<T>, usize)> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Runs `f` with an idle item, a newly created one (below `max`), or one freed within `wait`.
    pub(crate) fn with<R>(
        &self,
        create: impl FnOnce() -> Result<T, OcrError>,
        f: impl FnOnce(&mut T) -> Result<R, OcrError>,
    ) -> Result<R, OcrError> {
        // `None` (e.g. `Duration::MAX`): wait without a deadline instead of overflowing `Instant`.
        let deadline = Instant::now().checked_add(self.wait);
        let mut g = self.lock();
        let idle = loop {
            if let Some(item) = g.0.pop() {
                break Some(item);
            }
            if g.1 < self.max {
                g.1 += 1;
                break None;
            }
            let Some(deadline) = deadline else {
                g = self.freed.wait(g).unwrap_or_else(|e| e.into_inner());
                continue;
            };
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(OcrError::Engine(format!(
                    "no OCR session became free within {:?}",
                    self.wait
                )));
            }
            g = self
                .freed
                .wait_timeout(g, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        };
        drop(g);
        // From here on the live count includes this slot; the guard undoes it on error or unwind.
        let mut slot = Slot {
            pool: self,
            item: idle,
        };
        if slot.item.is_none() {
            slot.item = Some(create()?);
        }
        f(slot.item.as_mut().expect("set above"))
    }

    #[cfg(test)]
    fn live(&self) -> usize {
        self.lock().1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panics_and_failed_creation_do_not_leak_slots() {
        let pool = Pool::<u32>::new(1, Duration::from_secs(5));
        for _ in 0..3 {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pool.with(|| Ok(7), |_| -> Result<(), OcrError> { panic!("boom") })
            }));
            assert!(r.is_err());
            assert_eq!(
                pool.live(),
                0,
                "panicked item is dropped and its slot freed"
            );
        }
        let e = pool.with(|| Err(OcrError::Engine("no session".into())), |_| Ok(()));
        assert!(e.is_err());
        assert_eq!(pool.with(|| Ok(7), |v| Ok(*v)).unwrap(), 7);
        assert_eq!(pool.live(), 1, "healthy item is kept for reuse");
    }

    #[test]
    fn waiting_for_a_busy_pool_times_out() {
        let pool = Pool::<u32>::new(1, Duration::from_millis(50));
        let r = pool.with(
            || Ok(1),
            |_| pool.with(|| Ok(2), |_| Ok(())), // the only slot is held by the outer call
        );
        assert!(matches!(r, Err(OcrError::Engine(m)) if m.contains("no OCR session")));
        assert_eq!(pool.live(), 1);
    }

    #[test]
    fn unbounded_wait_does_not_overflow() {
        let pool = Pool::<u32>::new(1, Duration::MAX);
        assert_eq!(pool.with(|| Ok(3), |v| Ok(*v)).unwrap(), 3);
        assert_eq!(
            pool.with(|| Ok(4), |v| Ok(*v)).unwrap(),
            3,
            "idle item reused"
        );
    }
}
