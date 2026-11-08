//! Tests of the hidden-text rules.

use super::facts::{Paint, facts_rule, low_contrast, occluded, occluders, ocr_layer};
use super::*;
use pdf_oxide::RegionRole;
use pdf_oxide::extractors::{ClipKind, PaintKind};
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::{Color, TextSpan};

fn span(text: &str, y: f32, h: f32, grey: f32) -> TextSpan {
    TextSpan {
        text: text.into(),
        bbox: Rect::new(72.0, y, 100.0, h),
        font_size: h,
        color: Color {
            r: grey,
            g: grey,
            b: grey,
        },
        ..Default::default()
    }
}

fn page(spans: Vec<TextSpan>) -> StructuredPage {
    let mut r = StructuredRegion {
        kind: RegionRole::BodyBlock,
        text: String::new(),
        bbox: spans[0].bbox,
        spans,
        column_index: None,
        section_id: None,
    };
    rebuild(&mut r);
    StructuredPage {
        page_index: 0,
        page_width: 612.0,
        page_height: 792.0,
        regions: vec![r],
    }
}

fn facts() -> PageFacts {
    PageFacts {
        media: Rect::new(0.0, 0.0, 612.0, 792.0),
        scan: false,
        offpage: (0, 0),
        big_images: Vec::new(),
        paints: Vec::new(),
        occluders: Vec::new(),
        removed: Vec::new(),
    }
}

fn text(p: &StructuredPage) -> String {
    p.regions
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("|")
}

#[test]
fn strip_removes_reliable_rules_and_keeps_visible_text() {
    let mut p = page(vec![
        span("VIS01", 700.0, 12.0, 0.0),
        TextSpan {
            bbox: Rect::new(72.0, 650.0, 2.0, 0.5),
            ..span("PWNED tiny", 650.0, 0.5, 0.0)
        },
        // dvips Type3: 0.24 pt box but normal advance (kept)
        TextSpan {
            bbox: Rect::new(72.0, 640.0, 60.0, 0.24),
            ..span("dvips", 640.0, 0.24, 0.0)
        },
        span("PWNED off", 900.0, 12.0, 0.0),
        span("P\u{200B}W\u{200B}N\u{200B}E\u{200B}D", 600.0, 12.0, 0.0),
        span("A\u{E0041}\u{E0042}B", 560.0, 12.0, 0.0),
        span("\u{202E}xyz", 550.0, 12.0, 0.0),
    ]);
    let c = filter_page(&mut p, HiddenText::Strip, &facts());
    assert_eq!(text(&p), "VIS01 dvips AB");
    assert_eq!(c.spans, [1, 1, 0, 1, 1, 1, 0, 0, 0, 0, 0]);
    // one warning per (page, rule), the rule in `code`
    let w = c.warnings();
    let codes: Vec<_> = w.iter().map(|w| (w.code.unwrap(), w.level)).collect();
    assert_eq!(
        codes,
        ["offpage", "tiny", "uni_tags", "uni_bidi", "uni_zw"].map(|c| (c, Level::Info))
    );
    assert_eq!(w[1].message, "hidden text removed: tiny=9 chars (1 spans)");
}

#[test]
fn flag_keeps_text_and_warns() {
    let tiny = TextSpan {
        bbox: Rect::new(72.0, 650.0, 2.0, 0.5),
        ..span("PWNED", 650.0, 0.5, 0.0)
    };
    let mut p = page(vec![span("VIS", 700.0, 12.0, 0.0), tiny]);
    let c = filter_page(&mut p, HiddenText::Flag, &facts());
    assert_eq!(text(&p), "VIS PWNED");
    let w = c.warnings();
    assert_eq!(
        (w.len(), w[0].level, w[0].code),
        (1, Level::Warn, Some("tiny"))
    );
}

#[test]
fn tiny_on_a_scan_is_kept_with_info() {
    let tiny = TextSpan {
        bbox: Rect::new(72.0, 650.0, 0.5, 0.5),
        ..span("1", 650.0, 0.5, 0.0)
    };
    let mut p = page(vec![span("VIS", 700.0, 12.0, 0.0), tiny]);
    let mut f = facts();
    f.scan = true;
    let c = filter_page(&mut p, HiddenText::Strip, &f);
    assert_eq!(text(&p), "VIS 1");
    let w = c.warnings();
    assert_eq!(
        (w.len(), w[0].level, w[0].code),
        (1, Level::Info, Some("tiny"))
    );
    assert!(
        w[0].message
            .starts_with("hidden text kept on a scan: tiny="),
        "{}",
        w[0].message
    );
    // `Flag` still warns
    let c = filter_page(&mut p, HiddenText::Flag, &f);
    assert_eq!(c.warnings()[0].level, Level::Warn);
}

#[test]
fn off_does_nothing() {
    let mut p = page(vec![TextSpan {
        bbox: Rect::new(72.0, 650.0, 2.0, 0.5),
        ..span("PWNED", 650.0, 0.5, 0.0)
    }]);
    let c = filter_page(&mut p, HiddenText::Off, &facts());
    assert_eq!(
        (text(&p).as_str(), c.clone()),
        ("PWNED", HiddenCounts::default())
    );
    assert!(c.warnings().is_empty());
}

#[test]
fn legitimate_format_characters_are_kept() {
    // SHY, ZWNJ, RLM, ZWJ emoji, RLE..PDF
    let t = "co\u{00AD}op می\u{200C}خواهم \u{200F}x 👩\u{200D}💻 \u{202B}abc\u{202C}";
    let mut p = page(vec![span(t, 700.0, 12.0, 0.0)]);
    assert_eq!(
        filter_page(&mut p, HiddenText::Strip, &facts()),
        HiddenCounts::default()
    );
    assert_eq!(text(&p), t);
}

#[test]
fn rtl_isolates_are_kept() {
    // Hebrew and Arabic with FSI…PDI / LRI…PDI around a Latin word (RTL control)
    let t = "שלום \u{2068}KEEP-RTL\u{2069} עולם مرحبا \u{2066}PDF\u{2069} \u{2067}x\u{2069}";
    let mut p = page(vec![span(t, 700.0, 12.0, 0.0)]);
    assert_eq!(
        filter_page(&mut p, HiddenText::Strip, &facts()),
        HiddenCounts::default()
    );
    assert_eq!(text(&p), t);
}

#[test]
fn table_words_get_the_span_rules() {
    let word = |t: &str, h: f64| crate::tables::InputWord {
        rect: crate::tables::Rect::new(72.0, 100.0, 6.0 * t.chars().count() as f64 * h / 12.0, h),
        text: t.to_string(),
        font_size: h,
        char_x: Vec::new(),
        rotated: false,
    };
    let words = || {
        vec![
            word("Total", 12.0),
            word("PWNED", 0.5),
            word("\u{202E}DENWP", 12.0),
            word("4.10\u{E0041}", 12.0),
        ]
    };
    let texts =
        |v: &[crate::tables::InputWord]| v.iter().map(|w| w.text.clone()).collect::<Vec<_>>();
    let mut v = words();
    let c = filter_words(&mut v, HiddenText::Strip, &facts(), (0.0, 792.0));
    assert_eq!(texts(&v), ["Total", "4.10"]);
    assert_eq!(
        (c.chars[TINY], c.chars[UNI_BIDI], c.chars[UNI_TAGS]),
        (5, 6, 1)
    );
    assert!(c.removed[TINY] && c.removed[UNI_BIDI] && c.removed[UNI_TAGS]);
    let mut v = words();
    let c = filter_words(&mut v, HiddenText::Flag, &facts(), (0.0, 792.0));
    assert_eq!((v.len(), c.removed[TINY]), (4, false));
    let mut v = words();
    assert_eq!(
        filter_words(&mut v, HiddenText::Off, &facts(), (0.0, 792.0)),
        HiddenCounts::default()
    );
    // a scan keeps tiny words
    let mut f = facts();
    f.scan = true;
    let mut v = vec![
        word("Total", 12.0),
        word("PWNED", 0.5),
        word("SECRET", 12.0),
    ];
    let c = filter_words(&mut v, HiddenText::Strip, &f, (0.0, 792.0));
    assert_eq!(v.len(), 3);
    assert_eq!((c.chars[TINY], c.removed[TINY]), (5, false));
    // two extractions of the same text: the larger count per rule, not the sum
    let mut a = c.clone();
    a.merge_max(&c);
    assert_eq!(a, c);
    // tag chars are cut from `char_x` in step with the text
    let mut t = word("4.10\u{E0041}", 12.0);
    t.char_x = (0..5).map(|i| (f64::from(i), f64::from(i) + 1.0)).collect();
    let mut v = vec![t];
    filter_words(&mut v, HiddenText::Strip, &facts(), (0.0, 792.0));
    assert_eq!((v[0].text.as_str(), v[0].char_x.len()), ("4.10", 4));
    // glyphs of a removed span (PDF coordinates) are cut from a word, uncounted
    let mut f = facts();
    f.removed = vec![(Rect::new(80.0, 680.0, 20.0, 0.5), "PWNED".to_string())];
    let mut g = word("4.10PWNED", 12.0);
    g.char_x = (0..9)
        .map(|i| (72.0 + 2.0 * f64::from(i), 74.0 + 2.0 * f64::from(i)))
        .collect();
    g.rect = crate::tables::Rect::new(72.0, 100.0, 18.0, 12.0);
    let mut v = vec![g];
    let c = filter_words(&mut v, HiddenText::Strip, &f, (0.0, 792.0));
    assert_eq!(
        (v[0].text.as_str(), v[0].rect.w, c),
        ("4.10", 8.0, HiddenCounts::default())
    );
    // a large removed box takes only its own text
    let mut f = facts();
    f.removed = vec![(Rect::new(60.0, 670.0, 100.0, 40.0), "\u{2020}".to_string())];
    let mut v = vec![word("9A", 12.0)];
    filter_words(&mut v, HiddenText::Strip, &f, (0.0, 792.0));
    assert_eq!(v[0].text, "9A");
    let (mut a, mut b) = (HiddenCounts::default(), HiddenCounts::default());
    a.add(TINY, 5, false);
    b.add(TINY, 7, true);
    b.add(TINY, 1, true);
    a.merge_max(&b);
    assert_eq!(
        (a.chars[TINY], a.spans[TINY], a.removed[TINY]),
        (8, 2, true)
    );
}

/// A word at `(x, top)` with chars `cw` wide, one `char_x` per char.
fn glyphs(t: &str, x: f64, top: f64, h: f64, cw: f64) -> crate::tables::InputWord {
    let n = t.chars().count();
    crate::tables::InputWord {
        rect: crate::tables::Rect::new(x, top, cw * n as f64, h),
        text: t.to_string(),
        font_size: h,
        char_x: (0..n)
            .map(|i| (x + cw * i as f64, x + cw * (i + 1) as f64))
            .collect(),
        rotated: false,
    }
}

/// Texts of `words` after cutting the removed spans `removed` (PDF coordinates, page top 792).
fn cut(mut words: Vec<crate::tables::InputWord>, removed: &[(Rect, &str)]) -> Vec<String> {
    let removed: Vec<(Rect, String)> = removed.iter().map(|(r, t)| (*r, t.to_string())).collect();
    cut_removed(&mut words, &removed, (0.0, 792.0));
    words.into_iter().map(|w| w.text).collect()
}

#[test]
fn removed_spans_are_cut_by_normalised_text_and_glyphs_in_the_box() {
    // payload chars at x 80..90 (2 pt each after "4.10"), glued: one word
    let payload = Rect::new(80.0, 680.0, 10.0, 0.5);
    let w = |t: &str| glyphs(t, 72.0, 100.0, 12.0, 2.0);
    // zero-width chars on either side are not compared
    assert_eq!(
        cut(vec![w("4.10PWNED")], &[(payload, "PW\u{200B}NED")]),
        ["4.10"]
    );
    let zw = Rect::new(80.0, 680.0, 12.0, 0.5);
    assert_eq!(
        cut(vec![w("4.10PW\u{200B}NED")], &[(zw, "PWNED")]),
        ["4.10"]
    );
    // an ignorable char of the span in front of the run goes with it
    let rlo = "\u{202E}DENWP";
    assert_eq!(cut(vec![w(&format!("4.10{rlo}"))], &[(zw, rlo)]), ["4.10"]);
    // glyphs out of order (a TJ jump back): the span's chars in the box
    let back = Rect::new(80.0, 680.0, 18.0, 0.5);
    assert_eq!(
        cut(vec![w("4.10EDVILPWNE")], &[(back, "PWNEDEVIL")]),
        ["4.10"]
    );
    // each char of the span is cut once
    let ee = Rect::new(72.0, 680.0, 4.0, 0.5);
    assert_eq!(cut(vec![w("EE")], &[(ee, "XE")]), ["E"]);
    // a large box takes only the span's text, also by position
    let big = Rect::new(60.0, 670.0, 100.0, 40.0);
    assert_eq!(cut(vec![w("9A\u{2020}")], &[(big, "\u{2020}")]), ["9A"]);
    assert_eq!(cut(vec![w("9A")], &[(big, "\u{2020}")]), ["9A"]);
    // a word of the span's text in the box goes whole
    assert!(cut(vec![w("PWNED")], &[(big, "PW\u{200B}NED")]).is_empty());
}

#[test]
fn cutting_removed_spans_looks_only_at_their_line() {
    // 60 lines of 20 visible words and 200 removed 0.5 pt spans, one tall removed box
    let (lines, vis, tiny) = (60usize, 20usize, 200usize);
    let (mut words, mut removed) = (Vec::new(), Vec::new());
    for l in 0..lines {
        let top = 100.0 + 8.0 * l as f64;
        for k in 0..vis {
            words.push(glyphs("alpha", 72.0 + 22.5 * k as f64, top, 6.0, 3.0));
        }
        for k in 0..tiny {
            let x = 72.0 + 2.25 * k as f64;
            words.push(glyphs(&format!("Q{k}"), x, top + 3.0, 0.5, 0.2));
            let y = 792.0 - (top + 3.5);
            removed.push((Rect::new(x as f32, y as f32, 0.6, 0.5), format!("Q{k}")));
        }
    }
    removed.push((Rect::new(72.0, 100.0, 450.0, 600.0), "Z".to_string()));
    let n = words.len();
    let work = cut_removed(&mut words, &removed, (0.0, 792.0));
    assert_eq!(words.len(), lines * vis);
    assert!(words.iter().all(|w| w.text == "alpha"));
    // each word looks at the boxes of its own line and the tall box, not at the page
    assert!(work <= n * (tiny + 1), "{work}");
}

#[test]
fn emptied_regions_are_dropped() {
    // a region whose only span is removed disappears
    let mut p = page(vec![span("VIS", 700.0, 12.0, 0.0)]);
    p.regions.push(
        page(vec![span("PWNED", 900.0, 12.0, 0.0)])
            .regions
            .remove(0),
    );
    filter_page(&mut p, HiddenText::Strip, &facts());
    assert_eq!((text(&p).as_str(), p.regions.len()), ("VIS", 1));
}

#[test]
fn zero_width_break_hints_are_not_an_interleave() {
    assert_eq!(
        zw_interleave_run("P\u{200B}W\u{200B}N\u{200B}E\u{200B}D"),
        5
    );
    assert_eq!(
        zw_interleave_run("http://creat\u{200B}iveco\u{200B}mmons\u{200B}.org"),
        0
    );
    assert_eq!(
        zw_interleave_run("GTT\u{200B}TAA\u{200B}ACT\u{200B} CGG"),
        0
    );
    assert_eq!(zw_interleave_run("a\u{200B}b"), 2);
}

#[test]
fn offpage_spans_dropped_by_the_fork_count_as_removed_in_every_mode() {
    let mut f = facts();
    f.offpage = (2, 9);
    for mode in [HiddenText::Strip, HiddenText::Flag] {
        let mut p = page(vec![span("VIS", 700.0, 12.0, 0.0)]);
        let c = filter_page(&mut p, mode, &f);
        assert_eq!(text(&p), "VIS");
        let w = c.warnings();
        assert_eq!(
            (w.len(), w[0].level, w[0].code),
            (1, Level::Info, Some("offpage"))
        );
        assert_eq!(
            w[0].message,
            "hidden text removed: offpage=9 chars (2 spans)"
        );
    }
    let mut p = page(vec![span("VIS", 700.0, 12.0, 0.0)]);
    assert_eq!(
        filter_page(&mut p, HiddenText::Off, &f),
        HiddenCounts::default()
    );
}

#[test]
fn invisible_render_modes_outside_an_ocr_layer() {
    for tr in [3, 7] {
        assert_eq!(facts_rule(tr, 1.0, 1.0, 1.0, || false), Some(INVISIBLE));
        assert_eq!(facts_rule(tr, 1.0, 1.0, 1.0, || true), None);
    }
    for tr in [0, 1, 2, 4, 5, 6] {
        assert_eq!(facts_rule(tr, 1.0, 1.0, 1.0, || unreachable!()), None);
    }
    let s = span("word", 700.0, 12.0, 0.0); // x 72..172
    let mut f = facts();
    assert!(!ocr_layer(&s, &f));
    f.big_images = vec![Rect::new(150.0, 0.0, 400.0, 792.0)]; // overlaps the span's right end
    assert!(ocr_layer(&s, &f));
    f.big_images = vec![Rect::new(200.0, 0.0, 400.0, 792.0)];
    assert!(!ocr_layer(&s, &f));
    let glyphless = TextSpan {
        font_name: "GlyphLessFont".into(),
        ..s.clone()
    };
    assert!(ocr_layer(&glyphless, &f));
    f.scan = true;
    assert!(ocr_layer(&s, &f));
}

#[test]
fn alpha_follows_the_painted_parts_of_the_render_mode() {
    let no_ocr = || unreachable!();
    assert_eq!(facts_rule(0, 0.02, 1.0, 1.0, no_ocr), Some(ALPHA));
    assert_eq!(facts_rule(0, 0.5, 1.0, 1.0, no_ocr), None); // c12: ca 0.5
    assert_eq!(facts_rule(0, 1.0, 0.0, 1.0, no_ocr), None); // stroke alpha does not matter
    assert_eq!(facts_rule(1, 0.0, 1.0, 1.0, no_ocr), None); // stroke only: fill alpha does not matter
    assert_eq!(facts_rule(5, 1.0, 0.0, 1.0, no_ocr), Some(ALPHA));
    assert_eq!(facts_rule(2, 0.0, 1.0, 1.0, no_ocr), None); // the stroke is still visible
    assert_eq!(facts_rule(6, 0.0, 0.05, 1.0, no_ocr), Some(ALPHA));
    assert_eq!(facts_rule(4, 0.05, 1.0, 1.0, no_ocr), Some(ALPHA));
}

#[test]
fn clip_applies_to_painting_modes_only() {
    let no_ocr = || unreachable!();
    for tr in [0, 1, 2] {
        assert_eq!(facts_rule(tr, 1.0, 1.0, 0.0, no_ocr), Some(CLIP));
        assert_eq!(facts_rule(tr, 1.0, 1.0, 0.01, no_ocr), Some(CLIP));
        assert_eq!(facts_rule(tr, 1.0, 1.0, 0.2, no_ocr), None); // text overflowing a cell
    }
    // Tr 4–6 add to the clip; the fork does not narrow it, so their coverage is not used
    for tr in [4, 5, 6] {
        assert_eq!(facts_rule(tr, 1.0, 1.0, 0.0, no_ocr), None);
    }
    // transparency is reported before the clip
    assert_eq!(facts_rule(0, 0.0, 1.0, 0.0, no_ocr), Some(ALPHA));
}

fn paint(kind: PaintKind, bbox: Rect, order: u32) -> Paint {
    Paint {
        kind,
        bbox,
        color: None,
        alpha: 1.0,
        masked: false,
        clip: ClipKind::None,
        order,
    }
}

/// `occluded` for a span at paint order 5, with the occluders filtered from `f.paints`.
fn occ(f: &mut PageFacts, b: &Rect) -> bool {
    f.occluders = occluders(&f.paints, &f.media);
    occluded(5, b, f)
}

#[test]
fn later_opaque_rects_and_images_hide_text() {
    let b = Rect::new(72.0, 300.0, 100.0, 12.0);
    let over = Rect::new(60.0, 290.0, 500.0, 30.0);
    let mut f = facts();
    assert!(!occ(&mut f, &b));
    for kind in [PaintKind::Rect, PaintKind::Image] {
        f.paints = vec![paint(kind, over, 6)];
        assert!(occ(&mut f, &b), "{kind:?}");
        // drawn before the text: a background
        f.paints = vec![paint(kind, over, 4)];
        assert!(!occ(&mut f, &b), "{kind:?}");
    }
    // other paint kinds, masked or translucent paints and small overlaps do not hide
    for kind in [PaintKind::Path, PaintKind::Shading, PaintKind::Form] {
        f.paints = vec![paint(kind, over, 6)];
        assert!(!occ(&mut f, &b), "{kind:?}");
    }
    f.paints = vec![Paint {
        masked: true,
        ..paint(PaintKind::Image, over, 6)
    }];
    assert!(!occ(&mut f, &b));
    f.paints = vec![Paint {
        alpha: 0.5,
        ..paint(PaintKind::Rect, over, 6)
    }];
    assert!(!occ(&mut f, &b));
    f.paints = vec![paint(
        PaintKind::Rect,
        Rect::new(72.0, 300.0, 85.0, 12.0),
        6,
    )];
    assert!(!occ(&mut f, &b)); // 85 % of the box
    f.paints = vec![paint(
        PaintKind::Rect,
        Rect::new(72.0, 300.0, 95.0, 12.0),
        6,
    )];
    assert!(occ(&mut f, &b));
    // a full-page image over the text: a scan over its OCR layer (ABBYY)
    f.paints = vec![paint(
        PaintKind::Image,
        Rect::new(0.0, 0.0, 612.0, 792.0),
        6,
    )];
    assert!(!occ(&mut f, &b));
    f.paints = vec![paint(PaintKind::Rect, Rect::new(0.0, 0.0, 612.0, 792.0), 6)];
    assert!(occ(&mut f, &b));
    // a fill under a ring or polygon clip paints less than its box; a rectangle clip is exact
    for (clip, hides) in [
        (ClipKind::Rect, true),
        (ClipKind::Path, false),
        (ClipKind::EvenOdd, false),
    ] {
        f.paints = vec![Paint {
            clip,
            ..paint(PaintKind::Rect, over, 6)
        }];
        assert_eq!(occ(&mut f, &b), hides, "{clip:?}");
    }
}

#[path = "hidden_contrast_test.rs"]
mod contrast;
