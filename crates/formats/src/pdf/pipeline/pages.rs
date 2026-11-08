//! Stage A over the pages of one document on a bounded number of PDF_POOL threads.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use plaindoc_core::Result;
use rayon::prelude::*;

/// Pages of one document in flight at once when `ParseOptions::page_threads` is `None`.
pub(super) const DEFAULT_PAGE_THREADS: usize = 6;

/// Workers for `pages` pages: `requested` (0 counts as 1) or [`DEFAULT_PAGE_THREADS`], at most
/// `pages`, at least 1. More workers than pool threads just wait for a free thread, so
/// PDF_POOL (one thread per core) caps the default at min(cores, 6).
pub(super) fn page_workers(requested: Option<usize>, pages: usize) -> usize {
    requested.unwrap_or(DEFAULT_PAGE_THREADS).min(pages).max(1)
}

/// Runs `f` for every page of `pages` on at most `workers` tasks of the current rayon pool (no
/// new pool, no `install`): each task takes the next page from a shared counter. Results come
/// back in page order. Pages are taken in increasing order and a task finishes the page it holds,
/// so after a failure every earlier page has a result and the error returned is the first one
/// in page order, as in the sequential loop.
pub(super) fn par_pages<T: Send>(
    pages: Range<usize>,
    workers: usize,
    f: impl Fn(usize) -> Result<T> + Sync,
) -> Result<Vec<T>> {
    let next = AtomicUsize::new(pages.start);
    let failed = AtomicBool::new(false);
    let mut done: Vec<(usize, Result<T>)> = (0..workers)
        .into_par_iter()
        .with_max_len(1)
        .flat_map_iter(|_| {
            let mut out = Vec::new();
            while !failed.load(Ordering::Relaxed) {
                let p = next.fetch_add(1, Ordering::Relaxed);
                if p >= pages.end {
                    break;
                }
                let r = f(p);
                if r.is_err() {
                    failed.store(true, Ordering::Relaxed);
                }
                out.push((p, r));
            }
            out
        })
        .collect();
    done.sort_unstable_by_key(|&(p, _)| p);
    done.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
#[path = "pages_test.rs"]
mod tests;
