//! Tests of the page pipeline (`parse_opened`).

use super::*;
use crate::pdf::open::open_input;
use crate::pdf::tables::{TableDetector, TableInputCounts};
use crate::pdf::tests::builder_pdf_bytes;
use pdf_oxide::PdfDocument;
use plaindoc_core::{BBox, Block, Input, LimitKind, Stage};
use std::sync::atomic::Ordering;

fn opened(b: Vec<u8>) -> OpenedPdf {
    open_input(Input::Bytes(b), &ParseOptions::default()).unwrap()
}
fn ctx(allowed: bool) -> JobCtx {
    JobCtx {
        cancel: Arc::default(),
        page_parallel_allowed: allowed,
    }
}
fn par_opts(on: bool) -> ParseOptions {
    let mut o = ParseOptions::default();
    o.page_parallel = on;
    o
}
fn cfg(min: usize) -> PdfConfig {
    PdfConfig {
        min_parallel_pages: min,
        ..Default::default()
    }
}

#[test]
fn builds_one_section_per_page_with_headings() {
    let p = parse_opened(
        opened(builder_pdf_bytes(3)),
        &ParseOptions::default(),
        &PdfConfig::default(),
        &ctx(true),
    )
    .unwrap();
    let d = &p.document;
    assert_eq!(d.sections.len(), 3);
    assert_eq!(d.metadata.page_count, Some(3));
    // SP8: the paragraph repeated at the top of every page becomes a running-header note
    assert!(
        d.sections[2]
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Heading { level: 1, text, .. } if text == "Chapter 2"))
    );
    assert_eq!(d.sections[1].index, 1);
    assert!(!p.ran_parallel);
}

#[test]
fn parallel_equals_sequential_bytewise() {
    let seq = parse_opened(
        opened(builder_pdf_bytes(24)),
        &par_opts(false),
        &cfg(2),
        &ctx(true),
    )
    .unwrap();
    let par = parse_opened(
        opened(builder_pdf_bytes(24)),
        &par_opts(true),
        &cfg(2),
        &ctx(true),
    )
    .unwrap();
    assert!(par.ran_parallel && !seq.ran_parallel);
    assert_eq!(format!("{:?}", seq.document), format!("{:?}", par.document));
    assert_eq!(seq.document.to_json(), par.document.to_json());
}

#[test]
fn parallel_needs_option_permission_and_size() {
    let run = |o: bool, allowed: bool, min: usize| {
        parse_opened(
            opened(builder_pdf_bytes(8)),
            &par_opts(o),
            &cfg(min),
            &ctx(allowed),
        )
        .unwrap()
        .ran_parallel
    };
    assert!(run(true, true, 8));
    assert!(
        !run(false, true, 8),
        "ParseOptions.page_parallel = false (default, parse_many)"
    );
    assert!(!run(true, false, 8), "caller is a rayon worker");
    assert!(!run(true, true, 9), "below min_parallel_pages");
}

#[test]
fn max_pages_truncates_with_partial_content() {
    let mut o = ParseOptions::default();
    o.max_pages = Some(2);
    let d = parse_opened(
        opened(builder_pdf_bytes(5)),
        &o,
        &PdfConfig::default(),
        &ctx(true),
    )
    .unwrap()
    .document;
    assert_eq!((d.sections.len(), d.metadata.page_count), (2, Some(5)));
    assert!(
        d.warnings
            .iter()
            .any(|w| w.kind == WarningKind::PartialContent && w.level == Level::Warn)
    );
}

#[test]
fn max_pages_zero_processes_no_pages() {
    let mut o = ParseOptions::default();
    o.max_pages = Some(0);
    let p = parse_opened(
        opened(builder_pdf_bytes(3)),
        &o,
        &PdfConfig::default(),
        &ctx(true),
    )
    .unwrap();
    let d = &p.document;
    assert!(d.sections.is_empty() && p.decisions.is_empty());
    assert_eq!(d.metadata.page_count, Some(3));
    assert!(
        d.warnings
            .iter()
            .any(|w| w.kind == WarningKind::PartialContent && w.level == Level::Warn)
    );
    assert!(
        !d.warnings
            .iter()
            .any(|w| w.kind == WarningKind::EmptyOutput)
    );
}

#[test]
fn cancel_flag_stops_with_timeout() {
    let c = ctx(true);
    c.cancel.store(true, Ordering::Relaxed);
    assert!(matches!(
        parse_opened(
            opened(builder_pdf_bytes(3)),
            &ParseOptions::default(),
            &PdfConfig::default(),
            &c
        ),
        Err(ParseError::Timeout(_))
    ));
}

#[test]
fn table_detector_output_is_appended_and_errors_warn() {
    struct Fake;
    impl TableDetector for Fake {
        fn detect(&self, _: &PdfDocument, page: &StructuredPage) -> Result<Vec<DetectedTable>> {
            if page.page_index == 1 {
                return Err(ParseError::Internal("x".into()));
            }
            Ok(vec![DetectedTable::new(
                BBox::new(0.0, 0.0, 1.0, 1.0),
                Block::paragraph("TABLE", None),
            )])
        }
    }
    let c = PdfConfig {
        tables: Arc::new(Fake),
        ..Default::default()
    };
    let d = parse_opened(
        opened(builder_pdf_bytes(2)),
        &ParseOptions::default(),
        &c,
        &ctx(true),
    )
    .unwrap()
    .document;
    assert!(
        matches!(d.sections[0].blocks.last(), Some(Block::Paragraph { text, .. }) if text == "TABLE")
    );
    assert!(
        d.sections[1]
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::TableUncertain)
    );
}

#[test]
fn decisions_follow_pages() {
    let p = parse_opened(
        opened(builder_pdf_bytes(2)),
        &ParseOptions::default(),
        &PdfConfig::default(),
        &ctx(true),
    )
    .unwrap();
    assert_eq!(p.decisions, vec![PageDecision::UseTextLayer; 2]);
}

/// SP5 decides on the fork's glyph count (before the hidden-text filter): `Strip` must not
/// turn a golden-set page into an OCR or loss decision, nor change any non-hidden warning.
#[test]
fn strip_keeps_page_decisions_on_the_hidden_set() {
    strip_keeps_page_decisions(&PdfConfig::default());
}

/// The same with visibility facts: the phase-2 rules remove text in stage A too.
#[test]
fn strip_with_facts_keeps_page_decisions_on_the_hidden_set() {
    strip_keeps_page_decisions(&PdfConfig {
        visibility_facts: true,
        ..PdfConfig::default()
    });
}

fn strip_keeps_page_decisions(cfg: &PdfConfig) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf/hidden");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
        .collect();
    files.sort();
    assert_eq!(files.len(), 58);
    let run = |f: &std::path::Path, mode: HiddenText| {
        let mut o = ParseOptions::default();
        o.hidden_text = mode;
        let p = parse_opened(
            open_input(Input::Path(f.to_path_buf()), &o).unwrap(),
            &o,
            cfg,
            &ctx(true),
        )
        .unwrap();
        let kinds = |w: &[Warning]| {
            w.iter()
                .filter(|w| w.kind != WarningKind::HiddenContent)
                .map(|w| (w.kind, w.level))
                .collect::<Vec<_>>()
        };
        let d = &p.document;
        let pages: Vec<_> = d.sections.iter().map(|s| kinds(&s.warnings)).collect();
        (p.decisions, pages, kinds(&d.warnings))
    };
    for f in &files {
        assert_eq!(
            run(f, HiddenText::Off),
            run(f, HiddenText::Strip),
            "{}",
            f.display()
        );
    }
}

#[test]
fn stats_count_stages_and_sizes() {
    let p = parse_opened(
        opened(builder_pdf_bytes(3)),
        &ParseOptions::default(),
        &PdfConfig::default(),
        &ctx(true),
    )
    .unwrap();
    let s = &p.stats;
    for st in [Stage::Text, Stage::Tables, Stage::Diagnose, Stage::Assemble] {
        assert!(s.stages_us.contains_key(&st), "{st:?}: {:?}", s.stages_us);
    }
    assert_eq!(
        (s.pages.failed, s.pages.skipped, s.panics_isolated),
        (0, 0, 0)
    );
    // 3 lines per page: heading + two paragraphs
    assert!(s.sizes.spans >= 9, "{}", s.sizes.spans);
    // "Chapter i" 2 + 8 + 9 words per page; slack for how `extract_words` splits punctuation
    assert!(s.sizes.table_words >= 3 * 18, "{}", s.sizes.table_words);
    assert!(s.sizes.est_work_bytes > 0);
    assert!(s.limits.is_empty());
}

#[test]
fn stats_parallel_equal_sequential_without_timings() {
    let seq = parse_opened(
        opened(builder_pdf_bytes(24)),
        &par_opts(false),
        &cfg(2),
        &ctx(true),
    )
    .unwrap();
    let par = parse_opened(
        opened(builder_pdf_bytes(24)),
        &par_opts(true),
        &cfg(2),
        &ctx(true),
    )
    .unwrap();
    assert!(par.stats.page_parallel && !seq.stats.page_parallel);
    let mut a = par.stats.without_timings();
    a.page_parallel = false;
    assert_eq!(a, seq.stats.without_timings());
}

#[test]
fn max_pages_counts_skipped_pages_and_one_limit() {
    let mut o = ParseOptions::default();
    o.max_pages = Some(2);
    let s = parse_opened(
        opened(builder_pdf_bytes(5)),
        &o,
        &PdfConfig::default(),
        &ctx(true),
    )
    .unwrap()
    .stats;
    assert_eq!((s.pages.skipped, s.limits[&LimitKind::MaxPages]), (3, 1));
}

#[test]
fn detector_counts_are_summed_in_page_order() {
    struct Counted;
    impl TableDetector for Counted {
        fn detect(&self, _: &PdfDocument, _: &StructuredPage) -> Result<Vec<DetectedTable>> {
            Ok(Vec::new())
        }
        fn detect_counted(
            &self,
            _: &PdfDocument,
            page: &StructuredPage,
            c: &mut TableInputCounts,
        ) -> Result<Vec<DetectedTable>> {
            c.words = 7;
            c.prims = 3;
            if page.page_index % 2 == 1 {
                c.limit = Some(LimitKind::TableCells);
                c.panicked = page.page_index == 3;
                return Err(ParseError::Internal("x".into()));
            }
            Ok(Vec::new())
        }
    }
    let c = PdfConfig {
        tables: Arc::new(Counted),
        ..Default::default()
    };
    let s = parse_opened(
        opened(builder_pdf_bytes(4)),
        &ParseOptions::default(),
        &c,
        &ctx(true),
    )
    .unwrap()
    .stats;
    assert_eq!(
        (
            s.sizes.table_words,
            s.sizes.table_words_max,
            s.sizes.table_prims
        ),
        (28, 7, 12)
    );
    assert_eq!(
        (s.limits[&LimitKind::TableCells], s.panics_isolated),
        (2, 1)
    );
}

#[cfg(feature = "ocr")]
#[path = "pipeline_ocr_test.rs"]
mod ocr;
