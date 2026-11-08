use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;
use pdf_oxide::{RegionRole, StructuredRegion};

use super::*;

/// A span at `x` of width `w` on the line at `y`, 10 pt.
fn span(text: &str, x: f32, y: f32, w: f32) -> TextSpan {
    TextSpan {
        text: text.into(),
        bbox: Rect::new(x, y, w, 10.0),
        font_size: 10.0,
        ..Default::default()
    }
}

/// Spans laid out left to right on one line: (text, width, gap before).
fn row(parts: &[(&str, f32, f32)]) -> Vec<TextSpan> {
    let mut x = 100.0;
    parts
        .iter()
        .map(|(t, w, gap)| {
            x += gap;
            let s = span(t, x, 700.0, *w);
            x += w;
            s
        })
        .collect()
}

/// The region the fork builds from `spans`: trimmed texts joined by a space.
fn region(spans: Vec<TextSpan>) -> StructuredRegion {
    let text = spans
        .iter()
        .map(|s| s.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    StructuredRegion {
        kind: RegionRole::BodyBlock,
        text,
        bbox: Rect::new(0.0, 0.0, 1.0, 1.0),
        spans,
        column_index: None,
        section_id: None,
    }
}

fn tightened(spans: Vec<TextSpan>) -> String {
    let mut r = region(spans);
    tighten(&mut r);
    r.text
}

#[test]
fn brackets_hug_their_word() {
    let s = row(&[
        ("menu", 20.0, 0.0),
        ("(", 3.0, 2.5),
        ("Swift", 20.0, 0.0),
        (")", 3.0, 0.01),
    ]);
    assert_eq!(tightened(s), "menu (Swift)");
}

#[test]
fn space_glyph_before_bracket_stays() {
    // the space is drawn as a glyph at the end of the word's span
    let s = row(&[
        ("слово ", 30.0, 0.0),
        ("(", 3.0, 0.0),
        ("пояснение", 40.0, 0.0),
        (")", 3.0, 0.0),
    ]);
    assert_eq!(tightened(s), "слово (пояснение)");
}

#[test]
fn hyphen_inside_a_word_has_no_spaces() {
    let s = row(&[("CAM", 20.0, 0.0), ("-", 4.0, 0.0), ("тулчейн", 30.0, 0.01)]);
    assert_eq!(tightened(s), "CAM-тулчейн");
}

#[test]
fn spaced_dash_stays() {
    let s = row(&[("слово", 30.0, 0.0), ("—", 10.0, 2.8), ("слово", 30.0, 2.8)]);
    assert_eq!(tightened(s), "слово — слово");
}

#[test]
fn address_dots_and_commas() {
    let s = row(&[
        ("name", 20.0, 0.0),
        (".", 2.0, 0.0),
        ("x", 5.0, -0.2),
        ("@", 8.0, 0.0),
        ("mail", 20.0, 0.0),
        (",", 2.0, 0.0),
        ("next", 20.0, 2.5),
    ]);
    assert_eq!(tightened(s), "name.x@mail, next");
}

#[test]
fn tab_stop_gap_keeps_space() {
    // `Figure 1.2.` then a tab (a whitespace glyph the region does not keep) and the title
    let s = row(&[("Figure 1.2.", 60.0, 0.0), ("Title", 30.0, 1.12)]);
    assert_eq!(tightened(s), "Figure 1.2. Title");
}

#[test]
fn letter_pairs_keep_the_fork_space() {
    // touching letter runs are the fork's call (fake zero gaps from coarse width tables)
    let s = row(&[("word", 20.0, 0.0), ("next", 20.0, 0.0)]);
    assert_eq!(tightened(s), "word next");
}

#[test]
fn list_marker_with_gap_keeps_space() {
    let s = row(&[("1.", 8.0, 0.0), ("Item", 20.0, 4.0)]);
    assert_eq!(tightened(s), "1. Item");
    let s = row(&[("•", 4.0, 0.0), ("Item", 20.0, 3.0)]);
    assert_eq!(tightened(s), "• Item");
}

#[test]
fn next_line_keeps_space() {
    let a = span("end-", 100.0, 700.0, 20.0);
    let b = span("(start", 120.0, 688.0, 20.0);
    assert_eq!(tightened(vec![a, b]), "end- (start");
}

#[test]
fn large_overlap_is_not_a_touch() {
    // a run drawn over most of its neighbour (duplicate, overprint) keeps the space
    let a = span("(", 100.0, 700.0, 3.0);
    let b = span("x", 99.0, 700.0, 3.0);
    assert_eq!(tightened(vec![a, b]), "( x");
}

#[test]
fn rotated_spans_keep_space() {
    let mut s = row(&[("(", 3.0, 0.0), ("a", 5.0, 0.0)]);
    s[1].rotation_degrees = 90.0;
    assert_eq!(tightened(s), "( a");
}

#[test]
fn text_not_from_spans_is_kept() {
    let s = row(&[("(", 3.0, 0.0), ("a", 5.0, 0.0)]);
    let mut r = region(s);
    r.text = "other ( a".into();
    tighten(&mut r);
    assert_eq!(r.text, "other ( a");
}

#[test]
fn kerned_join_of_the_fork_is_kept() {
    // the fork already joined these two letter spans without a space
    let s = row(&[("окру", 20.0, 0.0), ("глены", 20.0, -0.5), (")", 3.0, 0.0)]);
    let mut r = region(s);
    r.text = "округлены )".into();
    tighten(&mut r);
    assert_eq!(r.text, "округлены)");
}
