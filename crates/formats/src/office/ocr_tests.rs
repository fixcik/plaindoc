//! OCR of images in office sections, with a fake engine.

use super::test_ir::*;
use super::*;
use crate::ocr::OcrConfig;
use office_oxide::ir::{DocumentIR, Section as IrSection};
use plaindoc_core::{Block, OcrMode};
use plaindoc_ocr::{FakeEngine, TextBox};
use std::io::Cursor;
use std::sync::Arc;

mod models;

fn png_with_bar() -> Vec<u8> {
    let mut img = image::RgbImage::from_pixel(200, 100, image::Rgb([255, 255, 255]));
    for y in 20..40 {
        for x in 20..120 {
            img.put_pixel(x, y, image::Rgb([0, 0, 0]));
        }
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn fake_ocr(text: &str, conf: f32) -> (Arc<FakeEngine>, OcrConfig) {
    let e = Arc::new(FakeEngine::new(vec![(
        TextBox::from_aabb(15.0, 15.0, 125.0, 45.0, 0.9),
        text.into(),
        conf,
    )]));
    (e.clone(), OcrConfig::with_engine(e))
}

fn slide(elements: Vec<office_oxide::ir::Element>) -> DocumentIR {
    let mut ir = DocumentIR::default();
    ir.metadata.format = DocumentFormat::Pptx;
    ir.sections = vec![IrSection {
        elements,
        ..Default::default()
    }];
    ir
}

fn run(cfg: OcrConfig, mode: OcrMode, ir: &DocumentIR) -> Document {
    run_with(cfg, mode, ir, None, None)
}

fn run_with(
    cfg: OcrConfig,
    mode: OcrMode,
    ir: &DocumentIR,
    max_images: Option<usize>,
    page_timeout: Option<std::time::Duration>,
) -> Document {
    let ocr = Some(ocr::OcrRun {
        cfg,
        mode,
        deadline: None,
        max_images,
        page_timeout,
    });
    let (mut d, w) = document_from_ir(ir.clone(), false, None, Default::default(), &ocr);
    d.warnings.extend(w);
    d
}

fn has(w: &[Warning], kind: WarningKind, level: Level) -> bool {
    w.iter().any(|w| (w.kind, w.level) == (kind, level))
}

#[test]
fn header_pictures_follow_the_body_and_keep_the_ocr_pairing() {
    let mut ir = slide(vec![image_with_data(png_with_bar())]);
    ir.metadata.format = DocumentFormat::Docx;
    let logo = image_with_data(b"\x89PNG\r\n\x1a\nlogo".to_vec());
    ir.sections[0].header = Some(office_oxide::ir::HeaderFooter {
        content: vec![logo],
    });
    let (_, cfg) = fake_ocr("Тело", 0.9);
    let ocr = Some(ocr::OcrRun {
        cfg,
        mode: OcrMode::Auto,
        deadline: None,
        max_images: None,
        page_timeout: None,
    });
    let pics = pictures::Pictures::new(&plaindoc_core::ImageOptions::default());
    let (d, _) = document_from_ir(ir, false, None, pics, &ocr);
    let imgs: Vec<_> = (d.sections[0].blocks.iter())
        .filter_map(|b| match b {
            Block::Image {
                ocr_text, image, ..
            } => Some((ocr_text.clone(), image.is_some())),
            _ => None,
        })
        .collect();
    assert_eq!(imgs, [(Some("Тело".into()), true), (None, true)]);
}

#[test]
fn image_only_slide_gets_ocr_text() {
    let ir = slide(vec![image_with_data(png_with_bar())]);
    let (_, cfg) = fake_ocr("Слайд 1", 0.9);
    let d = run(cfg, OcrMode::Auto, &ir);
    let s = &d.sections[0];
    assert!(matches!(&s.blocks[0], Block::Image { ocr_text: Some(t), .. } if t == "Слайд 1"));
    assert!(has(&s.warnings, WarningKind::OcrUsed, Level::Info));
    assert!(has(&s.warnings, WarningKind::ImageTextPage, Level::Info));
    assert!(!has(&s.warnings, WarningKind::ImageTextPage, Level::Warn));
    assert!(!has(&d.warnings, WarningKind::EmptyOutput, Level::Warn));
}

#[test]
fn no_engine_unavailable_level_follows_the_mode() {
    let ir = slide(vec![image_with_data(png_with_bar())]);
    let text_ir = slide(vec![para("Есть текст"), image_with_data(png_with_bar())]);
    let level = |d: &Document| {
        d.warnings
            .iter()
            .find(|w| w.kind == WarningKind::OcrUnavailable)
            .map(|w| w.level)
    };
    let none = || OcrConfig::default();
    assert_eq!(level(&run(none(), OcrMode::Auto, &ir)), Some(Level::Info));
    assert_eq!(level(&run(none(), OcrMode::Always, &ir)), Some(Level::Warn));
    assert_eq!(level(&run(none(), OcrMode::Never, &ir)), None);
    // Always: any image with bytes counts, also in a section that has text.
    assert_eq!(
        level(&run(none(), OcrMode::Always, &text_ir)),
        Some(Level::Warn)
    );
    assert_eq!(level(&run(none(), OcrMode::Auto, &text_ir)), None);
    let d = run(none(), OcrMode::Auto, &ir);
    assert!(has(
        &d.sections[0].warnings,
        WarningKind::ImageTextPage,
        Level::Warn
    ));
}

#[test]
fn never_keeps_the_old_warning_and_skips_the_engine() {
    let ir = slide(vec![image_with_data(png_with_bar())]);
    let (e, cfg) = fake_ocr("x", 0.9);
    let d = run(cfg, OcrMode::Never, &ir);
    assert_eq!(e.detect_calls(), 0);
    assert!(has(
        &d.sections[0].warnings,
        WarningKind::ImageTextPage,
        Level::Warn
    ));
    assert!(
        d.warnings
            .iter()
            .all(|w| w.kind != WarningKind::OcrUnavailable)
    );
}

#[test]
fn sections_with_text_are_ocred_only_when_always() {
    let ir = slide(vec![para("Есть текст"), image_with_data(png_with_bar())]);
    let (e, cfg) = fake_ocr("Картинка", 0.9);
    let d = run(cfg.clone(), OcrMode::Auto, &ir);
    assert_eq!(e.detect_calls(), 0);
    assert!(matches!(
        &d.sections[0].blocks[1],
        Block::Image { ocr_text: None, .. }
    ));
    let d = run(cfg, OcrMode::Always, &ir);
    assert_eq!(e.detect_calls(), 1);
    assert!(matches!(
        &d.sections[0].blocks[1],
        Block::Image {
            ocr_text: Some(_),
            ..
        }
    ));
}

#[test]
fn image_next_to_a_table_is_ocred_after_the_table_is_mapped() {
    let t = table(vec![row(vec![cell("a", 1, 1)], false)]);
    let ir = slide(vec![t, image_with_data(png_with_bar())]);
    let (e, cfg) = fake_ocr("Схема", 0.9);
    let d = run(cfg, OcrMode::Always, &ir);
    assert_eq!(e.detect_calls(), 1);
    let b = &d.sections[0].blocks;
    assert!(matches!(&b[0], Block::Table { rows, .. } if rows[0][0].text() == "a"));
    assert!(matches!(&b[1], Block::Image { ocr_text: Some(t), .. } if t == "Схема"));
    // A picture inside a cell is not a section image: neither OCRed nor paired.
    let mut c = cell("a", 1, 1);
    c.content.push(image_with_data(png_with_bar()));
    let ir = slide(vec![
        table(vec![row(vec![c], false)]),
        image_with_data(png_with_bar()),
    ]);
    let (e, cfg) = fake_ocr("Схема", 0.9);
    let d = run(cfg, OcrMode::Always, &ir);
    assert_eq!(e.detect_calls(), 1);
    let b = &d.sections[0].blocks;
    assert!(matches!(&b[1], Block::Image { ocr_text: Some(t), .. } if t == "Схема"));
}

#[test]
fn bad_images_degrade_and_the_count_is_capped() {
    let mut els = vec![
        image_with_data(b"\x89PNG\r\n\x1a\ngarbage".to_vec()),
        image_with_data(b"GIF89a....".to_vec()),
        image(None),
    ];
    els.extend((0..20).map(|_| image_with_data(png_with_bar())));
    let (e, cfg) = fake_ocr("Слайд", 0.9);
    let d = run(cfg, OcrMode::Auto, &slide(els));
    assert_eq!(e.detect_calls(), ocr::MAX_REGIONS - 1); // 16 attempts, one is the garbage PNG
    let s = &d.sections[0];
    assert!(has(&s.warnings, WarningKind::PartialContent, Level::Warn));
    assert!(has(&s.warnings, WarningKind::OcrUsed, Level::Info));
}

#[test]
fn ocr_max_pages_caps_images_per_document() {
    let els = (0..5).map(|_| image_with_data(png_with_bar())).collect();
    let (e, cfg) = fake_ocr("Слайд", 0.9);
    let d = run_with(cfg, OcrMode::Auto, &slide(els), Some(2), None);
    assert_eq!(e.detect_calls(), 2);
    assert!(
        d.warnings.iter().any(
            |w| (w.kind, w.level) == (WarningKind::PartialContent, Level::Warn)
                && w.message.contains("OCR limited to 2 of 5 images")
        ),
        "{:?}",
        d.warnings
    );
}

#[test]
fn ocr_page_timeout_bounds_each_image() {
    let ir = slide(vec![image_with_data(png_with_bar())]);
    let (_, cfg) = fake_ocr("Слайд", 0.9);
    let d = run_with(
        cfg,
        OcrMode::Auto,
        &ir,
        None,
        Some(std::time::Duration::ZERO),
    );
    let s = &d.sections[0];
    assert!(
        s.warnings.iter().any(
            |w| (w.kind, w.level) == (WarningKind::PartialContent, Level::Warn)
                && w.message.contains("timed out")
        ),
        "{:?}",
        s.warnings
    );
}

#[test]
fn huge_timeout_does_not_panic() {
    let (_, cfg) = fake_ocr("x", 0.9);
    let mut opts = ParseOptions::default();
    opts.timeout = Some(std::time::Duration::MAX);
    let d = OfficeParser::with_ocr(cfg)
        .parse(
            Input::Path(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/office/generated/slides.pptx"
                )
                .into(),
            ),
            Format::Pptx,
            &opts,
        )
        .unwrap();
    assert!(!d.sections.is_empty());
}

#[test]
fn sheet_named_slide_master_is_not_taken_for_the_master() {
    // xlsx whose last sheet is titled like office_oxide's slide master; max_pages = 1 truncates.
    let mut ir = DocumentIR::default();
    ir.metadata.format = DocumentFormat::Xlsx;
    ir.sections = vec![
        IrSection {
            elements: vec![image_with_data(png_with_bar())],
            ..Default::default()
        },
        IrSection::default(),
        IrSection {
            title: Some(office_oxide::pptx::layout::MASTER_TEXT_SECTION_TITLE.into()),
            ..Default::default()
        },
    ];
    let (e, cfg) = fake_ocr("Лист 1", 0.9);
    let ocr = Some(ocr::OcrRun {
        cfg,
        mode: OcrMode::Always,
        deadline: None,
        max_images: None,
        page_timeout: None,
    });
    let (d, _) = document_from_ir(ir, false, Some(1), Default::default(), &ocr);
    assert_eq!(d.sections.len(), 1);
    assert_eq!(e.detect_calls(), 1, "sheet 1's own image is OCRed");
    assert!(
        matches!(&d.sections[0].blocks[0], Block::Image { ocr_text: Some(t), .. } if t == "Лист 1")
    );
}

#[test]
fn only_bad_images_keep_the_warning_and_do_not_fail() {
    let ir = slide(vec![image_with_data(b"\x89PNG\r\n\x1a\ngarbage".to_vec())]);
    let (_, cfg) = fake_ocr("x", 0.9);
    let d = run(cfg, OcrMode::Auto, &ir);
    let s = &d.sections[0];
    assert!(has(&s.warnings, WarningKind::ImageTextPage, Level::Warn));
    assert!(matches!(&s.blocks[0], Block::Image { ocr_text: None, .. }));
}
