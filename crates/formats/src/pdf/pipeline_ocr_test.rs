//! Tests of the page pipeline with the OCR pass.

use super::*;
use crate::ocr::OcrConfig;
use crate::pdf::ocr_stage::tests::{InkSpy, rotated_offset_pdf};
use plaindoc_core::{OcrMode, TextSource};
use plaindoc_ocr::{FakeEngine, TextBox};

const SCAN: &[u8] = include_bytes!("../../tests/fixtures/pdf/scan.pdf");
const IMGTEXT: &[u8] = include_bytes!("../../tests/fixtures/pdf/imgtext.pdf");

fn with_fake(items: Vec<(TextBox, &str, f32)>) -> (PdfConfig, Arc<FakeEngine>) {
    let e = Arc::new(FakeEngine::new(
        items
            .into_iter()
            .map(|(b, t, c)| (b, t.to_string(), c))
            .collect(),
    ));
    let c = PdfConfig {
        ocr: OcrConfig::with_engine(e.clone()),
        ..Default::default()
    };
    (c, e)
}
fn opts(mode: OcrMode) -> ParseOptions {
    let mut o = ParseOptions::default();
    o.ocr = mode;
    o
}
fn kinds(s: &Section) -> Vec<(WarningKind, Level)> {
    s.warnings.iter().map(|w| (w.kind, w.level)).collect()
}
fn paragraphs(s: &Section) -> Vec<&str> {
    s.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn scan_page_is_replaced_by_ocr_and_loss_becomes_info() {
    let (c, e) = with_fake(vec![(
        TextBox::from_aabb(100.0, 100.0, 900.0, 160.0, 0.9),
        "Акт приёмки",
        0.92,
    )]);
    let p = parse_opened(opened(SCAN.to_vec()), &opts(OcrMode::Auto), &c, &ctx(true)).unwrap();
    let s = &p.document.sections[0];
    assert_eq!(e.detect_calls(), 1);
    assert_eq!(s.source, TextSource::Ocr);
    assert!(
        matches!(&s.blocks[..], [Block::Paragraph { text, bbox: Some(_), .. }] if text == "Акт приёмки")
    );
    assert_eq!(
        kinds(s),
        vec![
            (WarningKind::NoTextLayer, Level::Info),
            (WarningKind::OcrUsed, Level::Info)
        ]
    );
    assert!((s.quality - 0.92).abs() < 1e-6);
    assert!(
        !p.document
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::OcrUnavailable)
    );
}

#[test]
fn never_skips_ocr_and_failure_keeps_loss() {
    let (c, e) = with_fake(vec![]);
    let p = parse_opened(opened(SCAN.to_vec()), &opts(OcrMode::Never), &c, &ctx(true)).unwrap();
    assert_eq!(e.detect_calls(), 0);
    assert_eq!(
        kinds(&p.document.sections[0]),
        vec![(WarningKind::NoTextLayer, Level::Loss)]
    );
    let c = PdfConfig {
        ocr: OcrConfig::with_engine(Arc::new(FakeEngine::failing())),
        ..Default::default()
    };
    let p = parse_opened(opened(SCAN.to_vec()), &opts(OcrMode::Auto), &c, &ctx(true)).unwrap();
    assert_eq!(
        kinds(&p.document.sections[0]),
        vec![
            (WarningKind::NoTextLayer, Level::Loss),
            (WarningKind::PartialContent, Level::Warn)
        ]
    );
}

#[test]
fn no_engine_reports_unavailable_info() {
    let mut o = opts(OcrMode::Auto);
    o.ocr_max_pages = Some(0); // no engine: the limit does not apply
    let p = parse_opened(opened(SCAN.to_vec()), &o, &PdfConfig::default(), &ctx(true)).unwrap();
    let w = p
        .document
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::OcrUnavailable)
        .unwrap();
    assert_eq!(w.level, Level::Info);
    assert!(w.message.contains("no OCR engine"), "{}", w.message);
    assert!(
        !p.document
            .warnings
            .iter()
            .any(|w| w.message.contains("OCR limited"))
    );
}

#[test]
fn auto_with_classifier_off_does_not_ocr_image_text_pages() {
    // SP2 gives imgtext.pdf `RunOcrAndMerge`; with the classifier off that is not acted on.
    let (mut c, e) = with_fake(vec![(
        TextBox::from_aabb(200.0, 2000.0, 900.0, 2060.0, 0.9),
        "stamp text",
        0.9,
    )]);
    c.diagnostics.need_ocr = None;
    let base = parse_opened(
        opened(IMGTEXT.to_vec()),
        &opts(OcrMode::Never),
        &c,
        &ctx(true),
    )
    .unwrap();
    assert_eq!(base.decisions, vec![PageDecision::RunOcrAndMerge]);
    let p = parse_opened(
        opened(IMGTEXT.to_vec()),
        &opts(OcrMode::Auto),
        &c,
        &ctx(true),
    )
    .unwrap();
    assert_eq!(e.detect_calls(), 0);
    assert_eq!(base.document.to_json(), p.document.to_json());
}

#[test]
fn merge_ocr_lowers_the_image_text_warn_to_info() {
    let done = |found: bool| PageOcr::Done {
        blocks: if found {
            vec![Block::paragraph("stamp text", None)]
        } else {
            Vec::new()
        },
        warnings: if found {
            Vec::new()
        } else {
            vec![Warning::new(WarningKind::OcrEmpty, Level::Info, "no text")]
        },
        quality: 1.0,
        replace: false,
        mismatch: None,
    };
    for (found, want) in [(true, Level::Info), (false, Level::Warn)] {
        let mut s = Section::new(SectionKind::Page, 0);
        s.warnings.push(Warning::new(
            WarningKind::ImageTextPage,
            Level::Warn,
            "page has text in images that is missing from the text layer",
        ));
        let ocr = (done(found), PageDecision::RunOcrAndMerge);
        apply_ocr(&mut s, Some(&ocr), HiddenText::Strip, &mut Vec::new());
        assert_eq!(s.warnings[0].level, want, "found {found}");
    }
}

#[test]
fn a_swapped_layer_takes_its_table_warnings_along() {
    use crate::pdf::ocr_stage::LayerMismatch;
    let done = PageOcr::Done {
        blocks: Vec::new(),
        warnings: Vec::new(),
        quality: 0.9,
        replace: false,
        mismatch: Some(LayerMismatch {
            check: crate::pdf::hidden_ocr::Unread {
                unread: 30,
                counted: 40,
                exhausted: false,
            },
            blocks: vec![Block::paragraph("what the scan shows", None)],
        }),
    };
    let ocr = (done, PageDecision::UseTextLayer);
    for (mode, swapped) in [(HiddenText::Strip, true), (HiddenText::Flag, false)] {
        let mut s = Section::new(SectionKind::Page, 1);
        s.warnings.push(Warning::new(
            WarningKind::TableUncertain,
            Level::Info,
            "table continues from page 1 (fragments are not merged)",
        ));
        assert_eq!(
            apply_ocr(&mut s, Some(&ocr), mode, &mut Vec::new()),
            swapped
        );
        let table = s
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::TableUncertain);
        assert_eq!(table, !swapped, "{mode:?}");
        assert_eq!(paragraphs(&s) == ["what the scan shows"], swapped);
    }
}

#[test]
fn always_merges_new_words_and_drops_duplicates_of_the_layer() {
    // builder page: "Chapter 0" heading at y=720pt -> raster y ~ (792-720)*200/72 = 200 px
    let (c, _) = with_fake(vec![
        (
            TextBox::from_aabb(190.0, 150.0, 700.0, 215.0, 0.9),
            "Chapter 0",
            0.9,
        ),
        (
            TextBox::from_aabb(200.0, 2000.0, 900.0, 2060.0, 0.9),
            "stamp text",
            0.9,
        ),
        (
            TextBox::from_aabb(200.0, 20.0, 900.0, 60.0, 0.9),
            "header note",
            0.9,
        ),
    ]);
    let p = parse_opened(
        opened(builder_pdf_bytes(1)),
        &opts(OcrMode::Always),
        &c,
        &ctx(true),
    )
    .unwrap();
    let s = &p.document.sections[0];
    assert_eq!(p.decisions, vec![PageDecision::UseTextLayer]);
    assert_eq!(s.source, TextSource::Mixed);
    let added: Vec<_> = paragraphs(s)
        .into_iter()
        .filter(|t| *t == "stamp text" || *t == "Chapter 0")
        .collect();
    assert_eq!(added, vec!["stamp text"]);
    // Placed by the top edge: the header above the layer's text, the stamp below it.
    assert!(
        matches!(s.blocks.first(), Some(Block::Paragraph { text, .. }) if text == "header note"),
        "{:?}",
        s.blocks
    );
    assert!(matches!(s.blocks.last(), Some(Block::Paragraph { text, .. }) if text == "stamp text"));
}

#[test]
fn rotated_page_merge_has_no_duplicates() {
    // Real PDF with /Rotate 90 and an offset MediaBox through parse_opened (I8).
    let mut c = PdfConfig {
        ocr: OcrConfig::with_engine(Arc::new(InkSpy)),
        ..Default::default()
    };
    c.ocr.params.deskew = false; // a single short line: skew estimate is undefined
    let p = parse_opened(
        opened(rotated_offset_pdf()),
        &opts(OcrMode::Always),
        &c,
        &ctx(true),
    )
    .unwrap();
    let s = &p.document.sections[0];
    assert_eq!(
        s.source,
        TextSource::Mixed,
        "merge path, not replace: {:?} {:?} {:?}",
        p.decisions,
        s.warnings,
        s.blocks
    );
    // All text blocks: a layer line laid out as a heading must not hide an OCR copy.
    let texts: Vec<&str> = s
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph { text, .. } | Block::Heading { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts.iter().filter(|t| t.contains("Chapter 0")).count(),
        1,
        "layer text once: {texts:?}"
    );
    assert!(texts.contains(&"stamp text"), "new text is kept");
}

#[test]
fn ocr_parallel_equals_sequential_bytewise() {
    let (c, _) = with_fake(vec![(
        TextBox::from_aabb(200.0, 2000.0, 900.0, 2060.0, 0.9),
        "stamp",
        0.9,
    )]);
    let c = PdfConfig {
        min_parallel_pages: 2,
        ..c
    };
    let mut o = opts(OcrMode::Always);
    let seq = parse_opened(opened(builder_pdf_bytes(6)), &o, &c, &ctx(true)).unwrap();
    o.page_parallel = true;
    let par = parse_opened(opened(builder_pdf_bytes(6)), &o, &c, &ctx(true)).unwrap();
    assert!(par.ran_parallel && !seq.ran_parallel);
    assert_eq!(seq.document.to_json(), par.document.to_json());
}

#[test]
fn ocr_max_pages_limits_and_warns() {
    let (c, e) = with_fake(vec![(
        TextBox::from_aabb(200.0, 2000.0, 900.0, 2060.0, 0.9),
        "stamp",
        0.9,
    )]);
    let mut o = opts(OcrMode::Always);
    o.ocr_max_pages = Some(2);
    let p = parse_opened(opened(builder_pdf_bytes(5)), &o, &c, &ctx(true)).unwrap();
    assert_eq!(e.detect_calls(), 2);
    assert!(
        p.document
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::PartialContent
                && w.message.contains("OCR limited to 2 of 5 pages"))
    );
    // The same first two pages are OCRed whatever the thread count.
    let sources: Vec<_> = p.document.sections.iter().map(|s| s.source).collect();
    assert_eq!(sources[..2], [TextSource::Mixed; 2]);
    assert!(!sources[2..].contains(&TextSource::Mixed), "{sources:?}");
    let c = PdfConfig {
        min_parallel_pages: 2,
        ..c
    };
    let seq = parse_opened(opened(builder_pdf_bytes(5)), &o, &c, &ctx(true)).unwrap();
    o.page_parallel = true;
    let par = parse_opened(opened(builder_pdf_bytes(5)), &o, &c, &ctx(true)).unwrap();
    assert!(par.ran_parallel && !seq.ran_parallel);
    assert_eq!(seq.document.to_json(), par.document.to_json());
    assert_eq!(p.document.to_json(), par.document.to_json());
}

#[test]
fn ocr_stats_count_pages_limit_and_raster() {
    let (c, _) = with_fake(vec![(
        TextBox::from_aabb(200.0, 2000.0, 900.0, 2060.0, 0.9),
        "stamp",
        0.9,
    )]);
    let mut o = opts(OcrMode::Always);
    o.ocr_max_pages = Some(2);
    let s = parse_opened(opened(builder_pdf_bytes(5)), &o, &c, &ctx(true))
        .unwrap()
        .stats;
    let pg = &s.pages;
    assert_eq!(
        (pg.ocr_wanted, pg.ocr_run, pg.ocr_skipped, pg.ocr_failed),
        (5, 2, 3, 0)
    );
    assert_eq!(s.limits[&LimitKind::OcrMaxPages], 1);
    assert!(s.stages_us.contains_key(&Stage::Ocr));
    // two OCR pages -> at most two rasters at once, never a saturated value
    assert!(
        s.sizes.est_work_bytes < u64::MAX / 2,
        "{}",
        s.sizes.est_work_bytes
    );
    let never = parse_opened(
        opened(builder_pdf_bytes(5)),
        &opts(OcrMode::Never),
        &c,
        &ctx(true),
    )
    .unwrap()
    .stats;
    // letter page at `page_dpi`: at least w*h*7 bytes more than without OCR
    let dpi = u64::from(c.ocr.page_dpi);
    let px = (612 * dpi / 72) * (792 * dpi / 72);
    assert!(
        s.sizes.est_work_bytes >= never.sizes.est_work_bytes + 7 * px / 2,
        "{} vs {}",
        s.sizes.est_work_bytes,
        never.sizes.est_work_bytes
    );
}

#[test]
fn ocr_page_timeout_is_a_page_limit() {
    let (c, _) = with_fake(
        (0..40)
            .map(|i| {
                (
                    TextBox::from_aabb(
                        100.0,
                        100.0 + 40.0 * i as f32,
                        900.0,
                        130.0 + 40.0 * i as f32,
                        0.9,
                    ),
                    "x",
                    0.9,
                )
            })
            .collect(),
    );
    let mut o = opts(OcrMode::Auto);
    o.ocr_page_timeout = Some(std::time::Duration::ZERO);
    let s = parse_opened(opened(SCAN.to_vec()), &o, &c, &ctx(true))
        .unwrap()
        .stats;
    assert_eq!(s.limits.get(&LimitKind::OcrPageTimeout), Some(&1));
}

#[test]
fn missing_engine_sets_ocr_unavailable() {
    let s = parse_opened(
        opened(SCAN.to_vec()),
        &opts(OcrMode::Auto),
        &PdfConfig::default(),
        &ctx(true),
    )
    .unwrap()
    .stats;
    assert!(s.ocr_unavailable);
    assert_eq!(
        (s.pages.ocr_wanted, s.pages.ocr_run, s.pages.ocr_skipped),
        (1, 0, 1)
    );
}

#[test]
fn page_timeout_gives_partial_content() {
    let (c, _) = with_fake(
        (0..40)
            .map(|i| {
                (
                    TextBox::from_aabb(
                        100.0,
                        100.0 + 40.0 * i as f32,
                        900.0,
                        130.0 + 40.0 * i as f32,
                        0.9,
                    ),
                    "x",
                    0.9,
                )
            })
            .collect(),
    );
    let mut o = opts(OcrMode::Auto);
    o.ocr_page_timeout = Some(std::time::Duration::ZERO);
    let p = parse_opened(opened(SCAN.to_vec()), &o, &c, &ctx(true)).unwrap();
    assert!(
        p.document.sections[0]
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::PartialContent && w.message.contains("OCR timed out")),
        "{:?}",
        p.document.sections[0].warnings
    );
}

#[test]
fn unavailable_engine_stops_rendering_after_the_first_batch() {
    struct Gone(std::sync::atomic::AtomicUsize);
    impl plaindoc_ocr::OcrEngine for Gone {
        fn detect(
            &self,
            _: &plaindoc_ocr::Image,
        ) -> std::result::Result<Vec<TextBox>, plaindoc_ocr::OcrError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(plaindoc_ocr::OcrError::ModelsUnavailable(
                "no models".into(),
            ))
        }
        fn recognize(
            &self,
            _: &plaindoc_ocr::Image,
            _: &[TextBox],
        ) -> std::result::Result<Vec<plaindoc_ocr::Recognized>, plaindoc_ocr::OcrError> {
            Ok(Vec::new())
        }
        fn max_concurrency(&self) -> usize {
            1
        }
    }
    let e = Arc::new(Gone(Default::default()));
    let c = PdfConfig {
        ocr: OcrConfig::with_engine(e.clone()),
        ..Default::default()
    };
    let p = parse_opened(
        opened(builder_pdf_bytes(4)),
        &opts(OcrMode::Always),
        &c,
        &ctx(true),
    )
    .unwrap();
    assert_eq!(
        e.0.load(Ordering::Relaxed),
        1,
        "one page tried, the rest skipped"
    );
    let w = p
        .document
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::OcrUnavailable)
        .unwrap();
    assert!(w.message.contains("no models"), "{}", w.message);
}

#[test]
fn replaced_page_feeds_scan_table_inputs_and_places_the_table() {
    use crate::pdf::tables::ScanTableInputs;
    use plaindoc_core::Cell;
    struct Spy(std::sync::Mutex<Vec<ScanTableInputs>>);
    impl TableDetector for Spy {
        fn detect(&self, _: &PdfDocument, _: &StructuredPage) -> Result<Vec<DetectedTable>> {
            Ok(vec![])
        }
        fn detect_scan(&self, i: &ScanTableInputs) -> Result<Vec<DetectedTable>> {
            self.0.lock().unwrap().push(i.clone());
            // one 1x1 table over the first word, in the aligned frame of the inputs
            let b = i.words[0].bbox;
            Ok(vec![DetectedTable::new(
                b,
                Block::Table {
                    rows: vec![vec![Cell::new(i.words[0].text.clone())]],
                    header_rows: 0,
                    bbox: Some(b),
                },
            )])
        }
    }
    let spy = Arc::new(Spy(Default::default()));
    // A second line below: "Акт" alone is under `has_text(5, ..)` and the page would stay `OcrEmpty`.
    let (mut c, _) = with_fake(vec![
        (
            TextBox::from_aabb(100.0, 100.0, 900.0, 160.0, 0.9),
            "Акт",
            0.92,
        ),
        (
            TextBox::from_aabb(100.0, 300.0, 900.0, 360.0, 0.9),
            "приёмки",
            0.92,
        ),
    ]);
    c.tables = spy.clone();
    let p = parse_opened(opened(SCAN.to_vec()), &opts(OcrMode::Auto), &c, &ctx(true)).unwrap();
    let got = spy.0.lock().unwrap();
    assert_eq!(got.len(), 1);
    let i = &got[0];
    assert!(i.page_width > 0.0 && i.page_height > 0.0);
    let w = &i.words[0];
    assert_eq!((w.text.as_str(), w.char_x.len()), ("Акт", 3));
    assert!(
        w.bbox.y > 0.0 && w.bbox.y + w.bbox.h <= i.page_height && w.font_size > 0.0,
        "{w:?}"
    );
    let s = &p.document.sections[0];
    assert!(
        s.blocks
            .iter()
            .any(|b| matches!(b, Block::Table { bbox: Some(_), .. }))
    );
    assert!(
        !s.blocks
            .iter()
            .any(|b| matches!(b, Block::Paragraph { text, .. } if text == "Акт")),
        "table text not repeated"
    );
    assert!(paragraphs(s).contains(&"приёмки"), "{:?}", s.blocks);
}
