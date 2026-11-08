mod pdf_common;
use pdf_common::*;
use plaindoc_core::*;
use plaindoc_formats::pdf::{PdfConfig, PdfParser};

fn has(doc: &Document, page: Option<usize>, kind: WarningKind, level: Level) -> bool {
    levels(doc).contains(&(page, kind, level))
}
fn no_warn_or_loss(doc: &Document) -> bool {
    levels(doc).iter().all(|(_, _, l)| *l == Level::Info)
}

// Engine-less `PdfParser`: through `Registry::with_defaults` (with `ocr-paddle` and models) the scan
// is OCRed instead, see `default_registry_pdf_parser_uses_the_default_engine`.
#[test]
fn scan_is_no_text_layer_loss() {
    let d = parse_with(&PdfConfig::default(), &fixture("scan.pdf")).unwrap();
    assert!(has(&d, Some(0), WarningKind::NoTextLayer, Level::Loss));
    assert!(has(&d, None, WarningKind::OcrUnavailable, Level::Info));
    assert!(d.sections[0].blocks.is_empty());
    assert_eq!(d.sections[0].quality, 0.0);
    assert_eq!(d.quality, 0.0);
}
/// With models (bundled or `$PLAINDOC_OCR_MODELS`) the scan is OCRed; without them (clean clone,
/// soft bundling) the parse still succeeds and reports `OcrUnavailable` naming what is missing.
///
#[cfg(feature = "ocr-paddle")]
#[test]
fn scan_is_ocred_and_the_loss_becomes_info() {
    use plaindoc_formats::ocr::OcrConfig;
    use plaindoc_formats::ocr::models::{DET_V5_MOBILE, load_model, resolve_models_dir};
    let mut c = plaindoc_formats::pdf::PdfConfig::default();
    c.ocr = OcrConfig::default_engine();
    let d = parse_with(&c, &fixture("scan.pdf")).unwrap();
    if load_model(&DET_V5_MOBILE, resolve_models_dir(None).as_deref()).is_err() {
        assert!(has(&d, Some(0), WarningKind::NoTextLayer, Level::Loss));
        let w = d
            .warnings
            .iter()
            .find(|w| w.kind == WarningKind::OcrUnavailable)
            .unwrap();
        assert!(w.message.contains(DET_V5_MOBILE.id), "{}", w.message);
        return;
    }
    assert!(has(&d, Some(0), WarningKind::NoTextLayer, Level::Info));
    assert!(has(&d, Some(0), WarningKind::OcrUsed, Level::Info));
    assert_eq!(d.sections[0].source, TextSource::Ocr);
    assert!(texts(&d.sections[0]).concat().contains("Договор поставки"));
    assert!(d.quality > 0.9, "{}", d.quality);
    let again = parse_with(&c, &fixture("scan.pdf")).unwrap();
    assert_eq!(d.to_json(), again.to_json(), "real engine is deterministic");
}
/// `default_parsers` PDF branch: `Registry::with_defaults` registers `PdfParser::with_ocr` with the
/// build's default engine. With models the scan is OCRed; without them `OcrUnavailable` names the
/// missing model (`ocr-paddle`) or the missing engine (`ocr` only).
#[cfg(feature = "ocr")]
#[test]
fn default_registry_pdf_parser_uses_the_default_engine() {
    let d = parse_path("scan.pdf").unwrap();
    #[cfg(feature = "ocr-paddle")]
    let missing = {
        use plaindoc_formats::ocr::models::{DET_V5_MOBILE, load_model, resolve_models_dir};
        load_model(&DET_V5_MOBILE, resolve_models_dir(None).as_deref())
            .err()
            .map(|_| DET_V5_MOBILE.id)
    };
    #[cfg(not(feature = "ocr-paddle"))]
    let missing = Some("no OCR engine");
    match missing {
        Some(why) => {
            assert!(has(&d, Some(0), WarningKind::NoTextLayer, Level::Loss));
            let w = d
                .warnings
                .iter()
                .find(|w| w.kind == WarningKind::OcrUnavailable)
                .unwrap();
            assert!(w.message.contains(why), "{}", w.message);
        }
        None => {
            assert!(has(&d, Some(0), WarningKind::OcrUsed, Level::Info));
            assert_eq!(d.sections[0].source, TextSource::Ocr);
            assert!(texts(&d.sections[0]).concat().contains("Договор поставки"));
        }
    }
}
/// Default classifier: the image-text hint needs the 50-dpi ink (feature `ocr`); without a
/// raster the page is plain text (the SP2 hint is in `sp2_config_keeps_the_sp2_stage_one`).
#[test]
fn image_text_is_info_and_text_survives() {
    let d = parse_path("imgtext.pdf").unwrap();
    let hint = has(&d, Some(0), WarningKind::ImageTextPage, Level::Info);
    assert!(hint == cfg!(feature = "ocr") && no_warn_or_loss(&d));
    assert!(
        matches!(&d.sections[0].blocks[0], Block::Heading { level: 1, text, .. } if text == "Отчёт")
    );
}
#[test]
fn truncated_is_truncated_and_garbage() {
    let d = parse_bytes(truncated_bytes(), &ParseOptions::default()).unwrap();
    assert!(has(&d, None, WarningKind::Truncated, Level::Warn));
    assert!(has(&d, Some(0), WarningKind::SuspectedGarbage, Level::Loss));
}
#[test]
fn mojibake_without_truncation() {
    let d = parse_bytes(mojibake_bytes(), &ParseOptions::default()).unwrap();
    assert!(has(&d, Some(0), WarningKind::SuspectedGarbage, Level::Loss));
    assert!(
        !levels(&d)
            .iter()
            .any(|(_, k, _)| *k == WarningKind::Truncated)
    );
}
#[test]
fn encrypted_requires_password() {
    let b = encrypted_bytes("user");
    assert!(matches!(
        parse_bytes(b.clone(), &ParseOptions::default()),
        Err(ParseError::Encrypted)
    ));
    let mut o = ParseOptions::default();
    o.password = Some("wrong".into());
    assert!(matches!(
        parse_bytes(b.clone(), &o),
        Err(ParseError::Encrypted)
    ));
    o.password = Some("user".into());
    let d = parse_bytes(b, &o).unwrap();
    assert!(has(&d, None, WarningKind::Encrypted, Level::Info));
    assert!(texts(&d.sections[0]).concat().contains("Secret body text."));
}
#[test]
fn encrypted_with_empty_user_password_opens() {
    let d = parse_bytes(encrypted_bytes(""), &ParseOptions::default()).unwrap();
    assert!(has(&d, None, WarningKind::Encrypted, Level::Info));
}
#[test]
fn blank_page_and_document() {
    let d = parse_path("empty.pdf").unwrap();
    assert!(has(&d, Some(0), WarningKind::EmptyOutput, Level::Info));
    assert!(has(&d, None, WarningKind::EmptyOutput, Level::Warn));
}
#[test]
fn zero_bytes_and_garbage_are_corrupt_without_panic() {
    assert!(matches!(
        parse_bytes(vec![], &ParseOptions::default()),
        Err(ParseError::Corrupt(_))
    ));
    let mut g = b"%PDF-1.7\n".to_vec();
    g.extend(vec![b'x'; 480]);
    assert!(matches!(
        parse_bytes(g, &ParseOptions::default()),
        Err(ParseError::Corrupt(_))
    ));
}
#[test]
fn every_prefix_of_ru_pdf_parses_without_panic() {
    // 69 cuts, 0 panics, ~1 s debug (measured)
    let full = std::fs::read(fixture("ru.pdf")).unwrap();
    let mut o = ParseOptions::default();
    o.timeout = Some(std::time::Duration::from_secs(10));
    for cut in (0..full.len()).step_by(997) {
        let r = parse_bytes(full[..cut].to_vec(), &o);
        assert!(
            !matches!(
                r,
                Err(ParseError::Internal(_)) | Err(ParseError::Timeout(_))
            ),
            "cut {cut}: {r:?}"
        );
    }
}
#[test]
fn russian_control_file_is_clean_and_keeps_marginals() {
    let d = parse_path("ru.pdf").unwrap();
    assert!(no_warn_or_loss(&d));
    let b = &d.sections[0].blocks;
    assert!(
        matches!(&b[0], Block::Heading { level: 1, text, .. } if text == "Годовой отчёт компании")
    );
    assert!(
        b.iter()
            .any(|x| matches!(x, Block::Heading { level: 2, text, .. } if text == "Показатели"))
    );
    assert!(
        b.iter()
            .any(|x| matches!(x, Block::Heading { level: 2, text, .. } if text == "Описание"))
    );
    // SP4: the table cell "300" that the engine labelled MarginalLabel now lives only in the table.
    assert!(
        b.iter()
            .any(|x| matches!(x, Block::Table { rows, header_rows: 1, .. }
        if rows.len() == 3 && rows[1][2].blocks == vec![Block::paragraph("300", None)]))
    );
    assert!(
        !b.iter()
            .any(|x| matches!(x, Block::Note { text, .. } if text.contains("300")))
    );
}
#[test]
fn russian_two_column_prose_is_read_left_column_first() {
    // «Описание» is two columns of prose under a title, headings and a table that the engine puts
    // into column 0 as well; the column gate must not read the page row by row
    let d = parse_path("ru.pdf").unwrap();
    let text: Vec<&str> = d.sections[0]
        .blocks
        .iter()
        .filter_map(|x| match x {
            Block::Paragraph { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let text = text.join("\n");
    let at = |t: &str| text.find(t).unwrap_or_else(|| panic!("{t:?} missing"));
    assert!(at("Первая колонка") < at("всего года."));
    assert!(at("всего года.") < at("Вторая часть"));
    assert!(at("Вторая часть") < at("продуктов."));
}
#[test]
fn xy_cut_puts_the_russian_intro_under_the_title() {
    // the full-width intro line has no column: region order puts it between the two «Описание»
    // columns, the XY-cut order between the title and «Показатели»
    let mut c = PdfConfig::default();
    c.layout.reading_order = plaindoc_formats::pdf::ReadingOrderMode::XyCut;
    let d = parse_with(&c, &fixture("ru.pdf")).unwrap();
    let text: Vec<&str> = d.sections[0]
        .blocks
        .iter()
        .filter_map(|x| match x {
            Block::Heading { text, .. } | Block::Paragraph { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let text = text.join("\n");
    let at = |t: &str| text.find(t).unwrap_or_else(|| panic!("{t:?} missing"));
    assert!(at("Годовой отчёт") < at("Это тестовый документ"));
    assert!(at("Это тестовый документ") < at("Показатели"));
    assert!(at("Первая колонка") < at("Вторая часть"));
}

#[test]
fn notes_headers_footers_headings_and_bbox_order() {
    let d = parse_path("notes.pdf").unwrap();
    assert_eq!(d.sections.len(), 3);
    for s in &d.sections {
        let y = |pred: &dyn Fn(&Block) -> bool| {
            s.blocks
                .iter()
                .find(|b| pred(b))
                .and_then(bbox_of)
                .map(|b| (b.y, b.h))
                .unwrap()
        };
        let (hy, hh) = y(&|b| {
            matches!(
                b,
                Block::Note {
                    kind: NoteKind::Header,
                    ..
                }
            )
        });
        let (gy, _) = y(&|b| matches!(b, Block::Heading { level: 1, .. }));
        let (fy, _) = y(&|b| {
            matches!(
                b,
                Block::Note {
                    kind: NoteKind::Footer,
                    ..
                }
            )
        });
        assert!(hy > gy && gy > fy && hy + hh <= 792.0, "{hy} {gy} {fy}");
        assert!(
            s.warnings
                .iter()
                .any(|w| w.kind == WarningKind::ContentDropped && w.level == Level::Info)
        );
    }
    assert!(
        d.sections[0]
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Heading { level: 1, text, .. } if text == "Chapter 1"))
    );
    assert!(
        d.sections[0]
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Heading { level: 2, text, .. } if text == "Section 1.1"))
    );
    let mut o = MarkdownOptions::default();
    assert!(!d.to_markdown_with(&o).contains("<!-- header:"));
    o.headers_footers = true;
    let md = d.to_markdown_with(&o);
    assert!(
        md.contains("<!-- header:") && md.contains("<!-- footer:"),
        "{md}"
    );
}
#[test]
fn path_and_bytes_inputs_give_same_document() {
    let p = parse_path("ru.pdf").unwrap();
    let b = parse_bytes(
        std::fs::read(fixture("ru.pdf")).unwrap(),
        &ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(format!("{:?}", p.sections), format!("{:?}", b.sections));
}
/// Stage 1 with the classifier off (`need_ocr = None`) equals the pre-task-15 output: page
/// warnings with messages, page quality, document warning kinds and levels. Only the document
/// `OcrUnavailable` message changed (Р-10 names the counts and the reason); with the switch off
/// it still counts only full-OCR pages (ruling Р-6), so `imgtext.pdf` gets none.
#[test]
fn sp2_config_keeps_the_sp2_stage_one() {
    use WarningKind::*;
    let off = sp2_off();
    let parse_path = |name: &str| parse_with(&off, &fixture(name));
    let parse_bytes = |b: Vec<u8>, o: &ParseOptions| {
        PdfParser::with_config(off.clone()).parse(Input::Bytes(b), Format::Pdf, o)
    };
    type Page<'a> = (usize, Vec<(WarningKind, Level, &'a str)>, f32);
    type DocWarnings = Vec<(WarningKind, Level)>;
    let dropped = |i| {
        (
            i,
            vec![(
                ContentDropped,
                Level::Info,
                "dropped 1 artifact regions (3 chars)",
            )],
            1.0,
        )
    };
    let moji = vec![(
        0,
        vec![(
            SuspectedGarbage,
            Level::Loss,
            "text layer looks like mojibake (359 glyphs, 0.00 image area, 0.00 garbled, 39 tokens, 0.54 mojibake ratio)",
        )],
        0.0,
    )];
    let want: Vec<(Document, Vec<Page>, DocWarnings)> = vec![
        (
            parse_path("empty.pdf").unwrap(),
            vec![(
                0,
                vec![(
                    EmptyOutput,
                    Level::Info,
                    "blank page (0 glyphs, 0.00 image area, 0.00 garbled, 0 tokens, 0.00 mojibake ratio)",
                )],
                1.0,
            )],
            vec![(EmptyOutput, Level::Warn)],
        ),
        (
            parse_path("imgtext.pdf").unwrap(),
            vec![(
                0,
                vec![(
                    ImageTextPage,
                    Level::Info,
                    "page has images that may contain text (46 glyphs, 0.16 image area, 0.00 garbled, 6 tokens, 0.00 mojibake ratio)",
                )],
                1.0,
            )],
            vec![],
        ),
        (
            parse_path("notes.pdf").unwrap(),
            vec![dropped(0), dropped(1), dropped(2)],
            vec![],
        ),
        (
            parse_path("ru.pdf").unwrap(),
            vec![(0, vec![], 1.0)],
            vec![],
        ),
        (
            parse_path("scan.pdf").unwrap(),
            vec![(
                0,
                vec![(
                    NoTextLayer,
                    Level::Loss,
                    "scanned page without a text layer (0 glyphs, 0.24 image area, 0.00 garbled, 0 tokens, 0.00 mojibake ratio)",
                )],
                0.0,
            )],
            vec![(OcrUnavailable, Level::Info)],
        ),
        (
            parse_bytes(truncated_bytes(), &ParseOptions::default()).unwrap(),
            moji.clone(),
            vec![(Truncated, Level::Warn), (OcrUnavailable, Level::Info)],
        ),
        (
            parse_bytes(mojibake_bytes(), &ParseOptions::default()).unwrap(),
            moji,
            vec![(OcrUnavailable, Level::Info)],
        ),
    ];
    for (i, (d, pages, doc)) in want.into_iter().enumerate() {
        let got: Vec<Page> = d
            .sections
            .iter()
            .map(|s| {
                let w = s.warnings.iter();
                (
                    s.index,
                    w.map(|w| (w.kind, w.level, w.message.as_str())).collect(),
                    s.quality,
                )
            })
            .collect();
        assert_eq!(got, pages, "input {i}");
        assert_eq!(
            d.warnings
                .iter()
                .map(|w| (w.kind, w.level))
                .collect::<Vec<_>>(),
            doc,
            "input {i}"
        );
    }
}

// Need-OCR classifier switched on (`need_ocr_on`).
fn parse_on(name: &str) -> Document {
    parse_with(&need_ocr_on(), &fixture(name)).unwrap()
}
#[test]
fn on_scan_is_no_text_layer_loss() {
    let d = parse_on("scan.pdf");
    assert!(has(&d, Some(0), WarningKind::NoTextLayer, Level::Loss));
    assert!(has(&d, None, WarningKind::OcrUnavailable, Level::Info));
    assert!(d.sections[0].blocks.is_empty());
    assert_eq!((d.sections[0].quality, d.quality), (0.0, 0.0));
}
/// `imgtext.pdf` has 49 glyph-like components in its picture: below `g_abs` 60, at least
/// `image_text_min_cc` 20 (ruling В5-4). Without `ocr` there is no raster, so no hint.
#[test]
fn on_image_text_page_keeps_its_layer() {
    let d = parse_on("imgtext.pdf");
    let page: Vec<_> = levels(&d).into_iter().filter(|w| w.0.is_some()).collect();
    #[cfg(feature = "ocr")]
    assert_eq!(
        page,
        vec![(Some(0), WarningKind::ImageTextPage, Level::Info)]
    );
    #[cfg(not(feature = "ocr"))]
    assert_eq!(page, vec![]);
    assert!(
        matches!(&d.sections[0].blocks[0], Block::Heading { level: 1, text, .. } if text == "Отчёт")
    );
}
#[test]
fn on_mojibake_and_truncated_are_garbage() {
    let c = need_ocr_on();
    let parse = |b| {
        PdfParser::with_config(c.clone())
            .parse(Input::Bytes(b), Format::Pdf, &ParseOptions::default())
            .unwrap()
    };
    for d in [parse(truncated_bytes()), parse(mojibake_bytes())] {
        assert!(has(&d, Some(0), WarningKind::SuspectedGarbage, Level::Loss));
    }
}
#[test]
fn on_blank_page_and_document() {
    let d = parse_on("empty.pdf");
    assert!(has(&d, Some(0), WarningKind::EmptyOutput, Level::Info));
    assert!(has(&d, None, WarningKind::EmptyOutput, Level::Warn));
}
#[test]
fn on_clean_files_stay_clean() {
    assert!(no_warn_or_loss(&parse_on("ru.pdf")));
    let d = parse_on("notes.pdf");
    assert!(no_warn_or_loss(&d) && has(&d, Some(0), WarningKind::ContentDropped, Level::Info));
    assert_eq!(
        format!("{:?}", parse_on("ru.pdf").sections[0].blocks),
        format!("{:?}", parse_path("ru.pdf").unwrap().sections[0].blocks),
        "the switch changes diagnostics, not blocks"
    );
}

/// The failure mode of a model-less build: an engine is set but its models are missing. The scan
/// stays empty, and a document `Warn` (shown by the CLI) says why and how to fix it, also under
/// `--ocr auto`; exit semantics stay those of a successful parse.
#[cfg(feature = "ocr")]
#[test]
fn engine_without_models_warns_with_the_fix() {
    use plaindoc_formats::ocr::OcrConfig;
    use plaindoc_ocr::{Image, OcrEngine, OcrError, Recognized, TextBox};
    struct NoModels;
    impl OcrEngine for NoModels {
        fn detect(&self, _: &Image) -> Result<Vec<TextBox>, OcrError> {
            Err(OcrError::ModelsUnavailable(
                "OCR models are missing: pass --ocr-models DIR or set $PLAINDOC_OCR_MODELS".into(),
            ))
        }
        fn recognize(&self, _: &Image, _: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
            unreachable!()
        }
    }
    let mut c = PdfConfig::default();
    c.ocr = OcrConfig::with_engine(std::sync::Arc::new(NoModels));
    let d = parse_with(&c, &fixture("scan.pdf")).unwrap();
    assert!(d.sections[0].blocks.is_empty());
    assert!(has(&d, Some(0), WarningKind::NoTextLayer, Level::Loss));
    let w = d
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::OcrUnavailable)
        .unwrap();
    assert_eq!(w.level, Level::Warn);
    assert!(w.message.contains("1 page(s) need OCR"), "{}", w.message);
    assert!(
        w.message.contains("OCR models are missing"),
        "{}",
        w.message
    );
    assert!(w.message.contains("--ocr-models"), "{}", w.message);
}
