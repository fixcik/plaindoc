use super::*;
use crate::tables::{CandidateKind, Features, GridCell, Structure};
use plaindoc_core::{Cell, Level, WarningKind};

fn grid(rows: Vec<Vec<GridCell>>, fill: f64, p: f64) -> DetectedGrid {
    DetectedGrid {
        kind: CandidateKind::Lined,
        bbox: [50.0, 100.0, 250.0, 140.0],
        cells: [50.0, 100.0, 250.0, 140.0],
        rows,
        header_rows: 1,
        structure: Structure::Planar,
        probability: p,
        features: Features {
            fill,
            ..Default::default()
        },
    }
}
fn gc(t: &str, cs: u32, rs: u32) -> GridCell {
    GridCell::new(t, cs, rs)
}

#[test]
fn no_tables_returns_empty() {
    let doc = PdfDocument::from_bytes(crate::pdf::tests::one_page_pdf("x")).unwrap();
    let page = doc.extract_structured(0).unwrap();
    assert!(NoTables.detect(&doc, &page).unwrap().is_empty());
}

#[test]
fn grid_becomes_table_block_with_pdf_bbox_and_spans() {
    let d = to_detected(
        &grid(
            vec![vec![gc("Head", 2, 1)], vec![gc("a", 1, 1), gc("b", 1, 1)]],
            1.0,
            0.9,
        ),
        800.0,
    );
    assert_eq!(d.bbox, BBox::new(50.0, 660.0, 200.0, 40.0));
    let Block::Table {
        rows,
        header_rows,
        bbox,
    } = &d.block
    else {
        panic!("not a table")
    };
    assert_eq!(
        (
            rows[0][0].colspan,
            rows[0][0].rowspan,
            rows[1].len(),
            *header_rows
        ),
        (2, 1, 2, 1)
    );
    assert_eq!(*bbox, Some(d.bbox));
    assert!(d.warnings.is_empty());
}

#[test]
fn sparse_or_doubtful_tables_warn() {
    let rows = vec![
        vec![gc("a", 1, 1), gc("", 1, 1)],
        vec![gc("", 1, 1), gc("b", 1, 1)],
    ];
    let w = to_detected(&grid(rows.clone(), 0.4, 0.9), 800.0).warnings;
    assert!(
        w[0].kind == WarningKind::TableUncertain
            && w[0].level == Level::Warn
            && w[0].message.contains("60%")
    );
    let w = to_detected(&grid(rows, 1.0, 0.2), 800.0).warnings;
    assert!(w[0].message.contains("0.20"));
}

#[test]
fn continuation_needs_edges_and_same_columns() {
    let t = |y: f32, h: f32, cols: usize| {
        let rows = vec![(0..cols).map(|_| Cell::new("x")).collect(); 2];
        DetectedTable::new(
            BBox::new(50.0, y, 400.0, h),
            Block::Table {
                rows,
                header_rows: 1,
                bbox: None,
            },
        )
    };
    assert!(continues(&t(60.0, 500.0, 3), &t(300.0, 450.0, 3), 800.0));
    assert!(
        !continues(&t(60.0, 500.0, 3), &t(300.0, 450.0, 4), 800.0),
        "columns differ"
    );
    assert!(
        !continues(&t(400.0, 300.0, 3), &t(300.0, 450.0, 3), 800.0),
        "previous ends mid-page"
    );
    assert!(
        !continues(&t(60.0, 500.0, 3), &t(100.0, 300.0, 3), 800.0),
        "next starts mid-page"
    );
}

fn parse(pdf: Vec<u8>) -> plaindoc_core::Document {
    use plaindoc_core::{Format, Input, ParseOptions, Parser};
    crate::pdf::PdfParser::new()
        .parse(Input::Bytes(pdf), Format::Pdf, &ParseOptions::default())
        .unwrap()
}

fn tables(d: &plaindoc_core::Document) -> Vec<&Vec<Vec<Cell>>> {
    d.sections
        .iter()
        .flat_map(|s| &s.blocks)
        .filter_map(|b| match b {
            Block::Table { rows, .. } => Some(rows),
            _ => None,
        })
        .collect()
}

const XS: [f32; 4] = [72.0, 200.0, 328.0, 456.0];
const YS: [f32; 5] = [640.0, 620.0, 600.0, 580.0, 560.0];

/// Paragraph plus a 3-column table; row 0 is one merged header cell when `span`.
fn table_text(span: bool) -> String {
    let cells = [
        ["Name", "Qty", "Price"],
        ["Apples", "12", "3.50"],
        ["Pears", "7", "4.10"],
        ["Plums", "30", "1.25"],
    ];
    let mut s =
        String::from("BT /F1 11 Tf 72 700 Td (Quarterly figures are listed below.) Tj ET\n");
    for (r, row) in cells.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            let t = if span && r == 0 {
                (c == 0).then_some("Fruit report 2025")
            } else {
                Some(*t)
            };
            if let Some(t) = t {
                s += &format!(
                    "BT /F1 11 Tf {} {} Td ({t}) Tj ET\n",
                    XS[c] + 6.0,
                    YS[r] - 14.0
                );
            }
        }
    }
    s
}

/// Rulings as 0.5-pt filled boxes: one `re f` each, or all in one path with a single `f`.
fn filled_rules(span: bool, one_path: bool) -> String {
    let mut boxes = Vec::new();
    for y in YS {
        boxes.push((XS[0], y - 0.25, XS[3] - XS[0], 0.5));
    }
    for (i, x) in XS.iter().enumerate() {
        let top = if span && i != 0 && i != 3 {
            YS[1]
        } else {
            YS[0]
        };
        boxes.push((x - 0.25, YS[4], 0.5, top - YS[4]));
    }
    let mut s = String::new();
    for (x, y, w, h) in boxes {
        s += &format!("{x} {y} {w} {h} re{}\n", if one_path { "" } else { " f" });
    }
    if one_path {
        s += "f\n";
    }
    s
}

#[test]
fn fill_only_composite_path_keeps_merged_header() {
    for one_path in [false, true] {
        let d = parse(crate::pdf::tests::raw_pdf(
            "0 0 612 792",
            &(filled_rules(true, one_path) + &table_text(true)),
        ));
        let t = tables(&d);
        assert_eq!(t.len(), 1, "one_path={one_path}");
        assert_eq!(
            (
                t[0].len(),
                t[0][0].len(),
                t[0][0][0].colspan,
                t[0][0][0].text().as_str()
            ),
            (4, 1, 3, "Fruit report 2025"),
            "one_path={one_path}"
        );
        assert!(
            d.to_markdown()
                .contains("Quarterly figures are listed below.")
        );
    }
}

#[test]
fn table_on_a_page_with_an_offset_media_box_is_found_in_pdf_coordinates() {
    // The same page shifted by (10000, 10000): clipping is relative to the MediaBox, the bbox stays PDF.
    let body = filled_rules(false, false) + &table_text(false);
    let d = parse(crate::pdf::tests::raw_pdf(
        "10000 10000 10612 10792",
        &format!("q 1 0 0 1 10000 10000 cm\n{body}Q\n"),
    ));
    let bb = d
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .find_map(|b| match b {
            Block::Table { bbox, .. } => *bbox,
            _ => None,
        })
        .unwrap_or_else(|| panic!("no table: {}", d.to_markdown()));
    let (x, y) = (bb.x, bb.y);
    assert!(
        (x - 10_071.75).abs() < 1.0 && (y - 10_559.75).abs() < 1.0,
        "{bb:?}"
    );
    assert!(
        (bb.w - 384.5).abs() < 1.0 && (bb.h - 80.5).abs() < 1.0,
        "{bb:?}"
    );
}

#[test]
fn landscape_table_on_a_portrait_page_is_not_a_table() {
    // The whole table rotated by 90° with `cm`: read unrotated it is garbage cells eating the paragraph.
    let body = filled_rules(false, false) + &table_text(false);
    let d = parse(crate::pdf::tests::raw_pdf(
        "0 0 612 792",
        &format!("q 0 1 -1 0 702 90 cm\n{body}Q\n"),
    ));
    assert!(tables(&d).is_empty(), "{}", d.to_markdown());
}

#[test]
fn rotated_header_glyphs_reach_the_cell() {
    // "2025" written bottom-to-top in the "Price" header, one positioned glyph at a time (as Chrome
    // prints rotated text): extract_words gives one word per glyph, the adapter merges the run.
    let mut s = filled_rules(false, false) + &table_text(false);
    for (i, g) in "2025".chars().enumerate() {
        let y = YS[1] + 1.0 + 5.0 * i as f32;
        s += &format!("BT /F1 9 Tf 0 1 -1 0 420 {y} Tm ({g}) Tj ET\n");
    }
    let d = parse(crate::pdf::tests::raw_pdf("0 0 612 792", &s));
    let t = tables(&d);
    assert_eq!(t.len(), 1, "{}", d.to_markdown());
    assert!(
        t[0].iter().flatten().any(|c| c.text().contains("2025")),
        "{:?}",
        t[0]
    );
}

#[test]
fn dense_mesh_outside_the_page_is_skipped_with_a_warning() {
    // 6 002 stroked segments drawn from the page corner to 9 000 pt: 259 MB RSS without the limits.
    let mut s = String::from("0.5 w\n");
    for k in 0..=3000 {
        let v = 3 * k;
        s += &format!("0 {v} m 9000 {v} l S\n{v} 0 m {v} 9000 l S\n");
    }
    s += "BT /F1 2 Tf 100 700 Td (77 77 77 77 77 77) Tj ET\n";
    let d = parse(crate::pdf::tests::raw_pdf("0 0 612 792", &s));
    assert!(tables(&d).is_empty());
    let w = &d.sections[0].warnings;
    assert!(
        w.iter().any(|w| w.kind == WarningKind::TableUncertain
            && w.level == Level::Warn
            && w.message
                .contains("limit exceeded: table detector: cells of a grid")),
        "{w:?}"
    );
}

#[test]
fn table_limit_kinds_cover_every_literal() {
    use plaindoc_core::LimitKind::*;
    for (what, want) in [
        ("words on the page", TableWords),
        ("paths and segments before clipping", TablePrims),
        ("paths and segments on the page", TablePrims),
        ("primitives derived from the page", TablePrims),
        ("cells of a grid", TableCells),
        ("model regions on the page", TableModelRegions),
    ] {
        assert_eq!(super::table_limit_kind(what), want, "{what}");
    }
    let e = crate::pdf::table_input::limit("words on the page", 9, 1);
    assert_eq!(super::adapter_limit(&e), Some(TableWords));
    assert_eq!(
        super::adapter_limit(&plaindoc_core::ParseError::Encrypted),
        None
    );
}

#[test]
fn dense_mesh_counts_the_cells_limit() {
    use plaindoc_core::{Format, Input, LimitKind, ParseOptions, ParseStats, Parser};
    let mut s = String::from("0.5 w\n");
    for k in 0..=3000 {
        let v = 3 * k;
        s += &format!("0 {v} m 9000 {v} l S\n{v} 0 m {v} 9000 l S\n");
    }
    s += "BT /F1 2 Tf 100 700 Td (77 77 77 77 77 77) Tj ET\n";
    let mut st = ParseStats::default();
    crate::pdf::PdfParser::new()
        .parse_with_stats(
            Input::Bytes(crate::pdf::tests::raw_pdf("0 0 612 792", &s)),
            Format::Pdf,
            &ParseOptions::default(),
            &mut st,
        )
        .unwrap();
    assert_eq!(
        st.limits.get(&LimitKind::TableCells),
        Some(&1),
        "{:?}",
        st.limits
    );
    assert!(st.sizes.table_prims >= 6002, "{}", st.sizes.table_prims);
}

/// A 2x3 ruled table whose verticals overhang the top (`header`) or bottom rule by 7 pt, with a 5-pt line
/// of `label{i}` words in that band (final review C1).
fn edge_band_pdf(header: bool, label: &str) -> Vec<u8> {
    let (v0, rules, band, cells) = if header {
        (600, [645, 625, 605], 647, [630, 610])
    } else {
        (603, [650, 630, 610], 604, [635, 615])
    };
    let mut s = String::new();
    for x in [50.0, 150.0, 250.0, 350.0] {
        s += &format!(
            "{} {v0} 0.5 {} re f\n",
            x - 0.25,
            if header { 52 } else { 47 }
        );
    }
    for y in rules {
        s += &format!("50 {} 300 0.5 re f\n", f64::from(y) - 0.25);
    }
    for (i, x) in [55, 155, 255].into_iter().enumerate() {
        s += &format!("BT /F1 5 Tf {x} {band} Td ({label}{i}) Tj ET\n");
        for (r, y) in cells.into_iter().enumerate() {
            s += &format!("BT /F1 10 Tf {x} {y} Td (v{r}{i}) Tj ET\n");
        }
    }
    crate::pdf::tests::raw_pdf("0 0 612 792", &s)
}

#[test]
fn words_in_the_edge_band_the_grid_did_not_take_stay_in_the_text() {
    for (header, label) in [(true, "HdrX"), (false, "FootZ")] {
        let d = parse(edge_band_pdf(header, label));
        let t = tables(&d);
        let md = d.to_markdown();
        assert_eq!(t.len(), 1, "{md}");
        assert!(
            t[0].iter().flatten().any(|c| c.text() == "v00"),
            "{:?}",
            t[0]
        );
        for i in 0..3 {
            let w = format!("{label}{i}");
            assert_eq!(md.matches(&w).count(), 1, "{w} once: {md}");
        }
    }
}

fn scan_word(text: &str, bbox: BBox, angle: f32) -> ScanWord {
    ScanWord {
        text: text.into(),
        bbox,
        char_x: Vec::new(),
        conf: 0.9,
        font_size: 11.0,
        line: 0,
        block: 0,
        angle,
    }
}

fn scan(words: Vec<ScanWord>, segments: Vec<ScanSegment>) -> ScanTableInputs {
    ScanTableInputs {
        page_index: 0,
        page_width: 800.0,
        page_height: 1000.0,
        words,
        segments,
        skew_deg: 0.0,
        rotation: 0,
        model_regions: Vec::new(),
    }
}

#[test]
fn scan_inputs_become_page_input_in_top_left() {
    let mut w = scan_word("x", BBox::new(50.0, 935.0, 50.0, 15.0), 90.0);
    w.char_x = vec![(50.0, 75.0), (75.0, 100.0)];
    let seg = ScanSegment {
        x1: 5.0,
        y1: 850.0,
        x2: 250.0,
        y2: 850.0,
    };
    let input = scan_page_input(&scan(vec![w], vec![seg]));
    assert_eq!((input.width, input.height), (800.0, 1000.0));
    assert_eq!(input.words[0].rect, Rect::new(50.0, 50.0, 50.0, 15.0));
    assert_eq!(input.words[0].char_x, vec![(50.0, 75.0), (75.0, 100.0)]);
    assert!(input.words[0].rotated, "angle 90");
    assert_eq!(
        input.segments[0],
        InputSegment {
            x1: 5.0,
            y1: 150.0,
            x2: 250.0,
            y2: 150.0,
            raster: true
        }
    );
    assert!(input.model_regions.is_empty());

    // A 3x3 grid of raster rules with nine words (y up): one table, bbox in the input frame.
    let (xs, ys) = (
        [72.0f32, 200.0, 328.0, 456.0],
        [640.0f32, 620.0, 600.0, 580.0],
    );
    let mut segs: Vec<ScanSegment> = ys
        .iter()
        .map(|&y| ScanSegment {
            x1: xs[0],
            y1: y,
            x2: xs[3],
            y2: y,
        })
        .collect();
    segs.extend(xs.iter().map(|&x| ScanSegment {
        x1: x,
        y1: ys[0],
        x2: x,
        y2: ys[3],
    }));
    let texts = [
        ["Name", "Qty", "Price"],
        ["Apples", "12", "3.50"],
        ["Pears", "7", "4.10"],
    ];
    let mut words = Vec::new();
    for (r, row) in texts.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            let b = BBox::new(xs[c] + 6.0, ys[r + 1] + 5.0, 6.0 * t.len() as f32, 10.0);
            words.push(scan_word(t, b, 0.0));
        }
    }
    let t = OwnTables::default()
        .detect_scan(&scan(words, segs))
        .unwrap();
    assert_eq!(t.len(), 1, "{t:?}");
    let b = t[0].bbox;
    assert!(
        (b.x - 72.0).abs() < 1.5
            && (b.y - 580.0).abs() < 1.5
            && (b.w - 384.0).abs() < 1.5
            && (b.h - 60.0).abs() < 1.5,
        "{b:?}"
    );
    let Block::Table { rows, .. } = &t[0].block else {
        panic!("not a table")
    };
    assert_eq!((rows.len(), rows[0].len()), (3, 3));
    assert_eq!(rows[1][0].text(), "Apples");
}

#[test]
fn default_detect_scan_finds_nothing() {
    assert!(
        NoTables
            .detect_scan(&scan(Vec::new(), Vec::new()))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn own_detector_finds_a_ruled_table_in_a_real_pdf() {
    let doc = PdfDocument::from_bytes(crate::pdf::tests::ruled_table_pdf()).unwrap();
    doc.set_char_x_offset_refinement(false);
    let page = doc.extract_structured(0).unwrap();
    let t = OwnTables::default().detect(&doc, &page).unwrap();
    assert_eq!(t.len(), 1);
    let Block::Table {
        rows, header_rows, ..
    } = &t[0].block
    else {
        panic!("not a table")
    };
    assert_eq!((rows.len(), rows[0].len(), *header_rows), (4, 3, 1));
}

/// A résumé page (`tables::detect::gutter_test::sidebar_tail` as a PDF): a main column at x 50 (lines
/// of 10 words) and a sidebar at x 420 (3 words) down the page, then their last lines side by side. The
/// words are numbered in the reading order of their column (`b...` main, `d...` sidebar) and made of
/// letters of one Helvetica width, so every full line ends at the same x.
fn sidebar_pdf() -> (Vec<u8>, Vec<String>, Vec<String>) {
    const L: &[u8] = b"abcdeghnopqu";
    let tok = |c: char, i: usize| {
        let l = |n: usize| char::from(L[n % L.len()]);
        format!("{c}{}{}{}a", l(i / 144), l(i / 12), l(i))
    };
    let (mut s, mut main, mut side) = (String::new(), Vec::new(), Vec::new());
    let mut put = |x: f64, top: f64, n: usize, side_col: bool| {
        let v = if side_col { &mut side } else { &mut main };
        let line: Vec<String> = (0..n)
            .map(|_| {
                let t = tok(if side_col { 'd' } else { 'b' }, v.len());
                v.push(t.clone());
                t
            })
            .collect();
        if n > 0 {
            s += &format!(
                "BT /F1 9 Tf {x} {} Td ({}) Tj ET\n",
                833.0 - top,
                line.join(" ")
            );
        }
    };
    let mut y = 80.0;
    while y < 640.0 {
        put(50.0, y, 10, false);
        put(420.0, y + 3.0, 3, true);
        y += 12.0;
    }
    for (l, r) in [(1, 3), (3, 1), (10, 2), (5, 3), (0, 2)] {
        y += 12.0;
        put(50.0, y, l, false);
        put(420.0, y, r, true);
    }
    (crate::pdf::tests::raw_pdf("0 0 595 842", &s), main, side)
}

/// The doubtful 2-column tail of a résumé page is no table, and its words reach the page text exactly as
/// without table detection (same blocks as `NoTables`), each once.
#[test]
fn doubtful_sidebar_tail_stays_page_text() {
    use plaindoc_core::{Format, Input, OcrMode, ParseOptions, Parser};
    let (pdf, main, side) = sidebar_pdf();
    // The generated words look like garbage to the need-OCR classifier: keep the text layer.
    let mut opts = ParseOptions::default();
    opts.ocr = OcrMode::Never;
    let with = |tables: std::sync::Arc<dyn TableDetector>| {
        crate::pdf::PdfParser::with_config(crate::pdf::PdfConfig {
            tables,
            ..Default::default()
        })
        .parse(Input::Bytes(pdf.clone()), Format::Pdf, &opts)
        .unwrap()
    };
    let off = with(std::sync::Arc::new(OwnTables {
        config: TableConfig {
            uncertain_page_columns: false,
            ..TableConfig::default()
        },
    }));
    assert_eq!(tables(&off).len(), 1, "{}", off.to_markdown());
    let d = with(std::sync::Arc::new(OwnTables::default()));
    assert!(tables(&d).is_empty(), "{}", d.to_markdown());
    assert_eq!(
        d.sections[0].blocks,
        with(std::sync::Arc::new(NoTables)).sections[0].blocks
    );
    let md = d.to_markdown();
    for w in main.iter().chain(&side) {
        assert_eq!(md.matches(w.as_str()).count(), 1, "{w}");
    }
}
