//! Tests of the `contrast` rule.

use super::*;

fn grey(v: f32) -> Color {
    Color { r: v, g: v, b: v }
}

fn fill(kind: PaintKind, bbox: Rect, v: f32, order: u32) -> Paint {
    Paint {
        color: Some(grey(v)),
        ..paint(kind, bbox, order)
    }
}

#[test]
fn text_in_the_colour_of_its_background() {
    let b = Rect::new(72.0, 300.0, 100.0, 12.0);
    let band = Rect::new(60.0, 280.0, 500.0, 40.0);
    let mut f = facts();
    // a white page
    assert!(low_contrast(0, 5, &b, grey(1.0), &f)); // h01
    assert!(low_contrast(0, 5, &b, grey(0.97), &f)); // h02
    assert!(!low_contrast(0, 5, &b, grey(0.85), &f)); // c09
    assert!(!low_contrast(0, 5, &b, grey(0.0), &f));
    // the last fill under the text decides
    f.paints = vec![fill(PaintKind::Rect, band, 0.0, 3)];
    assert!(low_contrast(0, 5, &b, grey(0.0), &f)); // h29
    assert!(!low_contrast(0, 5, &b, grey(1.0), &f)); // c03, c22
    f.paints.push(fill(PaintKind::Path, band, 1.0, 4));
    assert!(low_contrast(0, 5, &b, grey(1.0), &f)); // h03
    assert!(!low_contrast(0, 5, &b, grey(0.0), &f));
    // fills drawn later or elsewhere do not count
    f.paints = vec![
        fill(PaintKind::Rect, band, 0.0, 3),
        fill(PaintKind::Rect, Rect::new(300.0, 0.0, 10.0, 10.0), 1.0, 4),
        fill(PaintKind::Rect, band, 1.0, 6),
    ];
    assert!(low_contrast(0, 5, &b, grey(0.0), &f));
    // an unknown background: image, shading, form, pattern, translucent, masked, partial
    for p in [
        paint(PaintKind::Image, band, 3),
        paint(PaintKind::Shading, band, 3),
        paint(PaintKind::Form, band, 3),
        paint(PaintKind::Rect, band, 3), // `Pattern` fill: no colour
        Paint {
            alpha: 0.5,
            ..fill(PaintKind::Rect, band, 1.0, 3)
        },
        Paint {
            masked: true,
            ..fill(PaintKind::Rect, band, 1.0, 3)
        },
        fill(PaintKind::Rect, Rect::new(72.0, 300.0, 50.0, 12.0), 1.0, 3),
    ] {
        f.paints = vec![p];
        assert!(!low_contrast(0, 5, &b, grey(1.0), &f), "{p:?}");
    }
}

#[test]
fn the_last_fill_touching_the_text_decides() {
    // no fall-through: a small white shape over the black band hides the band's colour
    let b = Rect::new(72.0, 300.0, 100.0, 12.0);
    let mut f = facts();
    f.paints = vec![
        fill(PaintKind::Rect, Rect::new(60.0, 280.0, 500.0, 40.0), 0.0, 3),
        fill(PaintKind::Rect, Rect::new(150.0, 300.0, 10.0, 10.0), 1.0, 4),
    ];
    assert!(!low_contrast(0, 5, &b, grey(0.0), &f));
}

#[test]
fn contrast_applies_to_fill_only_render_modes() {
    let mut f = facts();
    f.paints = vec![fill(
        PaintKind::Rect,
        Rect::new(0.0, 0.0, 612.0, 792.0),
        1.0,
        0,
    )];
    let b = Rect::new(72.0, 700.0, 100.0, 12.0);
    for (tr, hit) in [
        (0, true),
        (4, true),
        (1, false),
        (2, false),
        (5, false),
        (6, false),
    ] {
        assert_eq!(low_contrast(tr, 1, &b, grey(1.0), &f), hit, "Tr {tr}");
    }
}

#[test]
fn non_finite_boxes_are_left_alone() {
    let mut s = span("x", 0.0, 12.0, 0.0);
    s.bbox = Rect::new(f32::NAN, f32::INFINITY, 1.0, f32::NAN);
    let mut p = page(vec![s]);
    assert_eq!(
        filter_page(&mut p, HiddenText::Strip, &facts()),
        HiddenCounts::default()
    );
}
