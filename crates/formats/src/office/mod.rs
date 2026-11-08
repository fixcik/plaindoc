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
mod ocr_tests;
