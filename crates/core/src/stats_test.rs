use super::*;
use crate::{Cell, ListItem, Section, SectionKind, Warning};

fn doc(text: &str) -> Document {
    let mut s0 = Section::new(SectionKind::Page, 0);
    s0.blocks = vec![
        Block::heading(1, format!("Two {text}"), None),
        Block::List {
            ordered: false,
            items: vec![ListItem::new("one"), ListItem::new("two three")],
            bbox: None,
        },
        Block::Table {
            rows: vec![
                vec![Cell::new("a b"), Cell::new("")],
                vec![Cell::new("c").with_span(2, 1)],
            ],
            header_rows: 1,
            bbox: None,
        },
    ];
    s0.warnings
        .push(Warning::new(WarningKind::TableUncertain, Level::Warn, text));
    let mut s1 = Section::new(SectionKind::Page, 1);
    s1.source = TextSource::Ocr;
    s1.blocks = vec![Block::Image {
        alt: Some(text.into()),
        ocr_text: Some("scan text".into()),
        bbox: None,
        image: None,
    }];
    s1.warnings
        .push(Warning::new(WarningKind::OcrUsed, Level::Info, text));
    let mut d = Document::new(vec![s0, s1]);
    d.warnings
        .push(Warning::new(WarningKind::TableUncertain, Level::Info, text));
    d.metadata.page_count = Some(5);
    d.metadata.title = Some(text.into());
    d.metadata.author = Some(text.into());
    d
}

#[test]
fn document_counters() {
    let mut s = ParseStats::default();
    s.fill_from_document(&doc("words"));
    // blocks: heading, list + 2 item paragraphs, table + 2 cell paragraphs ("" has none), image
    assert_eq!(
        (s.sizes.blocks, s.sizes.words, s.sizes.tables, s.sizes.cells),
        (8, 10, 1, 3)
    );
    // "Two words" 9 + "one" 3 + "two three" 9 + "a b" 3 + "c" 1 + "scan text" 9
    assert_eq!(s.sizes.text_bytes, 34);
    assert_eq!(
        (s.pages.total, s.pages.processed, s.pages.ocr, s.pages.mixed),
        (5, 2, 1, 0)
    );
    let tu = s.warnings[&WarningKind::TableUncertain];
    assert_eq!((tu.info, tu.warn, tu.loss), (1, 1, 0));
    assert_eq!(s.warnings[&WarningKind::OcrUsed].info, 1);
}

#[test]
fn fill_is_idempotent_and_keeps_in_flight_counters() {
    let mut s = ParseStats {
        sizes: Sizes {
            spans: 7,
            ..Default::default()
        },
        pages: PageCounts {
            skipped: 3,
            ..Default::default()
        },
        ..Default::default()
    };
    s.fill_from_document(&doc("x"));
    let once = s.clone();
    s.fill_from_document(&doc("x"));
    assert_eq!(s, once);
    assert_eq!((s.sizes.spans, s.pages.skipped), (7, 3));
}

#[test]
fn json_shape_uses_codes() {
    let mut s = ParseStats {
        format: Some(Format::Pdf),
        ..Default::default()
    };
    s.add_stage(Stage::Text, Duration::from_micros(1500));
    s.add_stage(Stage::Text, Duration::from_micros(500));
    s.hit(LimitKind::MaxPages, None);
    s.fill_from_document(&doc("x"));
    let j = s.to_json();
    assert!(!j.contains('\n'));
    for part in [
        r#""format":"pdf""#,
        r#""outcome":"ok""#,
        r#""stages_us":{"text":2000}"#,
        r#""limits":{"max_pages":1}"#,
        r#""OcrUsed":{"info":1,"warn":0,"loss":0}"#,
        r#""TableUncertain":{"info":1,"warn":1,"loss":0}"#,
    ] {
        assert!(j.contains(part), "{part} in {j}");
    }
}

#[test]
fn warning_kind_as_str_is_the_serialized_name() {
    use WarningKind::*;
    for k in [
        NoTextLayer,
        ImageTextPage,
        SuspectedGarbage,
        PartialContent,
        Truncated,
        Encrypted,
        OcrUsed,
        LowOcrConfidence,
        OcrEmpty,
        OcrUnavailable,
        TableUncertain,
        FormulaSkipped,
        EmptyOutput,
        HiddenContent,
        ContentDropped,
    ] {
        assert_eq!(
            serde_json::to_string(&k).unwrap(),
            format!("\"{}\"", k.as_str())
        );
    }
}

#[test]
fn codes_are_snake_case_as_serialized() {
    for l in [
        LimitKind::InputSize,
        LimitKind::TableModelRegions,
        LimitKind::OcrPageTimeout,
    ] {
        assert_eq!(
            serde_json::to_string(&l).unwrap(),
            format!("\"{}\"", l.as_str())
        );
    }
    for s in [Stage::Open, Stage::Diagnose, Stage::Assemble] {
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            format!("\"{}\"", s.as_str())
        );
    }
    assert_eq!(
        serde_json::to_string(&Outcome::TooLarge).unwrap(),
        "\"too_large\""
    );
}

#[test]
fn without_timings_zeroes_durations_only() {
    let mut s = ParseStats {
        wall_us: 9,
        sizes: Sizes {
            words: 4,
            ..Default::default()
        },
        ..Default::default()
    };
    s.add_stage(Stage::Ocr, Duration::from_millis(3));
    let z = s.without_timings();
    assert_eq!(
        (z.wall_us, z.stages_us[&Stage::Ocr], z.sizes.words),
        (0, 0, 4)
    );
}

#[test]
fn finish_maps_errors_to_outcome_and_limits() {
    let cases: Vec<(ParseError, Outcome, Option<LimitKind>)> = vec![
        (
            ParseError::TooLarge { size: 2, limit: 1 },
            Outcome::TooLarge,
            Some(LimitKind::InputSize),
        ),
        (
            ParseError::Limit {
                kind: "x".into(),
                limit: 1,
            },
            Outcome::Limit,
            Some(LimitKind::ParserLimit),
        ),
        (
            ParseError::Timeout(Duration::from_secs(1)),
            Outcome::Timeout,
            Some(LimitKind::DocumentTimeout),
        ),
        (ParseError::Corrupt("x".into()), Outcome::Corrupt, None),
        (ParseError::Internal("x".into()), Outcome::Internal, None),
        (ParseError::Encrypted, Outcome::Encrypted, None),
        (
            ParseError::Unsupported(Format::Txt),
            Outcome::Unsupported,
            None,
        ),
        (std::io::Error::other("x").into(), Outcome::Io, None),
    ];
    for (e, want, limit) in cases {
        let mut s = ParseStats {
            input_bytes: 10,
            ..Default::default()
        };
        s.finish(&Err(e), Duration::from_micros(42));
        assert_eq!(s.outcome, want);
        assert_eq!(s.limits.keys().next().copied(), limit);
        assert_eq!((s.wall_us, s.est_peak_bytes), (42, 10));
    }
}

#[test]
fn peak_estimate_is_input_plus_the_larger_of_work_and_text() {
    let mut s = ParseStats {
        input_bytes: 100,
        sizes: Sizes {
            est_work_bytes: 1000,
            ..Default::default()
        },
        ..Default::default()
    };
    s.finish(&Ok(doc("x")), Duration::ZERO);
    assert_eq!(s.est_peak_bytes, 1100);
    s.sizes.est_work_bytes = 0;
    s.finish(&Ok(doc("x")), Duration::ZERO);
    assert_eq!(s.est_peak_bytes, 100 + s.sizes.text_bytes);
}

#[test]
fn stats_hold_no_document_content() {
    const MARK: &str = "Qx7731SecretMarker";
    let mut s = ParseStats {
        format: Some(Format::Pdf),
        ..Default::default()
    };
    s.finish(&Ok(doc(MARK)), Duration::from_micros(1));
    assert!(!s.to_json().contains(MARK));
    assert!(!format!("{s:?}").contains(MARK));
    for e in [
        ParseError::Corrupt(MARK.into()),
        ParseError::Internal(MARK.into()),
    ] {
        let mut s = ParseStats::default();
        s.finish(&Err(e), Duration::ZERO);
        assert!(!s.to_json().contains(MARK) && !format!("{s:?}").contains(MARK));
    }
}
