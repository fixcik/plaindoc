//! End-to-end with the real PaddleOCR engine (ort, CPU). Needs the models (bundled or
//! `PLAINDOC_OCR_MODELS`): `cargo test --features ocr-paddle --test ocr_paddle_smoke -- --ignored`.
//! The text image is the page of the `scan.pdf` fixture rendered to PNG.

use pdf_oxide::PdfDocument;
use pdf_oxide::rendering::{PageRenderer, RenderOptions};
use plaindoc_core::{Block, Input, ParseOptions, WarningKind};
use plaindoc_formats::Registry;
use plaindoc_formats::ocr::paddle_engine;

const SCAN: &[u8] = include_bytes!("fixtures/pdf/scan.pdf");

fn scan_png() -> Vec<u8> {
    let doc = PdfDocument::from_bytes(SCAN.to_vec()).unwrap();
    PageRenderer::new(RenderOptions::with_dpi(200))
        .render_page(&doc, 0)
        .unwrap()
        .data
}

#[test]
#[ignore = "needs OCR models"]
fn text_image_is_read_by_paddle_and_is_deterministic() {
    let png = scan_png();
    let engine = paddle_engine(None, Some(1));
    let img = image::load_from_memory(&png).unwrap().to_rgb8();
    let boxes = engine.detect(&img).expect("models must load");
    assert!(!boxes.is_empty(), "no text found in the rendered scan");

    let reg = Registry::with_ocr_engine(engine);
    let parse = || {
        reg.parse(Input::Bytes(png.clone()), &ParseOptions::default())
            .unwrap()
    };
    let d = parse();
    let all: Vec<_> = d
        .warnings
        .iter()
        .chain(d.sections.iter().flat_map(|s| &s.warnings))
        .collect();
    assert!(
        all.iter().all(|w| w.kind != WarningKind::OcrUnavailable),
        "{all:?}"
    );
    let text: String = d
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .filter_map(|b| match b {
            Block::Paragraph { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    assert!(text.contains("договор"), "{text:?}");
    assert_eq!(d.to_json(), parse().to_json());
}

#[test]
#[ignore = "needs OCR models"]
fn scan_pdf_is_read_by_paddle() {
    use plaindoc_core::{Level, TextSource};
    let reg = Registry::with_ocr_engine(paddle_engine(None, Some(1)));
    let d = reg
        .parse(Input::Bytes(SCAN.to_vec()), &ParseOptions::default())
        .unwrap();
    assert_eq!(d.sections[0].source, TextSource::Ocr);
    let all: Vec<_> = d
        .warnings
        .iter()
        .chain(d.sections.iter().flat_map(|s| &s.warnings))
        .collect();
    assert!(
        all.iter()
            .any(|w| w.kind == WarningKind::OcrUsed && w.level == Level::Info),
        "{all:?}"
    );
    assert!(
        all.iter().all(|w| w.kind != WarningKind::OcrUnavailable),
        "{all:?}"
    );
    let text: String = d
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .filter_map(|b| match b {
            Block::Paragraph { text, .. } | Block::Heading { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Договор поставки"), "{text:?}");
}
