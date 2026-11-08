//! Worker thread with a large stack for `office_oxide` conversion and IR mapping.

use plaindoc_core::telemetry::{self, PanicSite};
use plaindoc_core::{ParseError, Result};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Stack size of the worker thread (IR nesting reaches 100 levels).
pub(crate) const WORKER_STACK: usize = 16 * 1024 * 1024;

/// Runs `f` on a dedicated thread with a [`WORKER_STACK`] stack; a panic becomes `Internal`.
pub(crate) fn run_isolated<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    // The worker thread does not inherit the caller's span: re-enter it there.
    let span = telemetry::Span::current();
    let handle = std::thread::Builder::new()
        .name("plaindoc-office".into())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            span.in_scope(|| {
                catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|p| {
                    telemetry::panic_isolated(PanicSite::OfficeWorker, None);
                    let msg = p
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| p.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("unknown panic");
                    Err(ParseError::Internal(format!(
                        "office worker panicked: {msg}"
                    )))
                })
            })
        })
        .map_err(|e| ParseError::Internal(format!("cannot spawn office worker: {e}")))?;
    handle
        .join()
        .unwrap_or_else(|_| Err(ParseError::Internal("office worker thread failed".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_value() {
        assert_eq!(run_isolated(|| Ok(7)).unwrap(), 7);
    }

    #[test]
    fn panic_becomes_internal_with_message() {
        let r: Result<()> = run_isolated(|| panic!("boom"));
        assert!(matches!(r, Err(ParseError::Internal(m)) if m.contains("boom")));
    }

    #[test]
    fn has_large_stack() {
        fn burn(n: usize) -> usize {
            let buf = std::hint::black_box([n as u8; 4096]);
            if n == 0 {
                buf[0] as usize
            } else {
                burn(n - 1) + buf[1] as usize
            }
        }
        // ~1000 frames * 4 KiB ≈ 4 MiB: overflows a 2 MiB thread, fits in 16 MiB
        assert!(run_isolated(|| Ok(burn(1000))).is_ok());
    }

    #[test]
    fn errors_pass_through() {
        let r: Result<()> = run_isolated(|| Err(ParseError::Encrypted));
        assert!(matches!(r, Err(ParseError::Encrypted)));
    }
}
