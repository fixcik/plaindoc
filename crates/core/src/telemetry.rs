//! Spans and events for an external collector, over `tracing` behind the `telemetry` feature.
//! Without the feature every item is an inline no-op and `tracing` is not compiled in.
//!
//! The library installs no subscriber and sends nothing. The API takes only enums and integers,
//! so no call site can pass document text, warning messages, error strings, metadata or paths.
//!
//! Target `plaindoc`. Spans: `document` (INFO) > `page` (DEBUG, PDF) > `stage` (DEBUG). Events:
//! `limit_hit`, `ocr_unavailable`, `panic_isolated` (WARN); `parsed` and `warnings` (INFO) at the
//! end of each document. Work moved to another thread (PDF_POOL, rayon page workers, the timeout
//! and office threads) re-enters the caller's span taken with [`Span::current`].
//!
//! Hooking up: build the crates with `telemetry`, install your own `tracing::Subscriber` (filter
//! on target `plaindoc`), and open a request span around `Registry::parse_with_stats`; the
//! `document` span nests under it, and the returned `ParseStats` carries the same numbers.
//!
//! Fields:
//! - `document`: `input_bytes`, and at the end `format`, `outcome`, `pages`, `wall_us`.
//! - `page`: `index`. `stage`: `stage` (name of the stage).
//! - `limit_hit`: `kind`, `page` (optional). `panic_isolated`: `site`, `page` (optional).
//! - `parsed`: `format`, `outcome`, `wall_us`, `pages`, `pages_skipped`, `pages_failed`,
//!   `pages_ocr`, `words`, `cells`, `est_peak_bytes`. `pages_ocr` is `ParseStats::pages.ocr_run`
//!   (pages whose OCR ran, Mixed included), not `pages.ocr` (sections with `TextSource::Ocr`).
//! - `warnings` (one per kind): `kind`, `info`, `warn`, `loss`.
//!
//! No document content: codes and numbers only.

pub use imp::{
    Span, StageGuard, document, finished, limit_hit, ocr_unavailable, page, panic_isolated, stage,
    stage_guard,
};

/// Target of every span and event.
pub const TARGET: &str = "plaindoc";

/// Where a panic was caught.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PanicSite {
    /// `catch_parse_with_stats`: a parser panicked on the caller's (or timeout) thread.
    Parser,
    /// `pdf::runner::run_caught`: the PDF job on PDF_POOL.
    PdfJob,
    /// `tables::guarded`: the table detector of one page (the document survives).
    TableDetector,
    /// `office::worker::run_isolated`.
    OfficeWorker,
}

impl PanicSite {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            PanicSite::Parser => "parser",
            PanicSite::PdfJob => "pdf_job",
            PanicSite::TableDetector => "table_detector",
            PanicSite::OfficeWorker => "office_worker",
        }
    }
}

#[cfg(feature = "telemetry")]
mod imp {
    use super::{PanicSite, TARGET};
    use crate::{LimitKind, ParseStats, Stage};
    use tracing::field::Empty;

    /// A span handle (zero-sized without the feature).
    #[derive(Clone, Debug)]
    pub struct Span(tracing::Span);

    impl Default for Span {
        fn default() -> Self {
            Self(tracing::Span::none())
        }
    }

    impl Span {
        /// The calling thread's current span, to re-enter on another thread.
        #[inline]
        pub fn current() -> Self {
            Self(tracing::Span::current())
        }

        #[inline]
        pub fn in_scope<R>(&self, f: impl FnOnce() -> R) -> R {
            self.0.in_scope(f)
        }
    }

    /// Guard of an entered `stage` span (not `Send`: drop it on the thread that made it).
    #[must_use]
    pub struct StageGuard(#[allow(dead_code)] tracing::span::EnteredSpan);

    #[inline]
    pub fn document(input_bytes: u64) -> Span {
        Span(tracing::info_span!(
            target: TARGET,
            "document",
            input_bytes,
            format = Empty,
            outcome = Empty,
            pages = Empty,
            wall_us = Empty
        ))
    }

    #[inline]
    pub fn page(index: usize) -> Span {
        Span(tracing::debug_span!(target: TARGET, "page", index = index as u64))
    }

    #[inline]
    pub fn stage<R>(stage: Stage, f: impl FnOnce() -> R) -> R {
        tracing::debug_span!(target: TARGET, "stage", stage = stage.as_str()).in_scope(f)
    }

    #[inline]
    pub fn stage_guard(stage: Stage) -> StageGuard {
        StageGuard(tracing::debug_span!(target: TARGET, "stage", stage = stage.as_str()).entered())
    }

    #[inline]
    pub fn limit_hit(kind: LimitKind, page: Option<usize>) {
        tracing::warn!(target: TARGET, kind = kind.as_str(), page = page.map(|p| p as u64), "limit_hit");
    }

    #[inline]
    pub fn ocr_unavailable() {
        tracing::warn!(target: TARGET, "ocr_unavailable");
    }

    #[inline]
    pub fn panic_isolated(site: PanicSite, page: Option<usize>) {
        tracing::warn!(
            target: TARGET,
            site = site.as_str(),
            page = page.map(|p| p as u64),
            "panic_isolated"
        );
    }

    /// Records the summary on `doc` and emits `parsed` plus one `warnings` event per kind under it.
    pub fn finished(doc: &Span, s: &ParseStats) {
        let d = &doc.0;
        d.record("input_bytes", s.input_bytes);
        d.record("format", s.format.map(crate::Format::as_str));
        d.record("outcome", s.outcome.as_str());
        d.record("pages", s.pages.processed as u64);
        d.record("wall_us", s.wall_us);
        d.in_scope(|| {
            tracing::info!(
                target: TARGET,
                format = s.format.map(crate::Format::as_str),
                outcome = s.outcome.as_str(),
                wall_us = s.wall_us,
                pages = s.pages.processed as u64,
                pages_skipped = s.pages.skipped as u64,
                pages_failed = s.pages.failed as u64,
                pages_ocr = s.pages.ocr_run as u64,
                words = s.sizes.words,
                cells = s.sizes.cells,
                est_peak_bytes = s.est_peak_bytes,
                "parsed"
            );
            for (k, c) in &s.warnings {
                tracing::info!(
                    target: TARGET,
                    kind = k.as_str(),
                    info = c.info,
                    warn = c.warn,
                    loss = c.loss,
                    "warnings"
                );
            }
        });
    }
}

#[cfg(not(feature = "telemetry"))]
mod imp {
    use super::PanicSite;
    use crate::{LimitKind, ParseStats, Stage};

    #[derive(Clone, Copy, Debug, Default)]
    pub struct Span;

    impl Span {
        #[inline(always)]
        pub fn current() -> Self {
            Span
        }

        #[inline(always)]
        pub fn in_scope<R>(&self, f: impl FnOnce() -> R) -> R {
            f()
        }
    }

    #[must_use]
    pub struct StageGuard;

    #[inline(always)]
    pub fn document(_input_bytes: u64) -> Span {
        Span
    }
    #[inline(always)]
    pub fn page(_index: usize) -> Span {
        Span
    }
    #[inline(always)]
    pub fn stage<R>(_stage: Stage, f: impl FnOnce() -> R) -> R {
        f()
    }
    #[inline(always)]
    pub fn stage_guard(_stage: Stage) -> StageGuard {
        StageGuard
    }
    #[inline(always)]
    pub fn limit_hit(_kind: LimitKind, _page: Option<usize>) {}
    #[inline(always)]
    pub fn ocr_unavailable() {}
    #[inline(always)]
    pub fn panic_isolated(_site: PanicSite, _page: Option<usize>) {}
    #[inline(always)]
    pub fn finished(_doc: &Span, _s: &ParseStats) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Stage;

    #[test]
    fn wrappers_return_the_closure_value() {
        assert_eq!(stage(Stage::Text, || 7), 7);
        assert_eq!(Span::current().in_scope(|| 8), 8);
        assert_eq!(page(3).in_scope(|| document(1).in_scope(|| 9)), 9);
        let _g = stage_guard(Stage::Assemble);
    }

    #[cfg(not(feature = "telemetry"))]
    #[test]
    fn no_op_span_is_zero_sized() {
        assert_eq!(std::mem::size_of::<Span>(), 0);
        assert_eq!(std::mem::size_of::<StageGuard>(), 0);
    }

    #[test]
    fn panic_sites_are_codes() {
        for s in [
            PanicSite::Parser,
            PanicSite::PdfJob,
            PanicSite::TableDetector,
            PanicSite::OfficeWorker,
        ] {
            assert!(
                s.as_str()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
            );
        }
    }
}
