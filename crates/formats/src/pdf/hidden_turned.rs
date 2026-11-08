//! Span extents for `hidden`'s `tiny` rule on spans the fork marks `turned_bbox`.

use pdf_oxide::layout::TextSpan;

/// A span's line height and advance per non-whitespace char: a `turned_bbox` span turned a
/// quarter (90/270) has its physical box (the fork turned it) with the run along y, so its sides
/// swap back; a half turn (180) keeps its sides.
pub(crate) fn run_extent(s: &TextSpan) -> (f32, f32) {
    let quarter = s.turned_bbox && (s.rotation_degrees.rem_euclid(180.0) - 90.0).abs() < 1.0;
    let (h, w) = if quarter {
        (s.bbox.width, s.bbox.height)
    } else {
        (s.bbox.height, s.bbox.width)
    };
    let n = s.text.chars().filter(|c| !c.is_whitespace()).count().max(1) as f32;
    (h, w / n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_oxide::geometry::Rect;

    #[test]
    fn a_turned_span_swaps_its_sides_back() {
        // govdocs 002985: a turned `-` run of a 6.6 pt font, 0.9 pt long in raw y
        let s = |turned_bbox, rotation_degrees| TextSpan {
            text: "- -".into(),
            bbox: Rect::new(282.0, 681.0, 6.6, 0.9),
            turned_bbox,
            rotation_degrees,
            ..Default::default()
        };
        assert_eq!(run_extent(&s(true, 90.0)), (6.6, 0.45));
        assert_eq!(run_extent(&s(true, -90.0)), (6.6, 0.45));
        assert_eq!(run_extent(&s(false, 0.0)), (0.9, 3.3));
        // a half turn keeps its sides: width is the advance
        assert_eq!(run_extent(&s(true, 180.0)), (0.9, 3.3));
    }
}
