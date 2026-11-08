//! `PdfDocument` page → [`PageInput`] for the table detector. Mirrors the spike dumper
//! (`pdf-spike-segs dump`): words from `extract_words`, table-primitive paths as rendered boxes, and
//! stroked multi-subpath paths (a whole grid drawn as one path) expanded into segments. Converts the
//! PDF bottom-left origin to the detector's top-left origin.
//!
//! Deliberately beyond the dumper (spec §10 p. 1; SP4 review I1, I2): a fill-only multi-subpath path
//! (rulings as many `re` and one `f`, as Word/LibreOffice export them) gives one filled box per subpath
//! instead of one box around the whole grid; a curve moves the current point of a stroked composite path
//! (rounded cells); words carry `rotated`. `page_input_with(.., true)` keeps the dumper behaviour for the
//! parity test.

use pdf_oxide::PdfDocument;
use pdf_oxide::elements::PathOperation as Op;
use plaindoc_core::{ParseError, Result};

use crate::tables::{
    ADAPTER_MAX_PRIMS, ADAPTER_MAX_WORDS, InputPath, InputSegment, InputWord, PageInput, Rect,
};

fn engine(e: pdf_oxide::Error) -> ParseError {
    ParseError::Internal(format!("table input: {e}"))
}

/// The page warning of a detector limit (`OwnTables::detect` maps `PageTables::skipped` the same way).
pub(crate) fn limit(what: &str, count: usize, limit: usize) -> ParseError {
    ParseError::Limit {
        kind: format!("table detector: {what} ({count})"),
        limit: limit as u64,
    }
}

/// `extract_words` never merges glyphs of a rotated run (vertical header text): each glyph is its own
/// word. Glyphs of one run (same rotation, same column, consecutive content order) become one word in
/// content order, which is their reading order.
fn merge_rotated_runs(words: Vec<pdf_oxide::layout::Word>) -> Vec<pdf_oxide::layout::Word> {
    let (mut rot, mut out): (Vec<_>, Vec<_>) =
        words.into_iter().partition(|w| w.rotation_degrees != 0.0);
    rot.sort_by_key(|w| w.sequence);
    let mut run: Option<pdf_oxide::layout::Word> = None;
    for w in rot {
        match &mut run {
            Some(r)
                if w.sequence == r.sequence + 1
                    && w.rotation_degrees == r.rotation_degrees
                    && (w.bbox.x - r.bbox.x).abs() <= 1.0
                    && w.bbox.y.max(r.bbox.y) - w.bbox.y.min(r.bbox.y)
                        <= r.bbox.height + 1.5 * w.bbox.height =>
            {
                r.text.push_str(&w.text);
                r.bbox = r.bbox.union(&w.bbox);
                r.sequence = w.sequence;
                r.chars.clear();
                r.char_visibility.clear();
            }
            _ => out.extend(run.replace(w)),
        }
    }
    out.extend(run);
    out
}

/// MediaBox of the page; the detector works in page-local coordinates (origin at the box corner).
fn media_box(doc: &PdfDocument, page: usize) -> (f32, f32, f32, f32) {
    doc.get_page_media_box(page)
        .unwrap_or((0.0, 0.0, 612.0, 792.0))
}

/// `(x of the left edge, y of the top edge)` of the MediaBox in PDF coordinates: what the page-local
/// detector output must be shifted by to get PDF coordinates back.
pub(crate) fn page_origin(doc: &PdfDocument, page: usize) -> (f64, f64) {
    let mb = media_box(doc, page);
    (f64::from(mb.0), f64::from(mb.3))
}

pub(crate) fn page_input(doc: &PdfDocument, page: usize) -> Result<PageInput> {
    page_input_with(doc, page, false)
}

/// `spike = true` reproduces the spike dumps exactly (parity test): no rotated-run merge, no `rotated`
/// flag, fill-only composite paths as one box, curves ignored.
fn page_input_with(doc: &PdfDocument, page: usize, spike: bool) -> Result<PageInput> {
    let mb = media_box(doc, page);
    let (width, height) = (f64::from(mb.2 - mb.0), f64::from(mb.3 - mb.1));
    // Page-local coordinates: x from the left edge, top-down y from the top edge of the MediaBox.
    let (ox, top_y) = page_origin(doc, page);
    // Shared with `classify_page`: read in place, only the rotated glyphs (merged below) are copied.
    let words = doc.extract_words_shared(page).map_err(engine)?;
    if words.len() > ADAPTER_MAX_WORDS {
        return Err(limit("words on the page", words.len(), ADAPTER_MAX_WORDS));
    }
    let input_word = |w: &pdf_oxide::layout::Word| {
        let (x, y, ww, wh) = (
            f64::from(w.bbox.x),
            f64::from(w.bbox.y),
            f64::from(w.bbox.width),
            f64::from(w.bbox.height),
        );
        let char_x = if w.chars.len() == w.text.chars().count() {
            w.chars
                .iter()
                .map(|c| {
                    (
                        f64::from(c.bbox.x) - ox,
                        f64::from(c.bbox.x + c.bbox.width) - ox,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        InputWord {
            rect: Rect::new(x - ox, top_y - (y + wh), ww, wh),
            text: w.text.clone(),
            font_size: f64::from(w.avg_font_size),
            char_x,
            rotated: !spike && w.rotation_degrees != 0.0,
        }
    };
    // Order of `merge_rotated_runs` on the whole list: upright words in content order, then the runs.
    let (upright, rotated): (Vec<_>, Vec<_>) = words
        .iter()
        .partition(|w| spike || w.rotation_degrees == 0.0);
    let mut words: Vec<InputWord> = upright.into_iter().map(input_word).collect();
    let rotated = rotated.into_iter().cloned().collect();
    words.extend(merge_rotated_runs(rotated).iter().map(input_word));
    let (mut paths, mut segments) = (Vec::new(), Vec::new());
    // Counted before expanding (a stroked `re` is 4 segments): over the cap the page is skipped before the
    // copies are made (SP4 final review I-3).
    let mut prims = 0usize;
    for q in doc
        .extract_paths_shared(page)
        .map_err(engine)?
        .iter()
        .filter(|q| q.is_table_primitive())
    {
        let subpaths = q
            .operations
            .iter()
            .filter(|o| matches!(o, Op::MoveTo(..) | Op::Rectangle(..)))
            .count();
        prims += if subpaths > 1 && q.has_stroke() {
            q.operations
                .iter()
                .map(|o| match o {
                    Op::Rectangle(..) => 4,
                    Op::LineTo(..) | Op::ClosePath => 1,
                    _ => 0,
                })
                .sum()
        } else if subpaths > 1 && !spike {
            subpaths
        } else {
            1
        };
        if prims > ADAPTER_MAX_PRIMS {
            return Err(limit(
                "paths and segments on the page",
                prims,
                ADAPTER_MAX_PRIMS,
            ));
        }
        if subpaths > 1 && q.has_stroke() {
            let mut seg = |a: f32, b: f32, c: f32, d: f32| {
                segments.push(InputSegment {
                    x1: f64::from(a) - ox,
                    y1: top_y - f64::from(b),
                    x2: f64::from(c) - ox,
                    y2: top_y - f64::from(d),
                    raster: false,
                })
            };
            let (mut cx, mut cy, mut sx, mut sy) = (0f32, 0f32, 0f32, 0f32);
            for op in &q.operations {
                match *op {
                    Op::MoveTo(x, y) => (cx, cy, sx, sy) = (x, y, x, y),
                    Op::LineTo(x, y) => {
                        seg(cx, cy, x, y);
                        (cx, cy) = (x, y);
                    }
                    Op::ClosePath => {
                        seg(cx, cy, sx, sy);
                        (cx, cy) = (sx, sy);
                    }
                    Op::Rectangle(x, y, w, h) => {
                        seg(x, y, x + w, y);
                        seg(x + w, y, x + w, y + h);
                        seg(x + w, y + h, x, y + h);
                        seg(x, y + h, x, y);
                    }
                    Op::CurveTo(.., x, y) if !spike => (cx, cy) = (x, y),
                    _ => {}
                }
            }
            continue;
        }
        if subpaths > 1 && !spike {
            // Fill-only composite path: one filled box per subpath (bottom-left x0, y0, x1, y1).
            let mut boxes: Vec<[f32; 4]> = Vec::new();
            let mut add = |x: f32, y: f32, new: bool| match boxes.last_mut() {
                Some(b) if !new => *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
                _ => boxes.push([x, y, x, y]),
            };
            for op in &q.operations {
                match *op {
                    Op::MoveTo(x, y) => add(x, y, true),
                    Op::LineTo(x, y) | Op::CurveTo(.., x, y) => add(x, y, false),
                    Op::Rectangle(x, y, w, h) => {
                        add(x, y, true);
                        add(x + w, y + h, false);
                    }
                    Op::ClosePath => {}
                }
            }
            paths.extend(boxes.into_iter().map(|[x0, y0, x1, y1]| {
                let (x, y, w, h) = (
                    f64::from(x0),
                    f64::from(y0),
                    f64::from(x1 - x0),
                    f64::from(y1 - y0),
                );
                InputPath {
                    rect: Rect::new(x - ox, top_y - (y + h), w, h),
                    stroke: false,
                    fill: true,
                }
            }));
            continue;
        }
        let r = q.rendered_bbox();
        let (x, y, w, h) = (
            f64::from(r.x),
            f64::from(r.y),
            f64::from(r.width),
            f64::from(r.height),
        );
        paths.push(InputPath {
            rect: Rect::new(x - ox, top_y - (y + h), w, h),
            stroke: q.has_stroke(),
            fill: q.has_fill(),
        });
    }
    Ok(PageInput {
        width,
        height,
        words,
        paths,
        segments,
        model_regions: Vec::new(),
        image_boxes: Vec::new(),
    })
}

/// Boxes of the images painted on the page (page-local, top-left origin) for
/// [`crate::tables::PageInput::image_boxes`] from `page_image_handles`: one scan of the content stream for
/// `Do` and inline images (Forms recursed), nothing decoded (T18 gate). An engine error gives no boxes.
/// Not called by the shipped path; SP5-R2 calls it only where [`crate::tables::may_contain_table`] says no
/// (a page can list thousands of images: PMC3058066 p4, 9131, ~0.5 s).
#[allow(dead_code)] // used from SP5-R2
pub(crate) fn image_boxes(doc: &PdfDocument, page: usize) -> Vec<Rect> {
    let (ox, top_y) = page_origin(doc, page);
    let Ok(handles) = doc.page_image_handles(page) else {
        return Vec::new();
    };
    handles
        .iter()
        .map(|h| image_rect([h.bbox.x, h.bbox.y, h.bbox.width, h.bbox.height], ox, top_y))
        .collect()
}

/// PDF image box `[x, y, width, height]` (bottom-left origin) as a page-local top-left [`Rect`]. A flipped
/// CTM can leave a negative size: normalised to the low corner.
fn image_rect([x, y, w, h]: [f32; 4], ox: f64, top_y: f64) -> Rect {
    let (x0, x1) = (x.min(x + w), x.max(x + w));
    let (y0, y1) = (y.min(y + h), y.max(y + h));
    Rect::new(
        f64::from(x0) - ox,
        top_y - f64::from(y1),
        f64::from(x1 - x0),
        f64::from(y1 - y0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dump::page_from_dump;
    use crate::tables::{TableConfig, detect_tables};

    /// T18 pre-model gate on the real PDF path, with and without the image rule (docs/tables-eval.md, "T18
    /// pre-model gate"): `TABLES_GATE_PDF=<name>:<pdf dir>:<labels.jsonl>`, several separated by `;`
    /// (labels as for `page_gate_on_dumps`; `split == "test"` rows are not scored). Prints recall and the
    /// passed shares, the sensitivity to the image share, and the cost of `image_boxes`.
    #[test]
    #[ignore = "needs TABLES_GATE_PDF (corpora are kept outside the repo)"]
    fn page_gate_on_pdfs() {
        use crate::tables::{LARGE_IMAGE_SHARE, area_on_page, has_large_image, may_contain_table};
        let Ok(spec) = std::env::var("TABLES_GATE_PDF") else {
            eprintln!("skip: set TABLES_GATE_PDF=<name>:<pdf dir>:<labels.jsonl>[;...]");
            return;
        };
        let cfg = TableConfig::default();
        const SHARES: [f64; 4] = [0.05, LARGE_IMAGE_SHARE, 0.2, 0.3];
        for item in spec.split(';').filter(|s| !s.is_empty()) {
            let parts: Vec<&str> = item.split(':').collect();
            let [name, pdfs, labels] = parts[..] else {
                panic!("TABLES_GATE_PDF item {item:?}: expected <name>:<pdf dir>:<labels.jsonl>");
            };
            let mut docs: std::collections::BTreeMap<String, Vec<(usize, bool)>> =
                Default::default();
            for l in std::fs::read_to_string(labels).unwrap().lines() {
                let r: serde_json::Value = serde_json::from_str(l).unwrap();
                if r["split"] == "test" {
                    continue;
                }
                docs.entry(r["doc"].as_str().unwrap().to_string())
                    .or_default()
                    .push((r["page"].as_u64().unwrap() as usize, r["label"] == 1));
            }
            // [positives, passed without images, passed with images; negatives, ...]
            let mut n = [0usize; 6];
            let mut sens = [[0usize; 2]; SHARES.len()];
            let (mut img_ms, mut misses, mut unreadable, mut lazy_ms) =
                (Vec::new(), Vec::new(), 0usize, 0f64);
            for (doc, pages) in &docs {
                let Ok(pdf) = PdfDocument::open(format!("{pdfs}/{doc}.pdf")) else {
                    continue;
                };
                for &(i, label) in pages {
                    let Ok(mut p) =
                        page_input(&pdf, i).inspect_err(|e| eprintln!("{doc}:{i}: {e}"))
                    else {
                        unreadable += 1;
                        continue;
                    };
                    let t = std::time::Instant::now();
                    let boxes = image_boxes(&pdf, i);
                    let ms = t.elapsed().as_secs_f64() * 1e3;
                    img_ms.push(ms);
                    let found = detect_tables(&p, &cfg);
                    let plain = may_contain_table(&p, &found);
                    // SP5-R2 order: the images are read only when the cheap rules say no.
                    if !plain {
                        lazy_ms += ms;
                    }
                    if ms > 5.0 {
                        eprintln!(
                            "slow image_boxes {doc}:{i}: {ms:.1} ms, {} images, cheap rules pass: {plain}",
                            boxes.len()
                        );
                    }
                    let area = p.width * p.height;
                    let share = boxes
                        .iter()
                        .map(|r| area_on_page(&p, r).map_or(0.0, |a| a / area))
                        .fold(0.0, f64::max);
                    p.image_boxes = boxes;
                    // SP5-R2 order (images only where the cheap rules say no) equals the eager call.
                    let with = plain || has_large_image(&p);
                    assert_eq!(with, may_contain_table(&p, &found), "{doc}:{i}");
                    assert_eq!(with, plain || share >= LARGE_IMAGE_SHARE, "{doc}:{i}");
                    let k = if label { 0 } else { 3 };
                    n[k] += 1;
                    n[k + 1] += usize::from(plain);
                    n[k + 2] += usize::from(with);
                    for (s, &th) in sens.iter_mut().zip(&SHARES) {
                        s[usize::from(!label)] += usize::from(plain || share >= th);
                    }
                    if label && !with {
                        misses.push(format!("{doc}:{i} image share {share:.3}"));
                    }
                }
            }
            let r = |a: usize, b: usize| a as f64 / b.max(1) as f64;
            println!(
                "gate pdf {name}: recall {}/{} = {:.3} without images, {}/{} = {:.3} with; passed table-free {:.3} / {:.3}; passed all {:.3} / {:.3}; misses {misses:?}",
                n[1],
                n[0],
                r(n[1], n[0]),
                n[2],
                n[0],
                r(n[2], n[0]),
                r(n[4], n[3]),
                r(n[5], n[3]),
                r(n[1] + n[4], n[0] + n[3]),
                r(n[2] + n[5], n[0] + n[3])
            );
            for (s, th) in sens.iter().zip(SHARES) {
                println!(
                    "gate pdf {name}: image share >= {th}: recall {:.3}, passed table-free {:.3}, passed all {:.3}",
                    r(s[0], n[0]),
                    r(s[1], n[3]),
                    r(s[0] + s[1], n[0] + n[3])
                );
            }
            img_ms.sort_by(f64::total_cmp);
            let m = img_ms.len().max(1);
            println!(
                "gate pdf {name}: {unreadable} labelled pages not readable; image_boxes {} pages, mean {:.4} ms (only where the cheap rules say no: {:.4} ms per page), median {:.4}, p99 {:.4}, max {:.3}",
                img_ms.len(),
                img_ms.iter().sum::<f64>() / m as f64,
                lazy_ms / m as f64,
                img_ms.get(img_ms.len() / 2).copied().unwrap_or(0.0),
                img_ms.get(img_ms.len() * 99 / 100).copied().unwrap_or(0.0),
                img_ms.last().copied().unwrap_or(0.0)
            );
        }
    }

    #[test]
    fn flipped_image_box_is_normalised() {
        let want = Rect::new(90.0, 692.0, 50.0, 20.0);
        assert_eq!(image_rect([100.0, 80.0, 50.0, 20.0], 10.0, 792.0), want);
        assert_eq!(image_rect([150.0, 100.0, -50.0, -20.0], 10.0, 792.0), want);
    }

    #[test]
    fn ruled_table_page_has_words_and_rulings_in_top_left_origin() {
        let doc = PdfDocument::from_bytes(crate::pdf::tests::ruled_table_pdf()).unwrap();
        let p = page_input(&doc, 0).unwrap();
        assert_eq!((p.width, p.height), (612.0, 792.0));
        let name = p.words.iter().find(|w| w.text == "Name").unwrap();
        assert!(
            (name.rect.top - (792.0 - 626.0 - name.rect.h)).abs() < 3.0,
            "{:?}",
            name.rect
        );
        assert_eq!(name.char_x.len(), 4);
        assert!(
            p.paths.len() + p.segments.len() >= 9,
            "5 horizontal + 4 vertical rulings"
        );
    }

    /// Adapter vs spike dumps: `TABLES_PDFS=<pdf dir> TABLES_GEOM=<dump dir>`; same tables on ≥99% of
    /// pages (the dumps round to 0.01 pt; measured: ODL 1/200 pages differ).
    #[test]
    #[ignore = "needs TABLES_PDFS and TABLES_GEOM"]
    fn adapter_matches_spike_dumps() {
        let (Ok(pdfs), Ok(geom)) = (std::env::var("TABLES_PDFS"), std::env::var("TABLES_GEOM"))
        else {
            eprintln!("skip: set TABLES_PDFS and TABLES_GEOM (docs/tables-eval.md)");
            return;
        };
        let cfg = TableConfig::reference();
        let (mut pages, mut diff) = (0usize, Vec::new());
        let mut files: Vec<_> = std::fs::read_dir(&pdfs)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        for f in files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == "pdf"))
        {
            let stem = f.file_stem().unwrap().to_string_lossy().to_string();
            let Ok(dump) = std::fs::read_to_string(format!("{geom}/{stem}.json")) else {
                continue;
            };
            let dump: serde_json::Value = serde_json::from_str(&dump).unwrap();
            let doc = PdfDocument::open(f).unwrap();
            // The dumps were made with the engine default (refinement on), like SP4 production.
            for (i, dp) in dump["pages"].as_array().unwrap().iter().enumerate() {
                pages += 1;
                let rows = |p: &PageInput| -> Vec<_> {
                    detect_tables(p, &cfg)
                        .tables
                        .into_iter()
                        .map(|t| {
                            let cells = t.rows.iter().map(|r| {
                                r.iter()
                                    .map(|c| (c.flat_text(), c.colspan, c.rowspan))
                                    .collect::<Vec<_>>()
                            });
                            cells.collect::<Vec<_>>()
                        })
                        .collect()
                };
                if rows(&page_input_with(&doc, i, true).unwrap()) != rows(&page_from_dump(dp)) {
                    diff.push(format!("{stem}:{i}"));
                }
            }
        }
        println!(
            "adapter parity: {pages} pages, {} differ: {diff:?}",
            diff.len()
        );
        assert!(diff.len() * 100 <= pages);
    }
    #[test]
    fn char_x_is_page_local_on_an_offset_media_box() {
        let pdf = crate::pdf::tests::raw_pdf(
            "10000 10000 10612 10792",
            "BT /F1 10 Tf 10100 10700 Td (Name) Tj ET\n",
        );
        let doc = PdfDocument::from_bytes(pdf).unwrap();
        let p = page_input(&doc, 0).unwrap();
        let w = p.words.iter().find(|w| w.text == "Name").unwrap();
        assert_eq!(w.char_x.len(), 4);
        let (a, b) = (w.char_x[0].0, w.char_x[3].1);
        assert!(
            (a - w.rect.x).abs() < 1.0 && (b - (w.rect.x + w.rect.w)).abs() < 1.0,
            "{:?} vs {:?}",
            w.char_x,
            w.rect
        );
    }

    #[test]
    fn stroked_composite_path_over_the_limit_is_not_expanded() {
        // One stroked path of rectangles: 4 segments each, over the cap before any is made.
        let mut s = String::from("0.5 w\n");
        for k in 0..ADAPTER_MAX_PRIMS / 4 + 1 {
            // Within 1 000 pt (a taller path is no table primitive).
            s += &format!("72 {} 400 0.01 re\n", 20.0 + k as f64 * 0.0075);
        }
        s += "S\n";
        let doc = PdfDocument::from_bytes(crate::pdf::tests::raw_pdf("0 0 612 792", &s)).unwrap();
        let e = page_input(&doc, 0).unwrap_err();
        assert!(
            matches!(&e, ParseError::Limit { kind, limit }
                if kind.contains("paths and segments") && *limit == ADAPTER_MAX_PRIMS as u64),
            "{e}"
        );
    }

    #[test]
    fn words_over_the_cap_are_not_converted() {
        // 640 lines of 250 one-point words: twice the cap drawn (pdf_oxide drops some of such text).
        let line = "w ".repeat(250);
        let s: String = (0..2 * ADAPTER_MAX_WORDS / 250)
            .map(|k| {
                format!(
                    "BT /F1 1 Tf 10 {} Td ({line}) Tj ET\n",
                    780.0 - 1.2 * k as f64
                )
            })
            .collect();
        let doc = PdfDocument::from_bytes(crate::pdf::tests::raw_pdf("0 0 612 792", &s)).unwrap();
        let e = page_input(&doc, 0).unwrap_err();
        assert!(
            matches!(&e, ParseError::Limit { kind, limit }
                if kind.contains("words on the page") && *limit == ADAPTER_MAX_WORDS as u64),
            "{e}"
        );
    }

    #[test]
    fn curve_moves_the_current_point_of_a_composite_path() {
        // Rounded corner, then the right side: the side starts where the curve ends.
        let pdf = crate::pdf::tests::raw_pdf(
            "0 0 612 792",
            "0.5 w 72 640 m 452 640 l 454 640 456 638 456 636 c 456 560 l 72 600 m 456 600 l S\n",
        );
        let doc = PdfDocument::from_bytes(pdf).unwrap();
        let p = page_input(&doc, 0).unwrap();
        let side = InputSegment {
            x1: 456.0,
            y1: 792.0 - 636.0,
            x2: 456.0,
            y2: 792.0 - 560.0,
            raster: false,
        };
        assert!(p.segments.contains(&side), "{:?}", p.segments);
    }

    #[test]
    fn rotated_glyphs_of_one_run_become_one_word() {
        use pdf_oxide::geometry::Rect as R;
        let glyph = |t: &str, y: f32, seq: usize| pdf_oxide::layout::Word {
            chars: Vec::new(),
            bbox: R::new(300.0, y, 5.0, 10.0),
            text: t.into(),
            avg_font_size: 10.0,
            dominant_font: String::new(),
            is_bold: false,
            is_italic: false,
            mcid: None,
            sequence: seq,
            rotation_degrees: -90.0,
            char_visibility: Vec::new(),
        };
        // Content order 2,0,2,5 goes upwards (vertical header read bottom-to-top).
        let w = merge_rotated_runs(vec![
            glyph("5", 615.0, 4),
            glyph("2", 610.0, 3),
            glyph("0", 605.0, 2),
            glyph("2", 600.0, 1),
        ]);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].text, "2025");
        assert_eq!((w[0].bbox.y, w[0].bbox.height), (600.0, 25.0));
        let apart = merge_rotated_runs(vec![glyph("1", 600.0, 1), glyph("2", 400.0, 2)]);
        assert_eq!(apart.len(), 2, "not adjacent");
    }
}
