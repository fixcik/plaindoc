use plaindoc_core::*;

#[test]
fn format_names_and_extensions() {
    assert_eq!(Format::Docx.to_string(), "docx");
    assert_eq!(Format::Unknown.as_str(), "unknown");
    assert_eq!(Format::from_extension("MD"), Some(Format::Md));
    assert_eq!(Format::from_extension("markdown"), Some(Format::Md));
    assert_eq!(Format::from_extension("jpg"), Some(Format::Jpeg));
    assert_eq!(Format::from_extension("tab"), Some(Format::Tsv));
    assert_eq!(Format::from_extension("htm"), Some(Format::Html));
    assert_eq!(Format::from_extension("rs"), None);
}

#[test]
fn constructors_set_documented_defaults() {
    let s = Section::new(SectionKind::Body, 0);
    assert_eq!(s.quality, 1.0);
    assert_eq!(s.source, TextSource::TextLayer);
    assert!(s.blocks.is_empty() && s.warnings.is_empty() && s.name.is_none());

    let c = Cell::new("x");
    assert_eq!((c.colspan, c.rowspan), (1, 1));
    assert_eq!(c.blocks, vec![Block::paragraph("x", None)]);
    assert_eq!(c.text(), "x");
    assert!(c.is_simple() && c.value.is_none());
    assert!(Cell::new("").blocks.is_empty());
    assert!(!c.is_merged());
    assert!(Cell::new("y").with_span(2, 1).is_merged());
    assert_eq!(Cell::new("z").with_span(0, 0).colspan, 1);

    let d = Document::new(vec![s]);
    assert_eq!(d.quality, 1.0);
    assert_eq!(d.metadata, Metadata::default());
    assert_eq!(Document::default(), Document::new(vec![]));
    let v = CellValue::new(Some(ScalarValue::Number(1.5)));
    assert!(v.formula.is_none() && v.number_format.is_none());
    assert_eq!(ListItem::new("i").text(), "i");
}

#[test]
fn fields_are_mutable_from_another_crate() {
    let mut s = Section::new(SectionKind::Page, 2);
    s.name = Some("p".into());
    s.blocks
        .push(Block::paragraph("t", Some(BBox::new(1.0, 2.0, 3.0, 4.0))));
    let mut d = Document::new(vec![s]);
    d.metadata.title = Some("T".into());
    d.warnings
        .push(Warning::new(WarningKind::Truncated, Level::Warn, "m"));
    assert_eq!(d.sections[0].index, 2);
}

#[test]
fn parse_options_default_and_field_update() {
    let mut o = ParseOptions::default();
    assert_eq!(o.ocr, OcrMode::Auto);
    assert!(o.timeout.is_none() && o.max_size.is_none() && o.min_quality.is_none());
    assert!(!o.libreoffice_fallback && !o.page_parallel);
    assert_eq!(o.max_text_bytes, 64 * 1024 * 1024);
    o.ocr = OcrMode::Never;
    o.max_size = Some(10);
    let w = QualityWeights::default();
    assert_eq!((w.warn, w.loss), (0.1, 0.5));
    assert_eq!(o.quality, w);
}

#[test]
fn input_extension_is_lowercased() {
    assert_eq!(
        Input::Path("a/B.CSV".into()).extension().as_deref(),
        Some("csv")
    );
    assert_eq!(Input::Bytes(vec![]).extension(), None);
    assert_eq!(Input::Bytes(vec![1, 2]).into_bytes().unwrap(), vec![1, 2]);
}

#[test]
fn errors_display() {
    assert_eq!(
        ParseError::Unsupported(Format::Pdf).to_string(),
        "unsupported format: pdf"
    );
    assert_eq!(
        ParseError::Internal("boom".into()).to_string(),
        "internal error (parser panicked): boom"
    );
    let io: ParseError = std::io::Error::other("x").into();
    assert!(matches!(io, ParseError::Io(_)));
    let l = ParseError::Limit {
        kind: "expanded size".into(),
        limit: 5,
    };
    assert_eq!(l.to_string(), "limit exceeded: expanded size > 5");
}

#[test]
fn hidden_text_is_stripped_by_default() {
    assert_eq!(ParseOptions::default().hidden_text, HiddenText::Strip);
    assert_eq!(HiddenText::default(), HiddenText::Strip);
}
