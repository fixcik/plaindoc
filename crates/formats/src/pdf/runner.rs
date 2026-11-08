//! Runs PDF work on a dedicated big-stack rayon pool with panic catching, timeout and cancel.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use plaindoc_core::telemetry::{self, PanicSite};
use plaindoc_core::{ParseError, Result};

use super::pipeline::JobCtx;

pub(crate) const PDF_STACK_SIZE: usize = 32 * 1024 * 1024;

static PDF_POOL: OnceLock<std::result::Result<rayon::ThreadPool, String>> = OnceLock::new();

fn pdf_pool() -> Result<&'static rayon::ThreadPool> {
    PDF_POOL
        .get_or_init(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(std::thread::available_parallelism().map_or(4, |n| n.get()))
                .stack_size(PDF_STACK_SIZE)
                .thread_name(|i| format!("plaindoc-pdf-{i}"))
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| ParseError::Internal(format!("pdf pool: {e}")))
}

use crate::tables::panic_message;

/// Runs `job` with `ctx`, turning a panic into `ParseError::Internal` (and a `panic_isolated`
/// event in the current span).
fn run_caught<T>(job: impl FnOnce(&JobCtx) -> Result<T>, ctx: &JobCtx) -> Result<T> {
    std::panic::catch_unwind(AssertUnwindSafe(|| job(ctx))).unwrap_or_else(|p| {
        telemetry::panic_isolated(PanicSite::PdfJob, None);
        Err(ParseError::Internal(panic_message(p.as_ref())))
    })
}

/// Runs `job` on PDF_POOL, turning panics into `Internal`; on timeout sets the job's cancel flag
/// and returns `Timeout`. Cancel is cooperative: the job stops at its next check, so a hung job
/// keeps its pool thread. `page_parallel_allowed` = caller is not a rayon worker (captured before
/// dispatch).
///
/// Called from inside a PDF_POOL job: runs inline (no channel, no timeout, no page parallelism).
/// On the pool the job (and its panic event) runs in the caller's span.
pub(crate) fn run_isolated<T, F>(timeout: Option<Duration>, job: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&JobCtx) -> Result<T> + Send + 'static,
{
    let pool = pdf_pool()?;
    if pool.current_thread_index().is_some() {
        // Already on PDF_POOL: blocking on our own pool could starve it.
        let ctx = JobCtx {
            cancel: Arc::new(AtomicBool::new(false)),
            page_parallel_allowed: false,
        };
        return run_caught(job, &ctx);
    }
    let ctx = JobCtx {
        cancel: Arc::new(AtomicBool::new(false)),
        page_parallel_allowed: rayon::current_thread_index().is_none(),
    };
    let cancel = Arc::clone(&ctx.cancel);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    // Pool threads do not inherit the caller's span: re-enter it around the job.
    let span = telemetry::Span::current();
    pool.spawn(move || {
        // Without catch_unwind a panic in a spawned rayon job aborts the process.
        let _ = tx.send(span.in_scope(|| run_caught(job, &ctx)));
    });
    let exited = || ParseError::Internal("pdf job exited without a result".into());
    let got = match timeout {
        Some(limit) => rx.recv_timeout(limit).map_err(|e| match e {
            RecvTimeoutError::Timeout => {
                cancel.store(true, Ordering::Relaxed);
                ParseError::Timeout(limit)
            }
            RecvTimeoutError::Disconnected => exited(),
        }),
        None => rx.recv().map_err(|_| exited()),
    };
    got?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::open::map_pdf_error;
    use pdf_oxide::PdfDocument;
    use rayon::prelude::*;
    use std::time::Instant;

    #[test]
    fn returns_value_and_runs_on_pdf_pool() {
        let (on_pool, name) = run_isolated(None, |_| {
            let name = std::thread::current().name().map(str::to_owned);
            Ok((rayon::current_thread_index().is_some(), name))
        })
        .unwrap();
        assert!(on_pool);
        assert!(name.is_some_and(|n| n.starts_with("plaindoc-pdf-")));
    }

    #[test]
    fn panic_becomes_internal_with_message() {
        let r: Result<()> = run_isolated(None, |_| panic!("boom 42"));
        assert!(matches!(r, Err(ParseError::Internal(m)) if m.contains("boom 42")));
    }

    #[test]
    fn timeout_returns_promptly_and_sets_cancel() {
        let seen = Arc::new(AtomicBool::new(false));
        let s2 = seen.clone();
        let t = Instant::now();
        let r: Result<()> = run_isolated(Some(Duration::from_millis(50)), move |ctx| {
            while !ctx.cancel.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
            }
            s2.store(true, Ordering::Relaxed);
            Ok(())
        });
        assert!(matches!(r, Err(ParseError::Timeout(d)) if d == Duration::from_millis(50)));
        assert!(t.elapsed() < Duration::from_secs(1));
        std::thread::sleep(Duration::from_millis(100));
        assert!(seen.load(Ordering::Relaxed), "job must observe cancel");
    }

    #[test]
    fn page_parallel_allowed_only_outside_rayon() {
        assert!(run_isolated(None, |c| Ok(c.page_parallel_allowed)).unwrap());
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        assert!(
            !pool
                .install(|| run_isolated(None, |c| Ok(c.page_parallel_allowed)))
                .unwrap()
        );
    }

    #[test]
    fn nested_call_runs_inline() {
        let r = run_isolated(None, |_| {
            run_isolated(
                Some(Duration::from_secs(1)),
                |c| Ok(c.page_parallel_allowed),
            )
        })
        .unwrap();
        assert!(!r);
    }

    #[test]
    fn deep_recursion_fits_in_stack() {
        fn depth(n: u32) -> u32 {
            let pad = [0u8; 1024];
            if n == 0 {
                pad[0] as u32
            } else {
                depth(n - 1) + std::hint::black_box(pad)[1] as u32
            }
        }
        // debug frame ≈ 3 KiB: 6144 frames ≈ 18 MiB (> 8 MiB main stack, < 32 MiB); 12288 overflowed (measured)
        assert_eq!(run_isolated(None, |_| Ok(depth(6 * 1024))).unwrap(), 0);
    }

    #[test]
    fn no_deadlock_from_rayon_workers() {
        // Registry::parse_many + run_parser(timeout): rayon worker -> plain thread -> our parser.
        // extract_spans reaches the fork's internal par_iter (document.rs:16981).
        let bytes = crate::pdf::tests::builder_pdf_bytes(3);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        let results: Vec<Result<usize>> = pool.install(|| {
            (0..6)
                .into_par_iter()
                .map(|_| {
                    let b = bytes.clone();
                    std::thread::spawn(move || {
                        run_isolated(Some(Duration::from_secs(10)), move |_| {
                            let doc = PdfDocument::from_bytes(b).map_err(map_pdf_error)?;
                            Ok((0..3)
                                .map(|p| doc.extract_spans(p).map(|s| s.len()).unwrap_or(0))
                                .sum())
                        })
                    })
                    .join()
                    .unwrap()
                })
                .collect()
        });
        assert!(
            results.iter().all(|r| matches!(r, Ok(n) if *n > 0)),
            "{results:?}"
        );
    }
}
