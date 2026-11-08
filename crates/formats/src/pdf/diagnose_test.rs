use super::*;
use crate::pdf::need_ocr::{InkFeatures, TextFeatures};

const OK: GarbageScore = GarbageScore {
    tokens: 100,
    ratio: 0.01,
};
const BAD: GarbageScore = GarbageScore {
    tokens: 39,
    ratio: 0.54,
}; // truncated ru.pdf
const NONE: GarbageScore = GarbageScore {
    tokens: 0,
    ratio: 0.0,
};
fn facts(
    kind: FactKind,
    no_text_layer: bool,
    glyphs: usize,
    image_area_ratio: f32,
    garbled_ratio: f32,
) -> PageFacts {
    PageFacts {
        kind,
        no_text_layer,
        glyphs,
        image_area_ratio,
        garbled_ratio,
        vector_path_count: 0,
    }
}
fn kinds(d: &PageDiagnosis) -> Vec<(WarningKind, Level)> {
    d.warnings.iter().map(|w| (w.kind, w.level)).collect()
}
fn run(f: PageFacts, g: GarbageScore) -> PageDiagnosis {
    diagnose_page(
        &ClassifyOutcome::Facts(f),
        None,
        g,
        0,
        0,
        &DiagnosticsConfig::default(),
    )
}

#[test]
fn scan_without_text_is_loss_and_ocr() {
    let d = run(facts(FactKind::Scanned, true, 0, 0.24, 0.0), NONE);
    assert_eq!(kinds(&d), vec![(WarningKind::NoTextLayer, Level::Loss)]);
    assert_eq!((d.decision, d.quality), (PageDecision::RunOcr, 0.0));
}
#[test]
fn textlayer_mojibake_is_suspected_garbage_loss() {
    let d = run(facts(FactKind::TextLayer, false, 359, 0.0, 0.0), BAD);
    assert_eq!(
        kinds(&d),
        vec![(WarningKind::SuspectedGarbage, Level::Loss)]
    );
    assert_eq!(d.decision, PageDecision::RunOcr);
}
#[test]
fn engine_only_glyph_suspicion_is_info() {
    // odl 01030000000018
    let d = run(facts(FactKind::Scanned, false, 2456, 0.0, 0.01), OK);
    assert_eq!(
        kinds(&d),
        vec![(WarningKind::SuspectedGarbage, Level::Info)]
    );
    assert_eq!(d.decision, PageDecision::UseTextLayer);
}
#[test]
fn full_page_image_with_text_is_warn() {
    // odl 01030000000198
    let d = run(
        facts(FactKind::Scanned, false, 145, 1.0, 0.0),
        GarbageScore {
            tokens: 16,
            ratio: 0.0,
        },
    );
    assert_eq!(kinds(&d), vec![(WarningKind::ImageTextPage, Level::Warn)]);
    assert_eq!(d.decision, PageDecision::RunOcrAndMerge);
}
#[test]
fn image_text_page_is_info_and_merge() {
    let d = run(facts(FactKind::ImageText, false, 46, 0.16, 0.0), OK);
    assert_eq!(kinds(&d), vec![(WarningKind::ImageTextPage, Level::Info)]);
    assert_eq!((d.decision, d.quality), (PageDecision::RunOcrAndMerge, 1.0));
}
#[test]
fn empty_page_is_empty_output_info() {
    let d = run(facts(FactKind::Empty, false, 0, 0.0, 0.0), NONE);
    assert_eq!(kinds(&d), vec![(WarningKind::EmptyOutput, Level::Info)]);
}
#[test]
fn clean_text_layer_has_no_warnings_and_few_tokens_never_garbage() {
    assert!(
        run(facts(FactKind::TextLayer, false, 2000, 0.0, 0.0), OK)
            .warnings
            .is_empty()
    );
    assert!(
        run(
            facts(FactKind::TextLayer, false, 20, 0.0, 0.0),
            GarbageScore {
                tokens: 19,
                ratio: 1.0
            }
        )
        .warnings
        .is_empty()
    );
}
#[test]
fn dropped_artifacts_add_info_and_failures_are_partial() {
    let d = diagnose_page(
        &ClassifyOutcome::Facts(facts(FactKind::TextLayer, false, 9, 0.0, 0.0)),
        None,
        OK,
        2,
        6,
        &DiagnosticsConfig::default(),
    );
    assert_eq!(kinds(&d), vec![(WarningKind::ContentDropped, Level::Info)]);
    assert!(
        d.warnings[0]
            .message
            .contains("2 artifact regions (6 chars)")
    );
    let f = diagnose_page(
        &ClassifyOutcome::Failed("boom".into()),
        None,
        OK,
        0,
        0,
        &DiagnosticsConfig::default(),
    );
    assert_eq!(kinds(&f), vec![(WarningKind::PartialContent, Level::Warn)]);
    let e = extraction_failed("bad stream");
    assert_eq!(
        (kinds(&e), e.decision, e.quality),
        (
            vec![(WarningKind::PartialContent, Level::Loss)],
            PageDecision::Reject,
            0.0
        )
    );
}
/// The SP2 rules table (classifier switched off).
fn sp2_cfg() -> DiagnosticsConfig {
    DiagnosticsConfig {
        need_ocr: None,
        ..Default::default()
    }
}
#[test]
fn default_is_the_classifier_with_the_bundled_table() {
    let c = DiagnosticsConfig::default();
    assert_eq!(c.need_ocr, Some(NeedOcrConfig::RECALL));
    assert_eq!(c.bigrams, Bigrams::bundled());
    assert_eq!(bigrams_missing(&c).is_some(), c.bigrams.is_none());
    // Signals are ignored while the switch is off.
    let s = on_sig("", 1.0, Some(400));
    let f = ClassifyOutcome::Facts(facts(FactKind::TextLayer, false, 2000, 0.0, 0.0));
    let d = diagnose_page(&f, Some(&s), OK, 0, 0, &sp2_cfg());
    assert!(d.warnings.is_empty() && d.decision == PageDecision::UseTextLayer);
}
#[test]
fn classifier_without_bigrams_is_one_info() {
    let c = DiagnosticsConfig {
        bigrams: None,
        ..Default::default()
    };
    let w = bigrams_missing(&c).unwrap();
    assert_eq!(
        (w.kind, w.level),
        (WarningKind::SuspectedGarbage, Level::Info)
    );
    assert!(bigrams_missing(&sp2_cfg()).is_none());
}

// Need-OCR classifier switched on (`need_ocr = Some(RECALL)`).
const PROSE: &str =
    "The committee reviewed the annual report and approved the budget for the next fiscal year.";
fn on_sig(text: &str, cover: f32, gcc: Option<u32>) -> PageSignals {
    PageSignals {
        text: TextFeatures::from_text(text, None),
        image_cover: cover,
        ink: gcc.map(|g| InkFeatures {
            glyph_cc_outside: g,
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn on_cfg() -> DiagnosticsConfig {
    DiagnosticsConfig {
        need_ocr: Some(NeedOcrConfig::RECALL),
        ..Default::default()
    }
}
fn on_facts() -> ClassifyOutcome {
    ClassifyOutcome::Facts(facts(FactKind::TextLayer, false, 0, 0.0, 0.0))
}
fn on(s: PageSignals, g: GarbageScore) -> PageDiagnosis {
    diagnose_page(&on_facts(), Some(&s), g, 0, 0, &on_cfg())
}

#[test]
fn on_scan_without_text_is_loss_and_ocr() {
    let d = on(on_sig("", 1.0, Some(400)), OK);
    assert_eq!(kinds(&d), vec![(WarningKind::NoTextLayer, Level::Loss)]);
    assert_eq!((d.decision, d.quality), (PageDecision::RunOcr, 0.0));
    assert!(
        d.warnings[0].message.contains(
            "page text exists only as an image (0 chars, 1.00 image cover, \
             400 glyph-like components outside the layer, 100 tokens, 0.01 mojibake ratio)"
        ),
        "{}",
        d.warnings[0].message
    );
}
#[test]
fn on_blank_is_empty_output_info() {
    let d = on(on_sig("", 0.0, Some(2)), OK);
    assert_eq!(
        (kinds(&d), d.decision),
        (
            vec![(WarningKind::EmptyOutput, Level::Info)],
            PageDecision::UseTextLayer
        )
    );
}
#[test]
fn on_clean_layer_with_images_has_no_warning() {
    let d = on(on_sig(PROSE, 0.3, Some(10)), OK);
    assert!(d.warnings.is_empty() && d.decision == PageDecision::UseTextLayer);
}
#[test]
fn on_some_ink_in_images_below_the_ocr_threshold_is_info() {
    let d = on(on_sig(PROSE, 0.24, Some(49)), OK); // imgtext.pdf: 49 components < g_abs 60 (В5-4)
    assert_eq!(kinds(&d), vec![(WarningKind::ImageTextPage, Level::Info)]);
    assert_eq!((d.decision, d.quality), (PageDecision::UseTextLayer, 1.0));
    assert!(
        on(on_sig(PROSE, 0.0, Some(49)), OK).warnings.is_empty(),
        "no image, no hint"
    );
    assert!(on(on_sig(PROSE, 0.24, Some(19)), OK).warnings.is_empty());
    assert!(
        on(on_sig(PROSE, 0.24, None), OK).warnings.is_empty(),
        "not rendered"
    );
}
#[test]
fn on_ink_outside_layer_is_warn_and_merge() {
    let d = on(on_sig(PROSE, 0.5, Some(400)), OK);
    assert_eq!(kinds(&d), vec![(WarningKind::ImageTextPage, Level::Warn)]);
    assert_eq!((d.decision, d.quality), (PageDecision::RunOcrAndMerge, 1.0));
}
#[test]
fn on_digit_layer_on_scan_is_bad() {
    let d = on(
        on_sig("12 34 ,, .. 56 -- 78 ;; 90 11 22 33 ,, ..", 1.0, Some(0)),
        OK,
    );
    assert_eq!(
        (kinds(&d), d.decision),
        (
            vec![(WarningKind::SuspectedGarbage, Level::Loss)],
            PageDecision::RunOcr
        )
    );
}
#[test]
fn on_sp2_mojibake_net_still_fires() {
    let d = on(on_sig(PROSE, 0.0, None), BAD);
    assert_eq!(
        kinds(&d),
        vec![(WarningKind::SuspectedGarbage, Level::Loss)]
    );
    assert!(d.warnings[0].message.contains("not rendered"));
}
#[test]
fn on_failures_and_dropped_artifacts() {
    let f = diagnose_page(
        &ClassifyOutcome::Failed("boom".into()),
        Some(&on_sig(PROSE, 0.0, None)),
        OK,
        2,
        6,
        &on_cfg(),
    );
    assert_eq!(
        kinds(&f),
        vec![
            (WarningKind::PartialContent, Level::Warn),
            (WarningKind::ContentDropped, Level::Info)
        ]
    );
    let e = extraction_failed("bad stream");
    assert_eq!((e.decision, e.quality), (PageDecision::Reject, 0.0));
}
#[test]
fn ocr_unavailable_matrix() {
    use PageDecision::*;
    let mut o = ParseOptions::default();
    let w = ocr_unavailable(
        &[UseTextLayer, RunOcr, RunOcrAndMerge],
        &o,
        "no engine",
        false,
    )
    .unwrap();
    assert_eq!(
        (w.kind, w.level),
        (WarningKind::OcrUnavailable, Level::Info)
    );
    assert!(w.message.contains("2 page(s)") && w.message.contains("1 full, 1 image regions"));
    assert!(
        w.message.ends_with("unavailable: no engine"),
        "{}",
        w.message
    );
    assert!(ocr_unavailable(&[UseTextLayer], &o, "x", false).is_none());
    // An engine without models is a broken setup: `Warn` already under `Auto`.
    let w = ocr_unavailable(&[RunOcr], &o, "OCR models are missing", true).unwrap();
    assert_eq!(w.level, Level::Warn);
    assert!(ocr_unavailable(&[UseTextLayer], &o, "x", true).is_none());
    o.ocr = OcrMode::Always;
    assert_eq!(
        ocr_unavailable(&[UseTextLayer], &o, "models missing", false)
            .unwrap()
            .level,
        Level::Warn
    );
    o.ocr = OcrMode::Never;
    assert!(ocr_unavailable(&[RunOcr], &o, "x", false).is_none());
}
#[test]
fn off_path_ocr_needs_are_full_pages_only() {
    use PageDecision::*;
    let all = [UseTextLayer, RunOcr, RunOcrAndMerge, Reject];
    let off = ocr_needs(&all, &sp2_cfg());
    assert_eq!(off, vec![UseTextLayer, RunOcr, UseTextLayer, RunOcr]);
    assert_eq!(ocr_needs(&all, &on_cfg()), all.to_vec());
    let mut o = ParseOptions::default();
    assert!(ocr_unavailable(&ocr_needs(&[RunOcrAndMerge], &sp2_cfg()), &o, "x", false).is_none());
    let w = ocr_unavailable(&off, &o, "x", false).unwrap();
    assert!(
        w.message
            .starts_with("2 page(s) need OCR (2 full, 0 image regions)"),
        "{}",
        w.message
    );
    o.ocr = OcrMode::Always;
    let w = ocr_unavailable(&off, &o, "x", false).unwrap();
    assert_eq!(
        (w.level, w.message.starts_with("4 page(s)")),
        (Level::Warn, true)
    );
}
