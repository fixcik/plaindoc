//! OCR quality: CER/WER over normalised text and regression gates (task 23).
//!
//! The model gates need the PaddleOCR models (bundled or `PLAINDOC_OCR_MODELS`); the synthetic sets live
//! outside the repository and are passed by path (`OCR_EVAL_SET`, `LINES_EVAL_SET`):
//!
//! ```sh
//! OCR_EVAL_SET=… LINES_EVAL_SET=… cargo test --release -p plaindoc-formats --features ocr-paddle \
//!     --test ocr_eval -- --ignored --nocapture
//! ```

mod pdf_common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use plaindoc_core::{Format, Input, ParseOptions, Parser};
use plaindoc_formats::Registry;
use plaindoc_formats::ocr::{ImageParser, OcrConfig, paddle_engine};
use plaindoc_ocr::lines::{LinesParams, extract_lines};
use plaindoc_ocr::raster::{self, normalize_bg};
use plaindoc_ocr::{OcrEngine, OcrParams, ocr_image};
use rayon::prelude::*;
use serde_json::Value;

/// Lowercase, ё -> е, whitespace runs -> one space, trimmed.
fn norm(s: &str) -> Vec<char> {
    let s: String = s.to_lowercase().replace('ё', "е");
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect()
}

fn levenshtein<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Character error rate: Levenshtein(gt, ocr) / len(gt) over normalised text.
fn cer(gt: &str, ocr: &str) -> f64 {
    let (a, b) = (norm(gt), norm(ocr));
    levenshtein(&a, &b) as f64 / a.len().max(1) as f64
}

/// Word error rate: word-level Levenshtein / gt words over normalised text.
fn wer(gt: &str, ocr: &str) -> f64 {
    let words = |s: &str| -> Vec<String> {
        String::from_iter(norm(s))
            .split(' ')
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let (a, b) = (words(gt), words(ocr));
    levenshtein(&a, &b) as f64 / a.len().max(1) as f64
}

/// `[length of a covered by collinear b, length of a]`: for every segment `[x1, y1, x2, y2]` of `a`, the union
/// of the parts covered by `b` segments of the same orientation whose offset differs by at most `tol`.
fn covered_length(a: &[[f32; 4]], b: &[[f32; 4]], tol: f32) -> [f64; 2] {
    // (horizontal, lo, hi, at)
    let key = |s: &[f32; 4]| {
        let h = (s[3] - s[1]).abs() <= (s[2] - s[0]).abs();
        let (p, q, at) = if h {
            (s[0], s[2], (s[1] + s[3]) / 2.0)
        } else {
            (s[1], s[3], (s[0] + s[2]) / 2.0)
        };
        (h, p.min(q), p.max(q), at)
    };
    let bk: Vec<_> = b.iter().map(key).collect();
    let (mut covered, mut total) = (0f64, 0f64);
    for s in a {
        let (h, lo, hi, at) = key(s);
        total += f64::from(hi - lo);
        let mut iv: Vec<(f32, f32)> = bk
            .iter()
            .filter(|k| k.0 == h && (k.3 - at).abs() <= tol)
            .map(|k| (k.1.max(lo), k.2.min(hi)))
            .filter(|(p, q)| q > p)
            .collect();
        iv.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut cur = lo;
        for (p, q) in iv {
            if q > cur {
                covered += f64::from(q - p.max(cur));
                cur = q;
            }
        }
    }
    [covered, total]
}

fn engine() -> Arc<dyn OcrEngine> {
    static E: OnceLock<Arc<dyn OcrEngine>> = OnceLock::new();
    E.get_or_init(|| paddle_engine(None, None)).clone()
}

/// Heading, paragraph and note texts of a `fixtures/pdf` file parsed through the default registry.
fn parse_text(fixture: &str) -> String {
    let bytes = std::fs::read(pdf_common::fixture(fixture)).unwrap();
    let d = Registry::with_defaults()
        .parse(Input::Bytes(bytes), &ParseOptions::default())
        .unwrap();
    pdf_common::doc_text(&d)
}

/// The same through the image parser with the build's default engine.
fn parse_text_bytes(bytes: Vec<u8>, f: Format) -> String {
    let d = ImageParser::new(OcrConfig::default_engine())
        .parse(Input::Bytes(bytes), f, &ParseOptions::default())
        .unwrap();
    pdf_common::doc_text(&d)
}

fn render_png(pdf: Vec<u8>, page: usize, dpi: u32) -> Vec<u8> {
    use pdf_oxide::PdfDocument;
    use pdf_oxide::rendering::{PageRenderer, RenderOptions};
    let doc = PdfDocument::from_bytes(pdf).unwrap();
    PageRenderer::new(RenderOptions::with_dpi(dpi))
        .render_page(&doc, page)
        .unwrap()
        .data
}

/// `img` turned clockwise by `turn` degrees (0, 90, 180 or 270).
fn turned(img: &image::RgbImage, turn: u32) -> image::RgbImage {
    match turn {
        90 => image::imageops::rotate90(img),
        180 => image::imageops::rotate180(img),
        270 => image::imageops::rotate270(img),
        _ => img.clone(),
    }
}

fn set_dir(var: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| {
        panic!("set {var} to the evaluation set directory (see the module docs)")
    })
}

fn meta_lines(dir: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("meta.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn pool() -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
}

fn f32s(v: &Value) -> Vec<f32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap() as f32)
        .collect()
}

#[test]
fn cer_is_levenshtein_over_normalised_text() {
    assert_eq!(cer("Ёлка  стоит", "елка стоит"), 0.0);
    assert!((cer("abcd", "abxd") - 0.25).abs() < 1e-12);
    assert!((cer("abc", "") - 1.0).abs() < 1e-12);
}

#[test]
fn wer_counts_word_edits() {
    assert_eq!(wer("Ёлка  стоит", "елка стоит"), 0.0);
    assert!((wer("a b c d", "a x c") - 0.5).abs() < 1e-12);
    assert!((wer("a b", "") - 1.0).abs() < 1e-12);
}

#[test]
fn covered_length_unions_collinear_cover() {
    let a = [[0.0, 10.0, 100.0, 10.0], [50.0, 0.0, 50.0, 40.0]];
    let b = [
        [10.0, 11.0, 40.0, 11.0], // collinear within tol
        [30.0, 9.5, 60.0, 9.5],   // overlaps the first: union 10..60
        [70.0, 13.0, 90.0, 13.0], // offset 3 > tol
        [60.0, 10.0, 60.0, 30.0], // vertical, other orientation for a[0]; offset 10 for a[1]
        [51.0, 20.0, 51.0, 50.0], // covers a[1] 20..40
    ];
    let [c, t] = covered_length(&a, &b, 1.5);
    assert!((t - 140.0).abs() < 1e-9, "{t}");
    assert!((c - 70.0).abs() < 1e-9, "{c}");
    assert_eq!(covered_length(&[], &b, 1.5), [0.0, 0.0]);
}

#[test]
#[ignore = "needs OCR models"]
fn in_repo_scan_cer_gate() {
    let gt = "Договор поставки №42 Стороны договорились о следующем: поставщик передаёт товар в срок до 1 марта.";
    let text = parse_text("scan.pdf");
    let c = cer(gt, &text);
    eprintln!("scan.pdf: CER {c:.4} WER {:.4}\n{text}", wer(gt, &text));
    assert!(c <= 0.02, "CER {c:.4}: {text}"); // measured 0.0
}

#[test]
#[ignore = "needs OCR models"]
fn builder_page_english_cer_gate() {
    // DocumentBuilder Helvetica page: its text layer is the ground truth; the 200-dpi render of the same page
    // goes through the ImageParser (no merge/dedup in the way) and its OCR text is compared with the layer.
    let pdf = pdf_common::builder_pdf_bytes(1);
    let layer = pdf_common::doc_text(
        &pdf_common::parse_bytes(pdf.clone(), &ParseOptions::default()).unwrap(),
    );
    let ocr = parse_text_bytes(render_png(pdf, 0, 200), Format::Png);
    let c = cer(&layer, &ocr);
    eprintln!("builder page: CER {c:.4} WER {:.4}", wer(&layer, &ocr));
    assert!(c <= 0.03, "CER {c:.4}\nlayer: {layer:?}\nocr: {ocr}");
}

/// Per page of the scan set: `(variant, Σ char errors, gt chars, Σ word errors, gt words, mean conf,
/// residual skew = angle + skew_deg, rotation)`.
type ScanRow = (String, f64, f64, f64, f64, f32, f32, u32);

#[test]
#[ignore = "needs OCR_EVAL_SET (synthetic scan set outside the repo)"]
fn synthetic_scan_set_regression() {
    // OCR_EVAL_SET/meta.jsonl: {"img", "variant": c150|c200|s150|s200, "angle",
    //   "gt_lines": [{"text", "bbox_px": [x0,y0,x1,y1]}]}  (bbox in the level frame, angle = degradation tilt)
    // Gates: plan 0.04 / 0.045 / 0.06 / 0.10 (spike + ~1 pp; cyrillic rec, det 960), tightened by the first Rust
    // run to measured + 0.5 pp (bicubic deskew). Measured (51 pages, line-level CER / WER): c200 0.0162 / 0.070,
    // c150 0.0156 / 0.064, s200 0.0272 / 0.143, s150 0.0667 / 0.240.
    const GATES: [(&str, f64); 4] = [
        ("c200", 0.022),
        ("c150", 0.021),
        ("s200", 0.033),
        ("s150", 0.072),
    ];
    let dir = set_dir("OCR_EVAL_SET");
    // OCR_EVAL_TURN=90|180|270: every page is turned clockwise before OCR; the orientation check must undo it,
    // so the level-frame gt boxes stay valid and the CER stays within 0.01 of the upright run.
    let turn: u32 = std::env::var("OCR_EVAL_TURN").map_or(0, |v| v.parse().unwrap());
    let rows: Vec<ScanRow> = pool().install(|| {
        meta_lines(&dir)
            .par_iter()
            .map(|m| {
                let bytes = std::fs::read(dir.join(m["img"].as_str().unwrap())).unwrap();
                let img = image::load_from_memory(&bytes).unwrap().to_rgb8();
                let img = turned(&img, turn);
                let t0 = std::time::Instant::now();
                let page = ocr_image(&*engine(), &img, &OcrParams::default())
                    .expect("OCR models must load");
                let ms = t0.elapsed().as_secs_f64() * 1e3;
                if let Some(d) = std::env::var_os("OCR_EVAL_DUMP") {
                    let stem = m["stem"].as_str().unwrap_or("x");
                    let v = m["variant"].as_str().unwrap();
                    std::fs::write(
                        PathBuf::from(d).join(format!("{v}_{stem}_{turn}.txt")),
                        format!("{page:?}\n{ms:.1} ms\n"),
                    )
                    .unwrap();
                }
                let (mut ce, mut cn, mut we, mut wn) = (0f64, 0f64, 0f64, 0f64);
                for gt in m["gt_lines"].as_array().unwrap() {
                    let b = f32s(&gt["bbox_px"]);
                    let pad = 0.3 * (b[3] - b[1]);
                    let mut ws: Vec<_> = page
                        .words
                        .iter()
                        .filter(|w| {
                            let (cx, cy) =
                                ((w.bbox[0] + w.bbox[2]) / 2.0, (w.bbox[1] + w.bbox[3]) / 2.0);
                            cx >= b[0] - pad
                                && cx <= b[2] + pad
                                && cy >= b[1] - pad
                                && cy <= b[3] + pad
                        })
                        .collect();
                    ws.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
                    let ocr = ws
                        .iter()
                        .map(|w| w.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let t = gt["text"].as_str().unwrap();
                    let (n, nw) = (norm(t).len() as f64, t.split_whitespace().count() as f64);
                    ce += cer(t, &ocr) * n;
                    cn += n;
                    we += wer(t, &ocr) * nw;
                    wn += nw;
                }
                // Mean confidence as `ocr_warnings` computes it (char-weighted over words).
                let chars: f32 = page
                    .words
                    .iter()
                    .map(|w| w.text.chars().count() as f32)
                    .sum();
                let conf = page
                    .words
                    .iter()
                    .map(|w| w.conf * w.text.chars().count() as f32)
                    .sum::<f32>()
                    / chars.max(1.0);
                let resid = m["angle"].as_f64().unwrap_or(0.0) as f32 + page.skew_deg;
                let v = m["variant"].as_str().unwrap().to_string();
                (v, ce, cn, we, wn, conf, resid, page.rotation)
            })
            .collect()
    });
    let mut per: BTreeMap<&str, Vec<&ScanRow>> = BTreeMap::new();
    for r in &rows {
        per.entry(r.0.as_str()).or_default().push(r);
    }
    for (v, rs) in &per {
        let sum = |f: fn(&ScanRow) -> f64| rs.iter().map(|r| f(r)).sum::<f64>();
        let (ce, cn, we, wn) = (sum(|r| r.1), sum(|r| r.2), sum(|r| r.3), sum(|r| r.4));
        let mut confs: Vec<f32> = rs.iter().map(|r| r.5).collect();
        confs.sort_by(f32::total_cmp);
        let low = |t: f32| confs.iter().filter(|&&c| c < t).count();
        let res: Vec<f32> = rs.iter().map(|r| r.6.abs()).collect();
        eprintln!(
            "{v}: pages {} CER {:.4} WER {:.4} over {cn} chars / {wn} words | conf min {:.3} p10 {:.3} median {:.3} | \
             low-conf pages <0.70 {} <0.75 {} <0.80 {} <0.85 {} | residual skew mean {:.3} max {:.3} (>0.5°: {}) | rotated {} (undoing the turn {})",
            rs.len(),
            ce / cn,
            we / wn,
            confs[0],
            confs[confs.len() / 10],
            confs[confs.len() / 2],
            low(0.70),
            low(0.75),
            low(0.80),
            low(0.85),
            res.iter().sum::<f32>() / res.len() as f32,
            res.iter().copied().fold(0.0, f32::max),
            res.iter().filter(|&&r| r > 0.5).count(),
            rs.iter().filter(|r| r.7 != 0).count(),
            rs.iter().filter(|r| r.7 == (360 - turn) % 360).count(),
        );
    }
    for (v, gate) in GATES {
        let rs = per
            .get(v)
            .unwrap_or_else(|| panic!("variant {v} missing from meta.jsonl"));
        let c = rs.iter().map(|r| r.1).sum::<f64>() / rs.iter().map(|r| r.2).sum::<f64>();
        assert!(c <= gate, "{v}: CER {c:.4} > {gate}");
    }
}

#[test]
#[ignore = "needs LINES_EVAL_SET (ocr-tables degraded set + vector-line geometry)"]
fn synthetic_lines_regression() {
    // LINES_EVAL_SET/meta.jsonl: {"img", "variant": c150|c200|m150|m200|h150, "dpi", "angle",
    //   "vector_lines_pt": [[x1,y1,x2,y2], ...]}  (vector lines of the source PDF, ocr-tables geom/digital)
    // Gates (ocr-tables REPORT 4.3 at line_thr 0.80, minus ~1-2 pp). line_thr 0.85 (pale real scans) drops
    // h150 precision to ~0.53, so it is not the default and is not gated here.
    // Plan gates 0.94/0.97, 0.94/0.97, 0.94/0.965, 0.94/0.965, 0.92/0.83, tightened by the first Rust run to
    // measured - 0.5 pp (less 0.001 for the 3-digit rounding). Measured (140 pages, bicubic deskew):
    // c150 0.954/0.989, c200 0.952/0.990, m150 0.956/0.983, m200 0.953/0.985, h150 0.932/0.853 (research
    // 0.938/0.852; a bilinear deskew gave h150 precision 0.795).
    const GATES: [(&str, f64, f64); 5] = [
        ("c150", 0.948, 0.983),
        ("c200", 0.946, 0.984),
        ("m150", 0.950, 0.977),
        ("m200", 0.947, 0.979),
        ("h150", 0.926, 0.847),
    ];
    let dir = set_dir("LINES_EVAL_SET");
    // (variant, [covered vector length, vector length, matched raster length, raster length], residual skew)
    let rows: Vec<(String, [f64; 4], f32)> = pool().install(|| {
        meta_lines(&dir)
            .par_iter()
            .map(|m| {
                let dpi = m["dpi"].as_f64().unwrap() as f32;
                let g = raster::gray(
                    &image::open(dir.join(m["img"].as_str().unwrap()))
                        .unwrap()
                        .to_rgb8(),
                );
                // As the pipeline (`MIN_SKEW_DEG`, pipeline.rs) and the research: estimates below 0.25° are not applied.
                let skew = Some(raster::estimate_skew(&g))
                    .filter(|a| a.abs() >= 0.25)
                    .unwrap_or(0.0);
                let g = raster::rotate_gray(&g, skew, 255);
                let k = ((31.0 * dpi / 150.0) as u32) | 1;
                let mut lp = LinesParams::default(); // non_exhaustive
                lp.dpi = dpi;
                let s = 72.0 / dpi;
                let raster: Vec<[f32; 4]> =
                    extract_lines(&normalize_bg(&g, k), g.width(), g.height(), &lp)
                        .iter()
                        .map(|l| {
                            if l.horizontal {
                                [l.a * s, l.at * s, l.b * s, l.at * s]
                            } else {
                                [l.at * s, l.a * s, l.at * s, l.b * s]
                            }
                        })
                        .collect();
                let vector: Vec<[f32; 4]> = m["vector_lines_pt"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| {
                        let v = f32s(v);
                        [v[0], v[1], v[2], v[3]]
                    })
                    .collect();
                let [c, vl] = covered_length(&vector, &raster, 1.5);
                let [mr, rl] = covered_length(&raster, &vector, 1.5);
                let resid = m["angle"].as_f64().unwrap_or(0.0) as f32 + skew;
                (
                    m["variant"].as_str().unwrap().to_string(),
                    [c, vl, mr, rl],
                    resid,
                )
            })
            .collect()
    });
    let mut per: BTreeMap<&str, ([f64; 4], Vec<f32>)> = BTreeMap::new();
    for (v, e, r) in &rows {
        let p = per.entry(v.as_str()).or_default();
        (0..4).for_each(|i| p.0[i] += e[i]);
        p.1.push(r.abs());
    }
    for (v, ([c, vl, mr, rl], res)) in &per {
        eprintln!(
            "{v}: pages {} recall {:.3} precision {:.3} | residual skew mean {:.3} max {:.3}",
            res.len(),
            c / vl,
            mr / rl,
            res.iter().sum::<f32>() / res.len() as f32,
            res.iter().copied().fold(0.0, f32::max),
        );
    }
    for (v, rmin, pmin) in GATES {
        let [c, vl, mr, rl] = per
            .get(v)
            .unwrap_or_else(|| panic!("variant {v} missing from meta.jsonl"))
            .0;
        let (r, p) = (c / vl, mr / rl);
        assert!(
            r >= rmin && p >= pmin,
            "{v}: recall {r:.3} / precision {p:.3} (gates {rmin} / {pmin})"
        );
    }
}
