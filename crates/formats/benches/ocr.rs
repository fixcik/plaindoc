//! OCR budgets (SP5): need-OCR classifier, 200-dpi render, pipeline overhead, raster lines, Paddle.
//! Run: `cargo bench -p plaindoc-formats --features ocr-paddle --bench ocr -- --save-baseline sp5`.

use criterion::{Criterion, criterion_group, criterion_main};
use pdf_oxide::writer::DocumentBuilder;
use plaindoc_core::{Format, Input, ParseOptions, Parser};
use plaindoc_formats::pdf::{NeedOcrConfig, PdfConfig, PdfParser, bench_render_page};
use plaindoc_ocr::{FakeEngine, Image, OcrParams, TextBox, ocr_image};
use std::time::Duration;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/pdf/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Copy of `pdf_common::builder_pdf_bytes` (benches cannot see test modules).
fn builder_pdf_bytes(pages: usize) -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    for i in 0..pages {
        b.letter_page()
            .font("Helvetica", 12.0)
            .at(72.0, 720.0)
            .heading(1, &format!("Chapter {i}"))
            .font("Helvetica", 12.0)
            .at(72.0, 690.0)
            .paragraph("First paragraph of body text on this page.")
            .at(72.0, 660.0)
            .paragraph("Second paragraph with a few more words in it.")
            .done();
    }
    b.build().unwrap()
}

/// Classifier cost: the same parse with the SP2 rules (`need_ocr = None`) and with the classifier
/// (default `Some(RECALL)` with the bundled bigram table, which adds the 50-dpi render and raster
/// features); no engine in both.
fn bench_need_ocr(c: &mut Criterion) {
    let mut g = c.benchmark_group("need_ocr/page");
    let mut off = PdfConfig::default();
    off.diagnostics.need_ocr = None;
    assert_eq!(
        PdfConfig::default().diagnostics.need_ocr,
        Some(NeedOcrConfig::RECALL)
    );
    let (off, on) = (PdfParser::with_config(off), PdfParser::new());
    for (name, bytes) in [
        ("builder_1p", builder_pdf_bytes(1)),
        ("imgtext", fixture("imgtext.pdf")),
    ] {
        for (mode, p) in [("rules", &off), ("classifier", &on)] {
            g.bench_function(format!("{name}/{mode}"), |b| {
                b.iter(|| {
                    p.parse(
                        Input::Bytes(bytes.clone()),
                        Format::Pdf,
                        &ParseOptions::default(),
                    )
                    .unwrap()
                })
            });
        }
    }
    g.finish();
}

fn bench_render(c: &mut Criterion) {
    let scan = fixture("scan.pdf");
    c.bench_function("render/200dpi", |b| {
        b.iter_custom(|iters| bench_render_page(scan.clone(), 200, iters).unwrap())
    });
}

/// Deterministic page-like raster: white, with dark "text lines" in two columns.
fn page(w: u32, h: u32, rows: u32) -> (Image, Vec<[f32; 4]>) {
    let mut img = Image::from_pixel(w, h, image::Rgb([250, 250, 250]));
    let mut boxes = Vec::new();
    let (rh, top) = (
        h as f32 / (rows as f32 + 2.0),
        h as f32 / (rows as f32 + 2.0),
    );
    for r in 0..rows {
        for col in 0..2u32 {
            let x0 = (80 + col * (w / 2)) as f32;
            let y0 = top + r as f32 * rh;
            let (x1, y1) = (x0 + w as f32 / 2.0 - 160.0, y0 + rh * 0.6);
            for y in y0 as u32..y1 as u32 {
                for x in (x0 as u32..x1 as u32).filter(|x| (x / 7 + y / 5) % 3 != 0) {
                    img.put_pixel(x, y, image::Rgb([30, 30, 30]));
                }
            }
            boxes.push([x0, y0, x1, y1]);
        }
    }
    (img, boxes)
}

/// `scan.pdf` page 0 rendered at 200 dpi (a real page raster).
fn scan_image() -> Image {
    use pdf_oxide::PdfDocument;
    use pdf_oxide::rendering::{ImageFormat, PageRenderer, RenderOptions};
    let doc = PdfDocument::from_bytes(fixture("scan.pdf")).unwrap();
    let mut o = RenderOptions::with_dpi(200);
    o.format = ImageFormat::RawRgba8;
    let r = PageRenderer::new(o).render_page(&doc, 0).unwrap();
    let rgb: Vec<u8> = r
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    Image::from_raw(r.width, r.height, rgb).unwrap()
}

fn fake_engine(boxes: &[[f32; 4]]) -> FakeEngine {
    FakeEngine::new(
        boxes
            .iter()
            .map(|b| {
                (
                    TextBox::from_aabb(b[0], b[1], b[2], b[3], 0.9),
                    "Договор поставки номер 123".to_string(),
                    0.95,
                )
            })
            .collect(),
    )
}

/// Pipeline overhead with FakeEngine on 92 boxes. Budget ruling (SP5 final): `ocr_image` ≤ 30 ms
/// applies to pages that need no alignment (the skew estimate is below `MIN_SKEW_DEG`, no rotation);
/// tilted pages get ≤ 100 ms — the skew search and the bicubic full-page rotation are the accepted cost
/// of tilted scans. `skew/estimate` isolates the search, which also runs on level pages.
/// Measured at the SP5 final (loaded machine, medians): level_page 96 ms, tilted_page 181 ms,
/// skew/estimate 56-61 ms, skew/rotate 87 ms, level_page_no_deskew 29 ms; `pipeline/fake` (also a
/// level page) 87 ms. After the coarse-to-fine skew search (D12, same angles; same machine, before ->
/// after): skew/estimate 53 -> 23 ms, level_page 86 -> 56 ms, tilted_page 165 -> 135 ms, fake 85 ->
/// 57 ms (rotate 78 ms, no_deskew 27 ms). Both budgets are still exceeded; not loosened.
fn bench_pipeline(c: &mut Criterion) {
    let p = OcrParams::default();
    let (img, boxes) = page(1653, 2339, 46);
    assert_eq!(boxes.len(), 92);
    let engine = fake_engine(&boxes);
    c.bench_function("pipeline/fake", |b| {
        b.iter(|| ocr_image(&engine, &img, &p).unwrap())
    });
    // Same 92 boxes on a really rendered page (A4 at 200 dpi, level): budget 30 ms.
    let real = scan_image();
    let (_, boxes) = page(real.width(), real.height(), 46);
    let engine = fake_engine(&boxes);
    assert_eq!(
        ocr_image(&engine, &real, &p).unwrap().skew_deg,
        0.0,
        "level page"
    );
    c.bench_function("pipeline/level_page", |b| {
        b.iter(|| ocr_image(&engine, &real, &p).unwrap())
    });
    // The same page tilted by 1.5°: deskewed (estimate + rotation), budget 100 ms.
    let tilted = plaindoc_ocr::raster::rotate(&real, 1.5, [255; 3]);
    assert_ne!(
        ocr_image(&engine, &tilted, &p).unwrap().skew_deg,
        0.0,
        "tilted page"
    );
    c.bench_function("pipeline/tilted_page", |b| {
        b.iter(|| ocr_image(&engine, &tilted, &p).unwrap())
    });
    let gray = plaindoc_ocr::raster::gray(&real);
    c.bench_function("skew/estimate", |b| {
        b.iter(|| plaindoc_ocr::raster::estimate_skew(&gray))
    });
    c.bench_function("skew/rotate", |b| {
        b.iter(|| plaindoc_ocr::raster::rotate(&real, 1.5, [255; 3]))
    });
    let mut no_deskew = p;
    no_deskew.deskew = false;
    c.bench_function("pipeline/level_page_no_deskew", |b| {
        b.iter(|| ocr_image(&engine, &real, &no_deskew).unwrap())
    });
}

fn bench_lines(c: &mut Criterion) {
    use plaindoc_ocr::lines::{LinesParams, extract_lines};
    use plaindoc_ocr::raster;
    let (mut img, _) = page(1240, 1754, 40);
    for y in (200..1600).step_by(200) {
        for x in 100..1140 {
            img.put_pixel(x, y, image::Rgb([0, 0, 0]));
        }
    }
    for x in (100..1141).step_by(260) {
        for y in 200..1400 {
            img.put_pixel(x, y, image::Rgb([0, 0, 0]));
        }
    }
    let norm = raster::normalize_bg(&raster::gray(&img), 31);
    let p = LinesParams::default();
    assert!(!extract_lines(&norm, 1240, 1754, &p).is_empty());
    c.bench_function("lines/150dpi", |b| {
        b.iter(|| extract_lines(&norm, 1240, 1754, &p))
    });
}

#[cfg(feature = "ocr-paddle")]
fn bench_paddle(c: &mut Criterion) {
    use plaindoc_ocr::{PaddleConfig, PaddleEngine};
    let mut cfg = PaddleConfig::default();
    cfg.max_sessions = 1;
    let engine = match PaddleEngine::new(
        plaindoc_ocr::models::resolve_models_dir(None).as_deref(),
        cfg,
    ) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("paddle/page skipped: {e}");
            return;
        }
    };
    let img = scan_image();
    let p = OcrParams::default();
    assert!(!ocr_image(&engine, &img, &p).unwrap().lines.is_empty());
    let mut g = c.benchmark_group("paddle");
    g.sample_size(10);
    g.bench_function("page", |b| b.iter(|| ocr_image(&engine, &img, &p).unwrap()));
    g.finish();
}

#[cfg(not(feature = "ocr-paddle"))]
fn bench_paddle(_: &mut Criterion) {}

criterion_group! {
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(3));
    targets = bench_need_ocr, bench_render, bench_pipeline, bench_lines, bench_paddle
}
criterion_main!(benches);
