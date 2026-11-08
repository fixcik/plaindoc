//! RSS plateau of one PaddleOCR session pair on a mixed stream (SP5 task 24):
//! `PLAINDOC_OCR_MODELS=<dir> cargo test --release -p plaindoc-formats --features ocr-paddle \
//!  --test ocr_memory -- --ignored --test-threads 1 --nocapture`.
//! Gate: RSS after 60 documents <= 0.8 GB and growth from the 40th to the 60th document <= 5 %.

use image::imageops::FilterType;
use pdf_oxide::PdfDocument;
use pdf_oxide::rendering::{PageRenderer, RenderOptions};
use plaindoc_core::{Input, ParseOptions, WarningKind};
use plaindoc_formats::Registry;
use plaindoc_formats::ocr::paddle_engine;

const SCAN: &[u8] = include_bytes!("fixtures/pdf/scan.pdf");
const DOCS: usize = 60;
const LIMIT: u64 = 800 * 1024 * 1024;

fn rss_bytes() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .expect("rss")
        * 1024
}

/// PNG of the scan page with its longer side scaled to `long` px.
fn png(base: &image::DynamicImage, long: u32) -> Vec<u8> {
    let (w, h) = (base.width(), base.height());
    let k = f64::from(long) / f64::from(w.max(h));
    let img = base.resize_exact(
        ((f64::from(w) * k) as u32).max(1),
        ((f64::from(h) * k) as u32).max(1),
        FilterType::Triangle,
    );
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

#[test]
#[ignore = "needs OCR models"]
fn rss_plateaus_on_mixed_stream() {
    let doc = PdfDocument::from_bytes(SCAN.to_vec()).unwrap();
    let base = image::load_from_memory(
        &PageRenderer::new(RenderOptions::with_dpi(200))
            .render_page(&doc, 0)
            .unwrap()
            .data,
    )
    .unwrap();
    let reg = Registry::with_ocr_engine(paddle_engine(None, Some(1)));
    let mut seed = 12345u32;
    let mut series = Vec::new();
    for i in 1..=DOCS {
        let input = if i % 2 == 1 {
            Input::Bytes(SCAN.to_vec())
        } else {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            Input::Bytes(png(&base, 500 + (seed >> 8) % 2501))
        };
        let d = reg.parse(input, &ParseOptions::default()).unwrap();
        // Models missing must fail the test, not pass it vacuously.
        let unavailable = d
            .warnings
            .iter()
            .chain(d.sections.iter().flat_map(|s| &s.warnings))
            .any(|w| w.kind == WarningKind::OcrUnavailable);
        assert!(
            !unavailable,
            "OCR models unavailable (set PLAINDOC_OCR_MODELS)"
        );
        if i % 5 == 0 {
            let r = rss_bytes();
            println!("RSS after {i:2} docs: {:.0} MB", r as f64 / 1048576.0);
            series.push((i, r));
        }
    }
    let at = |n| series.iter().find(|s| s.0 == n).unwrap().1;
    let (r40, r60) = (at(40), at(60));
    let growth = (r60 as f64 - r40 as f64) / r40 as f64 * 100.0;
    println!("growth 40 -> 60: {growth:.1} %");
    assert!(r60 <= LIMIT, "RSS after {DOCS} docs {r60} > {LIMIT}");
    assert!(
        growth <= 5.0,
        "no plateau: +{growth:.1} % between docs 40 and 60"
    );
}
