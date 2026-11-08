use super::*;
use pdf_oxide::geometry::Rect;

fn span(text: &str, x: f32, y: f32, w: f32) -> TextSpan {
    TextSpan {
        text: text.into(),
        bbox: Rect::new(x, y, w, 10.0),
        font_size: 10.0,
        ..Default::default()
    }
}

fn region(col: Option<usize>, spans: Vec<TextSpan>) -> StructuredRegion {
    StructuredRegion {
        kind: RegionRole::BodyBlock,
        text: spans
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        bbox: spans[0].bbox,
        spans,
        column_index: col,
        section_id: None,
    }
}

fn page(regions: Vec<StructuredRegion>) -> StructuredPage {
    StructuredPage {
        page_index: 0,
        page_width: 612.0,
        page_height: 792.0,
        regions,
    }
}

/// `n` lines per side; left lines from x=72, right from x=320; widths from `w(i)`.
fn two_sides(n: usize, wl: impl Fn(usize) -> f32, wr: impl Fn(usize) -> f32) -> StructuredPage {
    let y = |i: usize| 700.0 - 14.0 * i as f32;
    let left = (0..n).map(|i| span("left", 72.0, y(i), wl(i))).collect();
    let right = (0..n).map(|i| span("right", 320.0, y(i), wr(i))).collect();
    page(vec![region(Some(0), left), region(Some(1), right)])
}

#[test]
fn prose_columns_keep_the_gutter() {
    let p = two_sides(12, |i| if i == 5 { 120.0 } else { 220.0 }, |_| 218.0);
    assert!(!false_gutter(&p));
}

#[test]
fn two_column_table_is_a_false_gutter() {
    // labels of varying width on the left, short values on the right
    let p = two_sides(
        8,
        |i| [40.0, 150.0, 60.0, 90.0, 30.0, 140.0, 50.0, 70.0][i],
        |i| [20.0, 35.0, 18.0, 60.0, 25.0, 22.0, 90.0, 30.0][i],
    );
    assert!(false_gutter(&p));
}

#[test]
fn misaligned_short_sidebar_keeps_the_gutter() {
    // a résumé sidebar: short lines of varying width (median/max 0.45, the side is 29 % of
    // the content width) whose baselines sit half a line pitch off the main column's
    let mut p = two_sides(12, |_| 220.0, |_| 0.0);
    let widths = [
        20.0, 100.0, 30.0, 90.0, 25.0, 85.0, 40.0, 70.0, 35.0, 95.0, 30.0, 45.0,
    ];
    p.regions[1].spans = (0..12)
        .map(|i| span("side", 320.0, 693.0 - 14.0 * i as f32, widths[i]))
        .collect();
    assert!(!false_gutter(&p));
}

#[test]
fn aligned_narrow_side_is_a_false_gutter() {
    // a table of contents: filled title lines on the left, a narrow column of page numbers
    // on the same baselines
    let p = two_sides(12, |_| 220.0, |i| if i % 3 == 0 { 12.0 } else { 6.0 });
    assert!(false_gutter(&p));
}

#[test]
fn running_footer_is_not_a_column() {
    // one column of prose; a running footer and the page number at the bottom right form
    // the fork's "column 1" with two lines
    let left = (0..20)
        .map(|i| span("body", 72.0, 700.0 - 14.0 * i as f32, 300.0))
        .collect();
    let right = vec![
        span("Report 2025", 450.0, 40.0, 80.0),
        span("7", 520.0, 26.0, 6.0),
    ];
    let p = page(vec![region(Some(0), left), region(Some(1), right)]);
    assert!(false_gutter(&p));
}

#[test]
fn short_sides_are_a_false_gutter() {
    let p = two_sides(2, |_| 220.0, |_| 220.0);
    assert!(false_gutter(&p));
}

#[test]
fn multi_chunk_side_lines_are_a_false_gutter() {
    let mut p = two_sides(6, |_| 60.0, |_| 220.0);
    // every left line gets a second cell 40 pt to the right: 3-column table split once
    let extra: Vec<TextSpan> = (0..6)
        .map(|i| span("cell", 172.0, 700.0 - 14.0 * i as f32, 60.0))
        .collect();
    p.regions[0].spans.extend(extra);
    assert!(false_gutter(&p));
}

#[test]
fn page_without_columns_is_untouched() {
    let p = page(vec![region(None, vec![span("full", 72.0, 700.0, 400.0)])]);
    assert!(!false_gutter(&p));
}

#[test]
fn non_finite_spans_do_not_panic() {
    let mut p = two_sides(4, |_| 220.0, |_| 220.0);
    p.regions[0].spans[0].bbox = Rect::new(f32::NAN, f32::INFINITY, f32::NAN, 10.0);
    let _ = false_gutter(&p);
}

#[test]
fn misaligned_wide_side_that_does_not_fill_is_a_false_gutter() {
    // the right side is wide (45 % of the content width, not a sidebar), ragged (median/max
    // 0.3) and half a line pitch off the left baselines
    let mut p = two_sides(12, |_| 220.0, |_| 0.0);
    let widths = [
        200.0, 60.0, 50.0, 60.0, 55.0, 60.0, 50.0, 65.0, 60.0, 55.0, 60.0, 50.0,
    ];
    p.regions[1].spans = (0..12)
        .map(|i| span("wide", 320.0, 693.0 - 14.0 * i as f32, widths[i]))
        .collect();
    assert!(false_gutter(&p));
}

#[test]
fn right_column_created_first_is_read_after_left() {
    let p = page(vec![
        region(Some(1), vec![span("R", 320.0, 760.0, 60.0)]),
        region(None, vec![span("Title", 72.0, 740.0, 400.0)]),
        region(Some(0), vec![span("L", 72.0, 700.0, 200.0)]),
    ]);
    let order: Vec<&str> = column_order(&p.regions)
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(order, ["L", "Title", "R"]);
}

#[test]
fn content_above_two_prose_columns_does_not_trip_the_gate() {
    // ru.pdf «Описание»: column 0 also holds the title, two headings and two table rows above
    // the prose (median/max 0.58, a third of its lines beside column 1); the prose itself is
    // four lines on the right next to three on the left
    let mut left = vec![
        span("Title", 34.0, 737.0, 257.0),
        span("Heading", 34.0, 676.0, 90.0),
        span("Header row", 38.0, 643.0, 150.0),
        span("Heading", 34.0, 574.0, 77.0),
    ];
    for y in [624.0, 605.0] {
        for (x, w) in [(38.0, 40.0), (100.0, 18.0), (140.0, 17.0)] {
            left.push(span("cell", x, y, w));
        }
    }
    for (y, w) in [(535.0, 238.0), (523.0, 194.0), (511.0, 259.0)] {
        left.push(span("left", 34.0, y, w));
    }
    let right = [
        (545.0, 209.0),
        (533.0, 219.0),
        (521.0, 213.0),
        (509.0, 52.0),
    ]
    .iter()
    .map(|&(y, w)| span("right", 317.0, y, w))
    .collect();
    let p = page(vec![region(Some(0), left), region(Some(1), right)]);
    assert!(!false_gutter(&p));
}

#[test]
fn many_table_rows_above_two_prose_columns_keep_the_gutter() {
    // a table of six 3-cell rows in column 0 above the prose: 6 of 10 left lines are chunked
    let mut left = vec![span("Title", 34.0, 737.0, 257.0)];
    for i in 0..6 {
        let y = 700.0 - 19.0 * i as f32;
        for (x, w) in [(38.0, 40.0), (100.0, 18.0), (140.0, 17.0)] {
            left.push(span("cell", x, y, w));
        }
    }
    for (y, w) in [(535.0, 238.0), (523.0, 194.0), (511.0, 259.0)] {
        left.push(span("left", 34.0, y, w));
    }
    let right = [
        (545.0, 209.0),
        (533.0, 219.0),
        (521.0, 213.0),
        (509.0, 52.0),
    ]
    .iter()
    .map(|&(y, w)| span("right", 317.0, y, w))
    .collect();
    let p = page(vec![region(Some(0), left), region(Some(1), right)]);
    assert!(!false_gutter(&p));
}

#[test]
fn stacked_sides_without_a_band_are_a_false_gutter() {
    // prose of column 0 entirely above prose of column 1
    let left = (0..6)
        .map(|i| span("top", 72.0, 700.0 - 14.0 * i as f32, 220.0))
        .collect();
    let right = (0..6)
        .map(|i| span("low", 320.0, 500.0 - 14.0 * i as f32, 220.0))
        .collect();
    let p = page(vec![region(Some(0), left), region(Some(1), right)]);
    assert!(false_gutter(&p));
}

#[test]
fn table_inside_the_band_under_prose_is_a_false_gutter() {
    // filled prose above, then a two-column table whose cells are split by the gutter:
    // the band is the table rows, where the left side is chunked cells
    let mut left: Vec<TextSpan> = (0..6)
        .map(|i| span("prose", 72.0, 700.0 - 14.0 * i as f32, 220.0))
        .collect();
    let mut right = Vec::new();
    for i in 0..6 {
        let y = 600.0 - 14.0 * i as f32;
        left.push(span("label", 72.0, y, 50.0));
        left.push(span("cell", 160.0, y, 40.0));
        right.push(span(
            "value",
            320.0,
            y,
            [30.0, 80.0, 20.0, 60.0, 25.0, 90.0][i],
        ));
    }
    let p = page(vec![region(Some(0), left), region(Some(1), right)]);
    assert!(false_gutter(&p));
}

/// `n` rows of two prose columns on shared baselines (left 72–290, right 306–524) under a
/// full-width caption, all in one region: the fork found no corridor over the whole page.
fn interleaved(n: usize) -> StructuredPage {
    let mut spans = vec![span("caption", 72.0, 720.0, 452.0)];
    for i in 0..n {
        let y = 700.0 - 14.0 * i as f32;
        spans.push(span(&format!("L{i}"), 72.0, y, 218.0));
        spans.push(span(&format!("R{i}"), 306.0, y, 218.0));
    }
    page(vec![region(None, spans)])
}

fn texts(p: &StructuredPage) -> Vec<String> {
    p.regions.iter().map(|r| r.text.clone()).collect()
}

#[test]
fn two_columns_inside_one_region_are_split() {
    let p = split_gutters(&interleaved(8)).expect("split");
    let left = (0..8)
        .map(|i| format!("L{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let right = (0..8)
        .map(|i| format!("R{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(texts(&p), ["caption".to_string(), left, right]);
    assert_eq!(p.regions.iter().map(|r| r.spans.len()).sum::<usize>(), 17);
    assert!(p.regions[1].bbox.x + p.regions[1].bbox.width <= 290.5);
}

#[test]
fn short_runs_and_fork_columns_are_not_split() {
    assert!(split_gutters(&interleaved(3)).is_none(), "3 rows < MIN_RUN");
    let p = two_sides(12, |_| 220.0, |_| 218.0);
    assert!(split_gutters(&p).is_none(), "the fork's own column regions");
}

#[test]
fn table_left_of_the_middle_is_not_split() {
    // names and dates of a two-column list under full-width prose: the gap sits at a quarter
    // of the text width
    let mut spans = vec![
        span("prose", 51.0, 720.0, 492.0),
        span("prose", 51.0, 706.0, 492.0),
    ];
    for i in 0..6 {
        let y = 680.0 - 14.0 * i as f32;
        spans.push(span("name", 74.0, y, 96.0));
        spans.push(span("date", 182.0, y, 147.0));
    }
    assert!(split_gutters(&page(vec![region(None, spans)])).is_none());
}

#[test]
fn table_rows_are_not_split() {
    // two cells left of the gap on every row: the left side is chunked like table cells
    let mut spans = Vec::new();
    for i in 0..8 {
        let y = 700.0 - 14.0 * i as f32;
        spans.push(span("a", 72.0, y, 40.0));
        spans.push(span("b", 172.0, y, 40.0));
        spans.push(span("c", 306.0, y, 218.0));
    }
    assert!(split_gutters(&page(vec![region(None, spans)])).is_none());
}

#[test]
fn adversarial_runs_are_bounded() {
    // 19 998 spans in 6 666 rows whose gap moves every row: every row starts a long run that
    // the gate rejects (the right side is chunked like table cells)
    let mut spans = Vec::new();
    for i in 0..6_666 {
        let y = 100_000.0 - 14.0 * i as f32;
        let g = 280.0 + (i % 3) as f32;
        spans.push(span("l", 72.0, y, g - 72.0));
        spans.push(span("r", g + 16.0, y, 20.0));
        spans.push(span("c", g + 100.0, y, 20.0));
    }
    let n = spans.len();
    let p = page(vec![region(None, spans)]);
    let t = std::time::Instant::now();
    let s = split_gutters(&p);
    eprintln!("split_gutters on 19 998 spans: {:?}", t.elapsed());
    assert!(
        s.is_none(),
        "the gate rejects every run: {n} spans left alone"
    );
    // a large page that does split loses no span
    let p = interleaved(9_000);
    let n = p.regions[0].spans.len();
    let t = std::time::Instant::now();
    let s = split_gutters(&p).expect("split");
    eprintln!("split_gutters on {n} spans that split: {:?}", t.elapsed());
    assert_eq!(s.regions.iter().map(|r| r.spans.len()).sum::<usize>(), n);
}

#[test]
fn unusable_spans_stay_in_place_without_panic() {
    let mut p = interleaved(8);
    p.regions[0].spans[3].bbox = Rect::new(f32::NAN, f32::INFINITY, f32::NAN, 10.0);
    p.regions[0].spans.push(span("  ", 72.0, 500.0, 5.0));
    let n = p.regions[0].spans.len();
    let s = split_gutters(&p).expect("split");
    assert_eq!(s.regions.iter().map(|r| r.spans.len()).sum::<usize>(), n);
}
