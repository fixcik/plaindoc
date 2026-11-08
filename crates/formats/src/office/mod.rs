//! Office formats (DOCX/XLSX/PPTX and legacy DOC/XLS/PPT) via `office_oxide`.

use office_oxide::DocumentFormat;
use plaindoc_core::{
    Document, Format, Input, Level, ParseError, ParseOptions, Parser, Result, Warning, WarningKind,
};

mod blocks;
mod diag;
mod error;
mod form;
mod inline;
mod input;
#[cfg(feature = "ocr")]
mod ocr;
mod pictures;
mod section;
mod table;
pub use input::CfbStreams;
#[cfg(test)]
mod test_ir;
mod text;
mod title;
mod worker;

/// Formats handled by the office parser.
pub(crate) const OFFICE_FORMATS: [Format; 6] = [
    Format::Docx,
    Format::Xlsx,
    Format::Pptx,
    Format::Doc,
    Format::Xls,
    Format::Ppt,
];

/// Maps a core [`Format`] to the `office_oxide` format; `None` for non-office formats.
pub(crate) fn office_format(f: Format) -> Option<DocumentFormat> {
    Some(match f {
        Format::Docx => DocumentFormat::Docx,
        Format::Xlsx => DocumentFormat::Xlsx,
        Format::Pptx => DocumentFormat::Pptx,
        Format::Doc => DocumentFormat::Doc,
        Format::Xls => DocumentFormat::Xls,
        Format::Ppt => DocumentFormat::Ppt,
        _ => return None,
    })
}

/// OCR settings of a parse call; nothing without the `ocr` feature.
#[cfg(feature = "ocr")]
pub(crate) type OcrSetup = Option<ocr::OcrRun>;
#[cfg(not(feature = "ocr"))]
pub(crate) type OcrSetup = ();

/// Maps the IR and adds section diagnostics (and, with `ocr`, text of images in text-less sections).
pub(crate) fn document_from_ir(
    ir: &office_oxide::ir::DocumentIR,
    date1904: bool,
    max_pages: Option<usize>,
    pics: pictures::Pictures,
    ocr: &OcrSetup,
) -> (Document, Vec<Warning>) {
    let (mut out, map_warnings) = section::map_document(ir, date1904, max_pages, pics);
    for s in &mut out.sections {
        let w = diag::section_warnings(s);
        s.warnings.extend(w);
    }
    #[cfg(feature = "ocr")]
    if let Some(run) = ocr {
        run.apply(&mut out, ir);
    }
    #[cfg(not(feature = "ocr"))]
    let () = *ocr;
    (out, map_warnings)
}

/// Opens, converts and maps one office document; runs inside the worker thread.
pub(crate) fn parse_in_worker(
    input: Input,
    format: DocumentFormat,
    oo: office_oxide::options::ParseOptions,
    max_pages: Option<usize>,
    mut pics: pictures::Pictures,
    ocr: &OcrSetup,
) -> Result<Document> {
    let mut opened = input::open_input(input)?;
    if matches!(
        format,
        DocumentFormat::Docx | DocumentFormat::Pptx | DocumentFormat::Xlsx
    ) {
        pics.read_media(&mut opened.reader);
    }
    let has_formulas = matches!(
        format,
        DocumentFormat::Doc | DocumentFormat::Xls | DocumentFormat::Ppt
    ) && input::xls_has_formulas(&mut opened.reader);
    let doc = office_oxide::Document::from_reader_with(opened.reader, format, &oo)
        .map_err(|e| error::map_error(e, oo.limits.package_bytes))?;
    let date1904 = doc.as_xlsx().is_some_and(|x| x.workbook.date1904);
    let ir = doc.to_ir();
    drop(doc); // not needed past the IR: two copies of a large sheet at the peak, not three
    let (mut out, map_warnings) = document_from_ir(&ir, date1904, max_pages, pics, ocr);
    out.warnings.extend(diag::ir_warnings(&ir.metadata));
    out.warnings.extend(map_warnings);
    if ir.metadata.format == DocumentFormat::Xls && has_formulas {
        out.warnings.push(Warning::new(
            WarningKind::FormulaSkipped,
            Level::Info,
            "office: xls formula expressions are not extracted; cells hold displayed values",
        ));
    }
    let w = diag::output_warnings(&out, opened.len);
    out.warnings.extend(w);
    Ok(out)
}

/// Parser for DOCX/XLSX/PPTX and legacy DOC/XLS/PPT.
#[derive(Default, Clone)]
#[non_exhaustive]
pub struct OfficeParser {
    #[cfg(feature = "ocr")]
    ocr: Option<crate::ocr::OcrConfig>,
}

impl std::fmt::Debug for OfficeParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OfficeParser")
    }
}

impl OfficeParser {
    /// Creates the parser without OCR.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parser that OCRs images of sections without text (`ParseOptions::ocr` decides: `Never`
    /// off, `Auto` text-less sections, `Always` every image). At most 16 images per section and
    /// `config.limits.max_frames` (capped by `ocr_max_pages`) per document; each image gets
    /// `ocr_page_timeout`; images are decoded within `config.limits`.
    #[cfg(feature = "ocr")]
    pub fn with_ocr(config: crate::ocr::OcrConfig) -> Self {
        Self { ocr: Some(config) }
    }
}

impl Parser for OfficeParser {
    fn formats(&self) -> &[Format] {
        &OFFICE_FORMATS
    }

    /// `opts.password` (no decryption) and `page_parallel` are not used; timeout,
    /// `max_size` and quality are applied by the core boundary.
    fn parse(&self, input: Input, format: Format, opts: &ParseOptions) -> Result<Document> {
        let f = office_format(format).ok_or(ParseError::Unsupported(format))?;
        let oo = office_oxide::options::ParseOptions::default();
        let max_pages = opts.max_pages;
        let pics = pictures::Pictures::new(&opts.images);
        #[cfg(feature = "ocr")]
        let ocr = self.ocr.clone().map(|cfg| ocr::OcrRun {
            cfg,
            mode: opts.ocr,
            deadline: opts
                .timeout
                .and_then(|t| std::time::Instant::now().checked_add(t)),
            max_images: opts.ocr_max_pages,
            page_timeout: opts.ocr_page_timeout,
        });
        #[cfg(not(feature = "ocr"))]
        let ocr = ();
        worker::run_isolated(move || parse_in_worker(input, f, oo, max_pages, pics, &ocr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use office_oxide::DocumentFormat as D;
    use plaindoc_core::SectionKind;

    #[test]
    fn office_formats_map_one_to_one() {
        let mapped: Vec<_> = OFFICE_FORMATS.iter().map(|f| office_format(*f)).collect();
        assert_eq!(
            mapped,
            [
                Some(D::Docx),
                Some(D::Xlsx),
                Some(D::Pptx),
                Some(D::Doc),
                Some(D::Xls),
                Some(D::Ppt)
            ]
        );
        assert_eq!(office_format(Format::Pdf), None);
        assert_eq!(office_format(Format::Html), None);
    }

    fn fx(rel: &str) -> std::path::PathBuf {
        std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/office/"
        ))
        .join(rel)
    }

    #[test]
    fn parser_reports_office_formats_and_rejects_others() {
        let p = OfficeParser::new();
        assert_eq!(p.formats(), &OFFICE_FORMATS);
        assert!(matches!(
            p.parse(Input::Bytes(vec![]), Format::Pdf, &ParseOptions::default()),
            Err(ParseError::Unsupported(Format::Pdf))
        ));
    }

    #[test]
    fn parse_docx_end_to_end() {
        let d = OfficeParser::new()
            .parse(
                Input::Path(fx("generated/tables.docx")),
                Format::Docx,
                &ParseOptions::default(),
            )
            .unwrap();
        assert_eq!(d.sections.len(), 1);
        assert_eq!(d.sections[0].kind, SectionKind::Body);
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    }

    #[test]
    fn legacy_format_hint_is_corrected_by_office_oxide() {
        let d = OfficeParser::new()
            .parse(
                Input::Path(fx("poi/SimpleWithFormula.xls")),
                Format::Doc,
                &ParseOptions::default(),
            )
            .unwrap();
        assert!(d.sections.iter().all(|s| s.kind == SectionKind::Sheet));
    }

    #[test]
    fn encrypted_given_as_docx_is_encrypted() {
        let r = OfficeParser::new().parse(
            Input::Path(fx("generated/encrypted.docx")),
            Format::Docx,
            &ParseOptions::default(),
        );
        assert!(matches!(r, Err(ParseError::Encrypted)));
    }

    #[test]
    fn text_budget_yields_truncated_loss_for_every_format() {
        for (rel, f) in [
            ("generated/tables.docx", D::Docx),
            ("generated/values.xlsx", D::Xlsx),
            ("generated/slides.pptx", D::Pptx),
            ("poi/table-merges.doc", D::Doc),
            ("poi/SimpleWithFormula.xls", D::Xls),
        ] {
            let mut oo = office_oxide::options::ParseOptions::default();
            oo.limits.text_chars = 10;
            let d = parse_in_worker(
                Input::Path(fx(rel)),
                f,
                oo,
                None,
                Default::default(),
                &Default::default(),
            )
            .unwrap();
            assert!(
                d.warnings
                    .iter()
                    .any(|w| (w.kind, w.level) == (WarningKind::Truncated, Level::Loss)),
                "{rel}: {:?}",
                d.warnings
            );
        }
    }

    #[test]
    fn xls_formulas_give_one_formula_skipped() {
        let d = parse_in_worker(
            Input::Path(fx("poi/SimpleWithFormula.xls")),
            D::Xls,
            Default::default(),
            None,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        let n = d
            .warnings
            .iter()
            .filter(|w| w.kind == WarningKind::FormulaSkipped)
            .count();
        assert_eq!(n, 1, "{:?}", d.warnings);
    }
}

#[cfg(all(test, feature = "ocr"))]
mod ocr_tests {
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
        let (mut d, w) = document_from_ir(ir, false, None, Default::default(), &ocr);
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
        let (d, _) = document_from_ir(&ir, false, None, pics, &ocr);
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
            d.warnings.iter().any(|w| (w.kind, w.level)
                == (WarningKind::PartialContent, Level::Warn)
                && w.message.contains("OCR limited to 2 of 5 images")),
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
            s.warnings.iter().any(|w| (w.kind, w.level)
                == (WarningKind::PartialContent, Level::Warn)
                && w.message.contains("timed out")),
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
        let (d, _) = document_from_ir(&ir, false, Some(1), Default::default(), &ocr);
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
}
