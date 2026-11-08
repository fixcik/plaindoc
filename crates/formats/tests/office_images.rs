#![cfg(feature = "office")]
//! Pictures of DOCX/PPTX/XLSX: extracted byte for byte, numbered, linked from Markdown.
mod pictures_util;
use office_oxide::DocumentFormat;
use office_oxide::create::create_from_ir_to_writer;
use office_oxide::ir::{DocumentIR, Element, Image, Metadata, Paragraph, Section, TextSpan};
use pictures_util::*;
use plaindoc_core::*;

const NS: &str = concat!(
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
    r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" "#,
    r#"xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office""#,
);

fn drawing(blip: &str, descr: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:inline><wp:extent cx="9525" cy="9525"/><wp:docPr id="1" name="Picture 1" descr="{descr}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="p"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip {blip}/></pic:blipFill><pic:spPr/></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

fn para(inner: &str) -> String {
    format!("<w:p>{inner}</w:p>")
}

fn text(t: &str) -> String {
    para(&format!("<w:r><w:t>{t}</w:t></w:r>"))
}

/// A hand-written docx: DrawingML picture, VML picture, linked picture, EMF, the first picture
/// again.
fn docx() -> Vec<u8> {
    let body = [
        text("Before"),
        para(&drawing(r#"r:embed="rId5""#, "Схема")),
        para(r#"<w:r><w:pict><v:shape style="width:10pt;height:10pt"><v:imagedata r:id="rId6" o:title="vml"/></v:shape></w:pict></w:r>"#),
        para(&drawing(r#"r:link="rId7""#, "")),
        para(&drawing(r#"r:embed="rId8""#, "")),
        para(&drawing(r#"r:embed="rId5""#, "")),
        text("After"),
    ]
    .concat();
    docx_of(&body)
}

/// A docx with `body` and the pictures `rId5` (red), `rId6` (blue), a link and an EMF.
fn docx_of(body: &str) -> Vec<u8> {
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document {NS}><w:body>{body}</w:body></w:document>"#
    );
    let rel = |id: &str, target: &str, ext: bool| {
        format!(
            r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{target}"{}/>"#,
            if ext { r#" TargetMode="External""# } else { "" }
        )
    };
    let rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}{}{}{}</Relationships>"#,
        rel("rId5", "media/image1.png", false),
        rel("rId6", "media/image2.png", false),
        rel("rId7", "https://example.org/linked.png", true),
        rel("rId8", "media/image3.emf", false),
    );
    let emf = emf();
    zip_of(&[
        (
            "[Content_Types].xml",
            br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="emf" ContentType="image/x-emf"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/media/image1.png", RED),
        ("word/media/image2.png", BLUE),
        ("word/media/image3.emf", &emf),
    ])
}

#[test]
fn docx_pictures_are_extracted_in_order_and_deduplicated() {
    let d = parse(docx(), &ParseOptions::default());
    let p = pictures(&d);
    assert_eq!(p.len(), 5, "{p:?}");
    let info = |i: usize| p[i].1.clone().unwrap();
    assert_eq!(info(0).file.as_deref(), Some("image1.png"));
    assert_eq!(info(0).source.as_deref(), Some("word/media/image1.png"));
    assert_eq!(info(0).mime, Some("image/png"));
    assert_eq!(info(0).size, Some(RED.len() as u64));
    assert_eq!(info(1).file.as_deref(), Some("image2.png"));
    assert_eq!(info(1).source.as_deref(), Some("word/media/image2.png"));
    assert_eq!(
        info(2).url.as_deref(),
        Some("https://example.org/linked.png")
    );
    assert_eq!(info(2).file, None);
    assert_eq!(info(3).file.as_deref(), Some("image3.emf"));
    assert_eq!(info(3).mime, Some("image/x-emf"));
    assert_eq!(info(4).file.as_deref(), Some("image1.png"));
    assert_eq!(stored(&d, "image1.png"), RED);
    assert_eq!(stored(&d, "image2.png"), BLUE);
    assert_eq!(d.images.len(), 3);
    let md = d.to_markdown();
    assert!(
        md.starts_with("Before\n\n![Схема](image1.png)\n\n![изображение 2](image2.png)\n\n"),
        "{md}"
    );
    assert!(
        md.contains("![изображение](https://example.org/linked.png)"),
        "{md}"
    );
    assert!(!md.contains("base64"), "{md}");
    let json = d.to_json();
    assert!(
        json.contains("\"source\": \"word/media/image1.png\""),
        "{json}"
    );
    assert!(
        d.warnings
            .iter()
            .any(|w| w.message.contains("image not saved: 1 linked")),
        "{:?}",
        d.warnings
    );
}

#[test]
fn docx_pictures_off_gives_bare_marks() {
    let d = parse(docx(), &no_images());
    assert!(pictures(&d).iter().all(|(_, i)| i.is_none()));
    assert!(d.images.is_empty());
    assert!(!d.to_json().contains("\"image\":"));
    assert!(d.to_markdown().contains("<!-- image: Схема -->"));
}

#[test]
fn limits_keep_the_mark_and_warn() {
    let mut o = ParseOptions::default();
    o.images.max_count = 1;
    let d = parse(docx(), &o);
    assert_eq!(
        files(&d),
        [
            Some("image1.png".into()),
            None,
            None,
            None,
            Some("image1.png".into())
        ]
    );
    let md = d.to_markdown();
    assert!(md.contains("<!-- image -->"), "{md}");
    assert!(
        md.contains(
            "<!-- warning(warn) ContentDropped: image skipped: more than 1 images (2 images) -->"
        ),
        "{md}"
    );
}

fn ir(format: DocumentFormat, sections: Vec<Section>) -> Vec<u8> {
    let doc = DocumentIR {
        metadata: Metadata {
            format,
            ..Default::default()
        },
        sections,
    };
    let mut buf = std::io::Cursor::new(Vec::new());
    create_from_ir_to_writer(&doc, format, &mut buf).unwrap();
    buf.into_inner()
}

fn ir_para(t: &str) -> Element {
    Element::Paragraph(Paragraph {
        content: vec![office_oxide::ir::InlineContent::Text(TextSpan::plain(t))],
        ..Default::default()
    })
}

fn ir_image(data: &[u8], alt: &str) -> Element {
    Element::Image(Image {
        data: Some(data.to_vec()),
        format: Some(office_oxide::ir::ImageFormat::Png),
        alt_text: Some(alt.into()),
        display_width_emu: Some(9525),
        display_height_emu: Some(9525),
        ..Default::default()
    })
}

fn sec(title: &str, elements: Vec<Element>) -> Section {
    Section {
        title: Some(title.into()),
        elements,
        ..Default::default()
    }
}

#[test]
fn pptx_pictures_stay_on_their_slide() {
    let bytes = ir(
        DocumentFormat::Pptx,
        vec![
            sec("One", vec![ir_para("Slide one"), ir_image(RED, "red")]),
            sec("Two", vec![ir_para("Slide two"), ir_image(BLUE, "blue")]),
        ],
    );
    let d = parse(bytes, &ParseOptions::default());
    let per_slide: Vec<Vec<Option<String>>> = d
        .sections
        .iter()
        .filter(|s| s.kind == SectionKind::Slide)
        .map(|s| {
            s.blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Image { image, .. } => Some(image.as_ref().and_then(|i| i.file.clone())),
                    _ => None,
                })
                .collect()
        })
        .collect();
    assert_eq!(
        per_slide,
        [
            vec![Some("image1.png".to_string())],
            vec![Some("image2.png".to_string())]
        ]
    );
    assert_eq!(stored(&d, "image2.png"), BLUE);
    let src = pictures(&d)[0].1.clone().unwrap().source.unwrap();
    assert!(src.starts_with("ppt/media/"), "{src}");
}

#[test]
fn xlsx_pictures_follow_the_sheet_table() {
    use office_oxide::ir::{Table, TableCell, TableRow};
    let cell = |t: &str| TableCell {
        content: vec![ir_para(t)],
        col_span: 1,
        row_span: 1,
        ..Default::default()
    };
    let table = Element::Table(Table {
        rows: vec![TableRow {
            cells: vec![cell("a"), cell("b")],
            ..Default::default()
        }],
        ..Default::default()
    });
    let bytes = ir(
        DocumentFormat::Xlsx,
        vec![sec("Data", vec![ir_image(RED, "chart"), table])],
    );
    let d = parse(bytes, &ParseOptions::default());
    let s = &d.sections[0];
    let kinds: Vec<&str> = s
        .blocks
        .iter()
        .map(|b| match b {
            Block::Table { .. } => "table",
            Block::Image { .. } => "image",
            _ => "other",
        })
        .collect();
    if kinds.contains(&"image") {
        assert_eq!(kinds.last(), Some(&"image"), "{kinds:?}");
        assert_eq!(stored(&d, "image1.png"), RED);
    } else {
        panic!("the xlsx writer dropped the picture: {kinds:?}");
    }
}

/// A docx whose header holds a logo and whose body refers to one picture `refs` times.
fn docx_header_and_refs(refs: usize) -> Vec<u8> {
    let body = format!(
        r#"{}{}<w:sectPr><w:headerReference w:type="default" r:id="rId9"/></w:sectPr>"#,
        text("Body"),
        para(&drawing(r#"r:embed="rId5""#, "")).repeat(refs)
    );
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document {NS}><w:body>{body}</w:body></w:document>"#
    );
    let hdr = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:hdr {NS}>{}{}</w:hdr>"#,
        text("Head"),
        para(&drawing(r#"r:embed="rId1""#, "Логотип"))
    );
    let rel = |id: &str, ty: &str, target: &str| {
        format!(
            r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{ty}" Target="{target}"/>"#
        )
    };
    let rels = |r: String| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{r}</Relationships>"#
        )
    };
    let doc_rels =
        rels(rel("rId5", "image", "media/image1.png") + &rel("rId9", "header", "header1.xml"));
    let hdr_rels = rels(rel("rId1", "image", "media/logo.png"));
    zip_of(&[
        (
            "[Content_Types].xml",
            br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/header1.xml", hdr.as_bytes()),
        ("word/_rels/header1.xml.rels", hdr_rels.as_bytes()),
        ("word/media/image1.png", RED),
        ("word/media/logo.png", BLUE),
    ])
}

#[test]
fn docx_header_picture_is_extracted_after_the_body() {
    let d = parse(docx_header_and_refs(1), &ParseOptions::default());
    let p = pictures(&d);
    assert_eq!(p.len(), 2, "{p:?}");
    assert_eq!(p[1].0.as_deref(), Some("Логотип"));
    let logo = p[1].1.clone().unwrap();
    assert_eq!(logo.file.as_deref(), Some("image2.png"));
    assert_eq!(logo.source.as_deref(), Some("word/media/logo.png"));
    assert_eq!(stored(&d, "image2.png"), BLUE);
    assert!(
        d.to_markdown().ends_with("![Логотип](image2.png)\n"),
        "{}",
        d.to_markdown()
    );
    // off: as before, no picture blocks from the header
    assert_eq!(
        pictures(&parse(docx_header_and_refs(1), &no_images())).len(),
        1
    );
}

#[test]
fn many_references_to_one_picture_give_one_file() {
    let d = parse(docx_header_and_refs(300), &ParseOptions::default());
    let f = files(&d);
    assert_eq!(f.len(), 301);
    assert!(f[..300].iter().all(|x| x.as_deref() == Some("image1.png")));
    assert_eq!(d.images.len(), 2);
}

/// `wp:docPr` with a name only (as Word writes it without alt text).
fn nameless_drawing(rid: &str) -> String {
    drawing(&format!(r#"r:embed="{rid}""#), "").replace(r#" descr="""#, "")
}

#[test]
fn docx_alt_is_the_author_descr_never_the_auto_name() {
    let body = [
        para(&drawing(r#"r:embed="rId5""#, "Вид &amp; [сбоку]")),
        para(&nameless_drawing("rId6")),
        text("Рисунок 2 - Вид сверху"),
        text("Обычный текст"),
        text("Схема 3"),
        para(&nameless_drawing("rId5")),
        text("Просто текст"),
    ]
    .concat();
    let d = parse(docx_of(&body), &ParseOptions::default());
    let alts: Vec<Option<String>> = pictures(&d).into_iter().map(|(a, _)| a).collect();
    // `name="Picture 1"` of wp:docPr is not exposed as alt; no descr: no alt
    assert_eq!(alts[0].as_deref(), Some("Вид & [сбоку]"));
    assert!(alts[1].as_deref().is_none_or(str::is_empty), "{alts:?}");
    let mut o = MarkdownOptions::default();
    o.image_dir = Some("img-1".into());
    let md = d.to_markdown_with(&o);
    assert!(
        md.starts_with("![Вид & \\[сбоку\\]](img-1/image1.png)\n\n![Рисунок 2 - Вид сверху](img-1/image2.png)\n\nРисунок 2 - Вид сверху\n\nОбычный текст\n\nСхема 3\n\n![Схема 3](img-1/image1.png)\n\nПросто текст\n"),
        "{md}"
    );
}

#[test]
fn pptx_alt_is_the_descr_of_the_picture() {
    let bytes = ir(
        DocumentFormat::Pptx,
        vec![sec("One", vec![ir_image(RED, "Круговая диаграмма")])],
    );
    let d = parse(bytes, &ParseOptions::default());
    assert_eq!(pictures(&d)[0].0.as_deref(), Some("Круговая диаграмма"));
}
