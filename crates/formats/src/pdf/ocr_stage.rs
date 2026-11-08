//! OCR of PDF pages on the PDF_POOL thread that owns the document: render, cascade, merge, plus
//! the 50-dpi need-OCR raster signals.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use pdf_oxide::rendering::{ImageFormat, PageRenderer, RenderOptions};
use pdf_oxide::{PdfDocument, StructuredPage};
use plaindoc_core::{
    BBox, Block, HiddenText, Level, ParseOptions, Stage, Warning, WarningKind, telemetry,
};
use plaindoc_ocr::lines::LinesParams;
use plaindoc_ocr::{Image, OcrError, OcrPage, ocr_image_cancellable};

use super::diagnose::PageDecision;
use super::hidden_ocr::{Unread, check_layer};
use super::need_ocr::{InkFeatures, ink_features};
use super::table_place::holds;
use super::tables::{ScanSegment, ScanTableInputs, ScanWord, TableDetector};
use crate::ocr::{OcrConfig, PageGeom, ocr_blocks, ocr_blocks_aligned, ocr_warnings, rgba_to_rgb};

/// RGB raster of a page plus its geometry.
pub(crate) struct Rendered {
    pub img: Image,
    pub geom: PageGeom,
}

pub(crate) fn render(doc: &PdfDocument, p: usize, dpi: u32) -> Result<Rendered, String> {
    let info = doc.get_page_info(p).map_err(|e| e.to_string())?;
    let mut opts = RenderOptions::with_dpi(dpi);
    opts.format = ImageFormat::RawRgba8;
    let r = PageRenderer::new(opts)
        .render_page(doc, p)
        .map_err(|e| e.to_string())?;
    let m = info.media_box;
    let mut media = [m.x, m.y, m.x + m.width, m.y + m.height];
    // The renderer draws CropBox ∩ MediaBox (MediaBox if they are disjoint), as pdf_oxide's
    // `page_render_box`.
    if let Some(c) = info.crop_box {
        let r = [
            c.x.max(media[0]),
            c.y.max(media[1]),
            (c.x + c.width).min(media[2]),
            (c.y + c.height).min(media[3]),
        ];
        if r[2] > r[0] && r[3] > r[1] {
            media = r;
        }
    }
    let mut geom = PageGeom {
        media,
        rotation: info.rotation.rem_euclid(360) as u32,
        scale: 1.0,
    };
    // Actual scale: the renderer may lower it to fit its pixel budget.
    geom.scale = r.width as f32 / geom.rotated_extent().0.max(1.0);
    let img =
        Image::from_raw(r.width, r.height, rgba_to_rgb(&r.data)).ok_or("raster size mismatch")?;
    Ok(Rendered { img, geom })
}

/// Text-layer span boxes in raster pixels of `geom`.
pub(crate) fn span_boxes_px(
    doc: &PdfDocument,
    p: usize,
    geom: &PageGeom,
) -> Result<Vec<[f32; 4]>, String> {
    layer_boxes(doc.extract_spans(p), geom)
}

/// A failed extraction stays an error: as an empty layer it would disable merge dedup.
fn layer_boxes(
    spans: pdf_oxide::Result<Vec<pdf_oxide::layout::TextSpan>>,
    geom: &PageGeom,
) -> Result<Vec<[f32; 4]>, String> {
    Ok(spans
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| geom.px_rect(s.bbox.x, s.bbox.y, s.bbox.width, s.bbox.height))
        .collect())
}

/// 50-dpi raster features; `None` if the page cannot be rendered.
pub(crate) fn ink_signals(doc: &PdfDocument, p: usize) -> Option<InkFeatures> {
    let r = render(doc, p, 50).ok()?;
    let boxes = span_boxes_px(doc, p, &r.geom).unwrap_or_default();
    let gray = image::imageops::grayscale(&r.img);
    let (w, h) = gray.dimensions();
    // Boxes are already pixels: pass them as points at 72 dpi (identity scale).
    Some(ink_features(
        gray.as_raw(),
        w as usize,
        h as usize,
        &boxes,
        (w as f32, h as f32),
        72.0,
    ))
}

/// Result of OCR on one page.
pub(crate) enum PageOcr {
    /// `replace`: blocks replace the text layer (full-page OCR of a lost page).
    Done {
        blocks: Vec<Block>,
        warnings: Vec<Warning>,
        quality: f32,
        replace: bool,
        /// Full-page OCR on the merge path with hidden-text rules on whose words do not read the
        /// text layer (`hidden::layer_mismatch`); `None` otherwise and when OCR stopped early.
        mismatch: Option<LayerMismatch>,
    },
    /// Engine missing or models unavailable: counted into the document `OcrUnavailable`.
    Unavailable(String),
    Failed(String),
}

/// A text layer that is not what the full-page OCR reads: the counts of the check and the OCR's
/// blocks without the layer-duplicate filter (the page's text in `Strip`).
pub(crate) struct LayerMismatch {
    pub check: Unread,
    pub blocks: Vec<Block>,
}

/// Whether the centre of `b` lies inside one of `boxes` (all in raster pixels).
pub(crate) fn center_inside(b: &[f32; 4], boxes: &[[f32; 4]]) -> bool {
    let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    boxes
        .iter()
        .any(|t| t[0] <= cx && cx <= t[2] && t[1] <= cy && cy <= t[3])
}

/// Counters of one page's OCR for `ParseStats`: `run_page_ocr` sets the raster and the cut-offs,
/// the OCR pass sets `took`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OcrPageCounts {
    pub took: std::time::Duration,
    pub raster_px: u64,
    pub timed_out: bool,
    pub boxes_dropped: usize,
}

/// Runs the cascade for one page. `RunOcr`/`Reject`: whole page, blocks replace the layer.
/// `RunOcrAndMerge`: OCR of the page's image regions (whole page when it has no usable image
/// boxes, e.g. outlined vector text, or past the region caps). `UseTextLayer` (`Always`): whole
/// page. Without `replace`, words whose centre lies in a text-layer span are dropped as duplicates.
/// `opts.ocr_page_timeout` bounds recognition of the page (all its regions together). `layer`: the
/// page's text layer after the hidden-text filter, checked against a full-page merge OCR.
/// `replace` pages also get raster rules and `tables.detect_scan`; their tables are placed among the
/// OCR paragraphs.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_page_ocr(
    doc: &PdfDocument,
    p: usize,
    decision: PageDecision,
    layer: Option<&StructuredPage>,
    cfg: &OcrConfig,
    tables: &dyn TableDetector,
    opts: &ParseOptions,
    cancel: &AtomicBool,
    counts: &mut OcrPageCounts,
) -> PageOcr {
    let Some(engine) = cfg.engine.as_deref() else {
        return PageOcr::Unavailable("no OCR engine in this build".into());
    };
    let r = match render(doc, p, cfg.page_dpi) {
        Ok(r) => r,
        Err(e) => return PageOcr::Failed(format!("page render failed: {e}")),
    };
    let replace = matches!(decision, PageDecision::RunOcr | PageDecision::Reject);
    let spans = if replace {
        Vec::new()
    } else {
        match span_boxes_px(doc, p, &r.geom) {
            Ok(s) => s,
            // Without the layer boxes merged OCR would repeat the layer's text: skip it.
            Err(e) => {
                return PageOcr::Failed(format!("text layer unreadable, OCR not merged: {e}"));
            }
        }
    };
    // `keep` gets page-pixel boxes (after the crop offset), the system of `spans`.
    let keep = |b: &[f32; 4]| !center_inside(b, &spans);
    let (w, h) = r.img.dimensions();
    counts.raster_px = u64::from(w) * u64::from(h);
    let mut regions: Vec<[u32; 4]> = Vec::new();
    if decision == PageDecision::RunOcrAndMerge {
        for im in doc.extract_images(p).unwrap_or_default() {
            if let Some(b) = im.bbox() {
                let px = r.geom.px_rect(b.x, b.y, b.width, b.height);
                let x0 = px[0].max(0.0) as u32;
                let y0 = px[1].max(0.0) as u32;
                let x1 = px[2].min(w as f32) as u32;
                let y1 = px[3].min(h as f32) as u32;
                // Off-raster boxes clip to an inverted rect; never keep one (`min_region_px` may be 0).
                if x1 > x0 && y1 > y0 && (x1 - x0).min(y1 - y0) >= cfg.min_region_px {
                    regions.push([x0, y0, x1, y1]);
                }
            }
        }
    }
    let regions = merge_regions(regions, (w, h));
    let mut params = cfg.params;
    if replace {
        // Full-page OCR (no image regions on `replace` pages): rules for the scan table detector, at the
        // actual raster resolution (`render` may lower it below `page_dpi`).
        let mut l = LinesParams::default();
        l.dpi = r.geom.scale * 72.0;
        params.table_lines = Some(l);
    }
    if let Some(t) = opts.ocr_page_timeout {
        params.deadline = Instant::now().checked_add(t);
    }
    let run = |img: &Image| ocr_image_cancellable(engine, img, &params, cancel);
    let mut pages: Vec<(OcrPage, (f32, f32))> = Vec::new();
    let scope = if regions.is_empty() {
        match run(&r.img) {
            Ok(o) => pages.push((o, (0.0, 0.0))),
            Err(e) => return failed(e),
        }
        "full page".to_string()
    } else {
        for g in &regions {
            let crop =
                image::imageops::crop_imm(&r.img, g[0], g[1], g[2] - g[0], g[3] - g[1]).to_image();
            match run(&crop) {
                Ok(o) => pages.push((o, (g[0] as f32, g[1] as f32))),
                Err(e) => return failed(e),
            }
        }
        format!("{} image region(s)", regions.len())
    };
    let refs: Vec<&OcrPage> = pages.iter().map(|(o, _)| o).collect();
    let (mut warnings, quality) = ocr_warnings(&refs, &scope, cfg.low_confidence);
    let dropped: usize = refs.iter().map(|o| o.boxes_dropped).sum();
    counts.boxes_dropped = dropped;
    if dropped > 0 {
        warnings.push(Warning::new(
            WarningKind::PartialContent,
            Level::Warn,
            format!(
                "OCR stopped after {} text boxes ({dropped} more)",
                cfg.params.max_boxes
            ),
        ));
    }
    if let Some(t) = opts.ocr_page_timeout
        && refs.iter().any(|o| o.timed_out)
    {
        counts.timed_out = true;
        let k: usize = refs.iter().map(|o| o.lines.len()).sum();
        warnings.push(Warning::new(
            WarningKind::PartialContent,
            Level::Warn,
            format!("OCR timed out after {t:?}: {k} line(s) recognised"),
        ));
    }
    let complete = dropped == 0 && !refs.iter().any(|o| o.timed_out);
    let mismatch = layer
        .filter(|_| !replace && regions.is_empty() && complete)
        .filter(|_| opts.hidden_text != HiddenText::Off)
        .and_then(|page| check_layer(page, &r.geom, &pages))
        .map(|check| LayerMismatch {
            check,
            blocks: pages
                .iter()
                .flat_map(|(o, off)| ocr_blocks(o, &r.geom, *off, &|_| true))
                .collect(),
        });
    // After `ocr_warnings`: words that go into tables still count as OCR text. An `OcrEmpty` page keeps
    // its stage-1 blocks (`apply_ocr`), so it gets no tables either.
    let found = if replace
        && !warnings.iter().any(|w| w.kind == WarningKind::OcrEmpty)
        && let [(o, _)] = pages.as_mut_slice()
        && !o.words.is_empty()
    {
        scan_tables(o, p, &r.geom, tables, &mut warnings)
    } else {
        Vec::new()
    };
    let mut blocks: Vec<(Block, [f32; 4])> = pages
        .iter()
        .flat_map(|(o, off)| ocr_blocks_aligned(o, &r.geom, *off, &keep))
        .collect();
    place_tables_aligned(&mut blocks, found, r.geom.scale);
    let blocks = blocks.into_iter().map(|(b, _)| b).collect();
    PageOcr::Done {
        blocks,
        warnings,
        quality,
        replace,
        mismatch,
    }
}

/// Tables of a full-page OCR: `detect_scan` on the aligned page, then the OCR words whose centre lies in
/// a table's `text_bbox` (aligned frame) are removed from `page` (they are the table's text) and the
/// table blocks are mapped to PDF user space. Each block comes with its table's rect in px of the aligned
/// raster, for placement in reading order. Table warnings, or the detector error, go to `warnings`.
/// The words are our own OCR of the rendered page, so the hidden-text filter does not apply to them
/// (OCR reads only what is drawn), unlike the text-layer words of `TableDetector::detect_filtered`.
fn scan_tables(
    page: &mut OcrPage,
    p: usize,
    geom: &PageGeom,
    detector: &dyn TableDetector,
    warnings: &mut Vec<Warning>,
) -> Vec<(Block, [f32; 4])> {
    let inputs = scan_inputs(page, p, geom.scale);
    let found = match telemetry::stage(Stage::Tables, || detector.detect_scan(&inputs)) {
        Ok(t) => t,
        Err(e) => {
            warnings.push(Warning::new(
                WarningKind::TableUncertain,
                Level::Warn,
                format!("table detection failed: {e}"),
            ));
            return Vec::new();
        }
    };
    if found.is_empty() {
        return Vec::new();
    }
    let (s, h) = (geom.scale, page.height as f32 / geom.scale);
    page.words.retain(|w| {
        let (x, y) = (
            (w.bbox[0] + w.bbox[2]) / 2.0 / s,
            h - (w.bbox[1] + w.bbox[3]) / 2.0 / s,
        );
        !found.iter().any(|t| holds(&t.text_bbox, x, y))
    });
    let to_user = |b: &BBox| table_bbox_to_user(b, page, geom);
    found
        .into_iter()
        .map(|mut t| {
            let aligned = aligned_px(&t.bbox, page, geom.scale);
            remap_table_bboxes(&mut t.block, &to_user);
            warnings.append(&mut t.warnings);
            (t.block, aligned)
        })
        .collect()
}

/// Inserts each table before the first block that starts at or below the table top (within 1 pt) and
/// overlaps it horizontally, as SP4 `place_tables` but in the aligned raster (px, y down), the frame of
/// the OCR reading order: in user space a turned or `/Rotate 90/270` page has its lines along x.
fn place_tables_aligned(
    blocks: &mut Vec<(Block, [f32; 4])>,
    tables: Vec<(Block, [f32; 4])>,
    px_per_pt: f32,
) {
    for (t, a) in tables {
        let at = blocks
            .iter()
            .position(|(_, b)| b[1] + px_per_pt >= a[1] && b[0] < a[2] && a[0] < b[2]);
        blocks.insert(at.unwrap_or(blocks.len()), (t, a));
    }
}

/// Aligned pt (y up) -> px rect `[x0, y0, x1, y1]` of the aligned raster of `page`.
fn aligned_px(b: &BBox, page: &OcrPage, s: f32) -> [f32; 4] {
    let h = page.height as f32;
    [b.x * s, h - (b.y + b.h) * s, (b.x + b.w) * s, h - b.y * s]
}

/// Table-detector input of an OCRed page: px of the aligned raster -> pt (`x / scale`,
/// `page_height - y / scale`), y up.
pub(crate) fn scan_inputs(page: &OcrPage, page_index: usize, scale: f32) -> ScanTableInputs {
    let h = page.height as f32 / scale;
    let words = page
        .words
        .iter()
        .map(|w| {
            let [x0, y0, x1, y1] = w.bbox;
            ScanWord {
                text: w.text.clone(),
                bbox: BBox::new(
                    x0 / scale,
                    h - y1 / scale,
                    (x1 - x0) / scale,
                    (y1 - y0) / scale,
                ),
                char_x: w
                    .char_x
                    .iter()
                    .map(|&(a, b)| (a / scale, b / scale))
                    .collect(),
                conf: w.conf,
                font_size: w.font_px / scale,
                line: w.line,
                block: w.block,
                angle: w.angle,
            }
        })
        .collect();
    let segments = page
        .segments
        .iter()
        .map(|g| {
            let (a, b, at) = (g.a / scale, g.b / scale, g.at / scale);
            if g.horizontal {
                ScanSegment {
                    x1: a,
                    y1: h - at,
                    x2: b,
                    y2: h - at,
                }
            } else {
                ScanSegment {
                    x1: at,
                    y1: h - a,
                    x2: at,
                    y2: h - b,
                }
            }
        })
        .collect();
    ScanTableInputs {
        page_index,
        page_width: page.width as f32 / scale,
        page_height: h,
        words,
        segments,
        skew_deg: page.skew_deg,
        rotation: page.rotation,
        model_regions: Vec::new(),
    }
}

/// Applies `f` to the bbox of a `Table` block and of every table nested in its cells.
pub(crate) fn remap_table_bboxes(block: &mut Block, f: &dyn Fn(&BBox) -> BBox) {
    if let Block::Table { rows, bbox, .. } = block {
        if let Some(b) = bbox {
            *b = f(b);
        }
        for b in rows.iter_mut().flatten().flat_map(|c| &mut c.blocks) {
            remap_table_bboxes(b, f);
        }
    }
}

/// Bbox of a table in the aligned frame of `page` (pt, y up) -> PDF user space: aligned px, then
/// `page.frame.to_source` (4 corners), then `geom` (`/Rotate`, MediaBox offset).
pub(crate) fn table_bbox_to_user(b: &BBox, page: &OcrPage, geom: &PageGeom) -> BBox {
    geom.bbox(page.frame.to_source(aligned_px(b, page, geom.scale)))
}

/// More regions than this, or regions covering more than `MAX_REGION_SHARE` of the page, are OCRed
/// as one full page (bounded engine calls on pages with hundreds of images).
pub(crate) const MAX_REGIONS: usize = 16;
pub(crate) const MAX_REGION_SHARE: f64 = 0.6;

/// Unions overlapping or touching (within 8 px) image regions so overlapping images are OCRed once
/// (no duplicate words between regions); returns no regions (= full page) past the caps.
pub(crate) fn merge_regions(mut rs: Vec<[u32; 4]>, (w, h): (u32, u32)) -> Vec<[u32; 4]> {
    let near = |a: &[u32; 4], b: &[u32; 4]| {
        a[0] <= b[2] + 8 && b[0] <= a[2] + 8 && a[1] <= b[3] + 8 && b[1] <= a[3] + 8
    };
    let mut merged = true;
    while merged {
        merged = false;
        'outer: for i in 0..rs.len() {
            for j in i + 1..rs.len() {
                if near(&rs[i], &rs[j]) {
                    let b = rs.swap_remove(j);
                    let a = &mut rs[i];
                    *a = [
                        a[0].min(b[0]),
                        a[1].min(b[1]),
                        a[2].max(b[2]),
                        a[3].max(b[3]),
                    ];
                    merged = true;
                    break 'outer;
                }
            }
        }
    }
    rs.sort_unstable_by_key(|r| (r[1], r[0]));
    let area: f64 = rs
        .iter()
        .map(|r| f64::from(r[2] - r[0]) * f64::from(r[3] - r[1]))
        .sum();
    if rs.len() > MAX_REGIONS || area > MAX_REGION_SHARE * f64::from(w) * f64::from(h) {
        return Vec::new();
    }
    rs
}

fn failed(e: OcrError) -> PageOcr {
    match e {
        OcrError::ModelsUnavailable(m) => PageOcr::Unavailable(m),
        e => PageOcr::Failed(e.to_string()),
    }
}

/// Warning for a page whose OCR failed (the stage-1 loss stays).
pub(crate) fn ocr_failed_warning(e: &str) -> Warning {
    Warning::new(
        WarningKind::PartialContent,
        Level::Warn,
        format!("OCR failed: {e}"),
    )
}

#[cfg(test)]
#[path = "ocr_stage_test.rs"]
pub(crate) mod tests;

/// SP5 Task 22.3: tables on scans through the real path (image -> OCR with raster rules -> `scan_inputs`
/// -> `OwnTables::detect_scan`), scored as `scan_metrics` (docs/tables-eval.md):
/// `SCAN_EVAL=<dir with img/deg/meta.jsonl> TABLES_GT=<gt.json> [SCAN_EVAL_VARIANTS=c150,m150]
/// [SCAN_EVAL_CFGS=default;pre_t19;off:<flag>] [TABLES_PAGES=...] [SCAN_EVAL_THREADS=4] [SCAN_EVAL_LIST=1]
/// PLAINDOC_OCR_MODELS=<models>`. OCR runs once per image; every config is scored on the same pages.
#[cfg(all(test, feature = "ocr-paddle", feature = "tables-eval"))]
#[path = "ocr_stage_scan_eval.rs"]
mod scan_eval;
