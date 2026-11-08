use super::*;
use pdf_oxide::{RegionRole, StructuredRegion};
use plaindoc_core::Level;

fn span(text: &str, x: f32, y: f32) -> TextSpan {
    TextSpan {
        text: text.into(),
        bbox: Rect::new(x, y, 100.0, 12.0),
        font_size: 12.0,
        ..Default::default()
    }
}

fn page(spans: Vec<TextSpan>) -> StructuredPage {
    StructuredPage {
        page_index: 0,
        page_width: 612.0,
        page_height: 792.0,
        regions: vec![StructuredRegion {
            kind: RegionRole::BodyBlock,
            text: spans.iter().map(|s| s.text.as_str()).collect(),
            bbox: spans[0].bbox,
            spans,
            column_index: None,
            section_id: None,
        }],
    }
}

fn levels(o: &OcgOff) -> Vec<(Option<&'static str>, Level)> {
    o.counts
        .warnings()
        .iter()
        .map(|w| (w.code, w.level))
        .collect()
}

#[test]
fn ocg_off_counts_the_page_difference() {
    let full = page(vec![
        span("VIS11", 72.0, 700.0),
        span("PWNED 11", 72.0, 650.0),
    ]);
    let shown = page(vec![span("VIS11", 72.0, 700.0)]);
    let o = ocg_off(&full, &shown, true);
    assert_eq!((o.counts.chars[OCG_OFF], o.counts.spans[OCG_OFF]), (7, 1));
    assert_eq!(levels(&o), [(Some("ocg_off"), Level::Info)]);
    let w = o.counts.warnings();
    assert!(!w[0].message.contains("PWNED"), "{}", w[0].message);
    // the default-off span's box and text go to the table words
    assert_eq!(
        o.removed,
        [(full.regions[0].spans[1].bbox, "PWNED 11".to_string())]
    );
    // `Flag`: counted the same way, kept
    let o = ocg_off(&full, &shown, false);
    assert_eq!(o.counts.chars[OCG_OFF], 7);
    assert_eq!(levels(&o), [(Some("ocg_off"), Level::Warn)]);
    assert!(o.removed.is_empty());
    // a visible run merged with a default-off one: its tokens are not cut from the words
    let merged = page(vec![span("Total PWNED", 72.0, 700.0)]);
    let total = page(vec![span("Total", 72.0, 700.0)]);
    assert_eq!(ocg_off(&merged, &total, true).removed[0].1, "PWNED");
}

#[test]
fn a_same_length_substitution_is_still_reported() {
    // the whole page kept the hidden upper-case copy of a visible run (dedup): no char difference
    let full = page(vec![span("VISIBLE TEXT", 72.0, 700.0)]);
    let shown = page(vec![span("visible text", 72.0, 700.0)]);
    assert!(!same_spans(&full, &shown));
    let o = ocg_off(&full, &shown, true);
    assert_eq!((o.counts.chars[OCG_OFF], o.counts.spans[OCG_OFF]), (11, 1));
    assert_eq!(levels(&o), [(Some("ocg_off"), Level::Info)]);
}

#[test]
fn words_are_not_cut_when_the_extractions_disagree_beyond_the_hidden_text() {
    // a payload glued to a visible token: the unmatched token holds more than the hidden chars
    let full = page(vec![span("4.10PWNED", 72.0, 700.0)]);
    let shown = page(vec![span("4.10", 72.0, 700.0)]);
    let o = ocg_off(&full, &shown, true);
    assert_eq!(o.counts.chars[OCG_OFF], 5);
    assert!(o.removed.is_empty());
    // not claimed removed: the payload may stay in a table cell
    assert_eq!(levels(&o), [(Some("ocg_off"), Level::Warn)]);
}

#[test]
fn many_default_off_spans_look_only_at_their_own_lines() {
    // 4000 visible and 4000 default-off spans on 400 lines; the merged run on each line keeps
    // its visible token
    let (lines, per) = (400, 10);
    let (mut full, mut shown) = (Vec::new(), Vec::new());
    for l in 0..lines {
        let y = 10.0 + 14.0 * l as f32;
        for k in 0..per {
            let x = 20.0 * k as f32;
            shown.push(span("alpha", x, y));
            full.push(span("alpha", x, y));
            full.push(span(&format!("PWNED{k}"), x + 5.0, y));
        }
        full.push(span("alpha beta", 200.0, y));
    }
    shown.push(span("tall", 500.0, 0.0));
    shown.last_mut().unwrap().bbox.height = 790.0;
    let (full, shown) = (page(full), page(shown));
    let o = ocg_off(&full, &shown, true);
    assert_eq!(o.counts.spans[OCG_OFF], lines * (per + 1));
    assert_eq!(o.removed.len(), lines * (per + 1));
    assert!(
        o.removed
            .iter()
            .all(|(_, t)| t.starts_with("PWNED") || t == "beta")
    );
    assert_eq!(o.removed.iter().filter(|(_, t)| t == "beta").count(), lines);
    // each default-off span looks at the spans of its own line and the tall one, not the page
    let (off, _) = unmatched(&full, &shown);
    let (_, work) = guarded(&off, &shown);
    assert!(work <= o.removed.len() * (per + 1), "{work}");
}

fn err() -> pdf_oxide::Result<StructuredPage> {
    Err(pdf_oxide::Error::InvalidXref)
}

/// `choose` with marked pages: `gate` and `filtered` tag what they read.
fn pick(
    full: pdf_oxide::Result<StructuredPage>,
    shown: pdf_oxide::Result<StructuredPage>,
    strip: bool,
) -> (Option<String>, OcgOff) {
    let tag = |mut p: StructuredPage, t: &str| {
        p.regions[0].text = format!("{t}:{}", p.regions[0].text);
        Ok(p)
    };
    let (p, o) = choose(
        full,
        shown,
        strip,
        |p| tag(p, "gate"),
        |p| tag(p.clone(), "filtered"),
    );
    (p.ok().map(|p| p.regions[0].text.clone()), o)
}

#[test]
fn extraction_errors_fail_open_with_a_warning() {
    let full = || {
        Ok(page(vec![
            span("KEEP", 72.0, 700.0),
            span("PWNED", 72.0, 600.0),
        ]))
    };
    let shown = || Ok(page(vec![span("KEEP", 72.0, 700.0)]));
    let unchecked = [(Some("ocg_off"), Level::Warn)];
    for strip in [true, false] {
        // the filtered page failed: the whole page (visible text kept), not checked
        let (p, o) = pick(full(), err(), strip);
        assert_eq!(p.as_deref(), Some("gate:KEEPPWNED"));
        assert_eq!(levels(&o), unchecked);
        assert!(!o.counts.warnings()[0].message.contains("PWNED"));
        // both failed: the page fails as without layers
        let (p, o) = pick(err(), err(), strip);
        assert_eq!((p, o.counts), (None, HiddenCounts::default()));
    }
    // the whole page failed: `Strip` uses the page without the layers, words cannot be cut
    let (p, o) = pick(err(), shown(), true);
    assert_eq!(p.as_deref(), Some("filtered:KEEP"));
    assert_eq!(levels(&o), unchecked);
    // `Flag` reads the whole page: it fails
    assert_eq!(pick(err(), shown(), false).0, None);
    // both read: `Strip` the filtered page, `Flag` the whole one; equal pages: the whole one
    assert_eq!(
        pick(full(), shown(), true).0.as_deref(),
        Some("filtered:KEEP")
    );
    assert_eq!(
        pick(full(), shown(), false).0.as_deref(),
        Some("gate:KEEPPWNED")
    );
    let (p, o) = pick(shown(), shown(), true);
    assert_eq!(
        (p.as_deref(), o.counts),
        (Some("gate:KEEP"), HiddenCounts::default())
    );
}
