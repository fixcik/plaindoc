use plaindoc_core::{Format, ParseError, classify_cfb_streams, detect};

const CFB: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0];

#[test]
fn pdf_magic_anywhere_in_first_1024_bytes() {
    assert_eq!(detect(b"%PDF-1.7\n", None).unwrap(), Format::Pdf);
    let mut junk = vec![b' '; 1000];
    junk.extend_from_slice(b"%PDF-1.4");
    assert_eq!(detect(&junk, Some("bin")).unwrap(), Format::Pdf);
    let mut late = vec![b' '; 1100];
    late.extend_from_slice(b"%PDF-1.4");
    assert!(detect(&late, Some("pdf")).is_err());
}

#[test]
fn zip_containers() {
    let ooxml = |part: &[u8]| [b"PK\x03\x04....[Content_Types].xml....".as_slice(), part].concat();
    assert_eq!(
        detect(&ooxml(b"word/document.xml"), None).unwrap(),
        Format::Docx
    );
    assert_eq!(
        detect(&ooxml(b"xl/workbook.xml"), None).unwrap(),
        Format::Xlsx
    );
    assert_eq!(
        detect(&ooxml(b"ppt/presentation.xml"), None).unwrap(),
        Format::Pptx
    );
    assert_eq!(
        detect(&ooxml(b"docProps/app.xml"), Some("xlsx")).unwrap(),
        Format::Xlsx
    );
    let odt = b"PK\x03\x04..........mimetypeapplication/vnd.oasis.opendocument.text";
    assert_eq!(detect(odt, None).unwrap(), Format::Odt);
    let ods = b"PK\x03\x04..........mimetypeapplication/vnd.oasis.opendocument.spreadsheet";
    assert_eq!(detect(ods, Some("zip")).unwrap(), Format::Ods);
    assert_eq!(
        detect(b"PK\x03\x04docProps/thumbnail.jpeg", Some("docx")).unwrap(),
        Format::Docx
    );
    assert!(matches!(
        detect(b"PK\x03\x04plain.zip", Some("zip")),
        Err(ParseError::Unsupported(Format::Unknown))
    ));
}

#[test]
fn cfb_without_probe_uses_extension() {
    assert_eq!(detect(CFB, Some("doc")).unwrap(), Format::Doc);
    assert_eq!(detect(CFB, Some("XLS")).unwrap(), Format::Xls);
    assert_eq!(detect(CFB, Some("ppt")).unwrap(), Format::Ppt);
    assert!(matches!(
        detect(CFB, Some("docx")),
        Err(ParseError::Encrypted)
    ));
    assert!(matches!(
        detect(CFB, Some("msg")),
        Err(ParseError::Unsupported(Format::Msg))
    ));
    assert!(matches!(
        detect(CFB, None),
        Err(ParseError::Unsupported(Format::Unknown))
    ));
}

#[test]
fn cfb_stream_names() {
    assert_eq!(
        classify_cfb_streams(&["WordDocument", "1Table"]).unwrap(),
        Format::Doc
    );
    assert_eq!(classify_cfb_streams(&["Workbook"]).unwrap(), Format::Xls);
    assert_eq!(classify_cfb_streams(&["Book"]).unwrap(), Format::Xls);
    assert_eq!(
        classify_cfb_streams(&["PowerPoint Document"]).unwrap(),
        Format::Ppt
    );
    assert!(matches!(
        classify_cfb_streams(&["EncryptionInfo", "EncryptedPackage"]),
        Err(ParseError::Encrypted)
    ));
    assert!(matches!(
        classify_cfb_streams(&["__properties_version1.0"]),
        Err(ParseError::Unsupported(Format::Msg))
    ));
    assert!(matches!(
        classify_cfb_streams(&["Foo"]),
        Err(ParseError::Unsupported(Format::Unknown))
    ));
}

#[test]
fn images_and_rtf() {
    assert_eq!(detect(b"\x89PNG\r\n\x1a\n", None).unwrap(), Format::Png);
    assert_eq!(
        detect(&[0xFF, 0xD8, 0xFF, 0xE0], None).unwrap(),
        Format::Jpeg
    );
    assert_eq!(detect(b"II*\0", None).unwrap(), Format::Tiff);
    assert_eq!(detect(b"MM\0*", None).unwrap(), Format::Tiff);
    assert_eq!(detect(b"{\\rtf1\\ansi", Some("doc")).unwrap(), Format::Rtf);
}

#[test]
fn extension_is_only_a_hint_for_csv_tsv_md() {
    assert_eq!(detect(b"a,b\n1,2\n", Some("csv")).unwrap(), Format::Csv);
    assert_eq!(detect(b"a\tb\n", Some("tsv")).unwrap(), Format::Tsv);
    assert_eq!(detect(b"# T\n", Some("md")).unwrap(), Format::Md);
    assert_eq!(detect(b"# T\n", Some("txt")).unwrap(), Format::Txt);
    assert_eq!(detect(b"a,b\n", None).unwrap(), Format::Txt);
    assert_eq!(detect(b"", Some("txt")).unwrap(), Format::Txt);
}

#[test]
fn text_is_decided_by_content() {
    assert_eq!(detect(b"plain words", None).unwrap(), Format::Txt);
    assert_eq!(detect(b"fn main() {}", Some("rs")).unwrap(), Format::Txt);
    assert_eq!(detect(b"{\"a\": 1}", Some("json")).unwrap(), Format::Txt);
    assert_eq!(
        detect(b"2026-11-07 INFO started\n", Some("log")).unwrap(),
        Format::Txt
    );
    assert_eq!(
        detect(b"<?xml version=\"1.0\"?><a/>", Some("txt")).unwrap(),
        Format::Xml
    );
    assert_eq!(
        detect(b"  <!DOCTYPE html><html>", None).unwrap(),
        Format::Html
    );
    assert_eq!(detect(b"<html>", Some("htm")).unwrap(), Format::Html);
    assert_eq!(detect(&[0xFF, 0xFE, b'a', 0], None).unwrap(), Format::Txt);
}

#[test]
fn binary_is_not_text_whatever_the_extension() {
    let unknown = |r| matches!(r, Err(ParseError::Unsupported(Format::Unknown)));
    assert!(unknown(detect(&[1, 0, 2, 0], None)));
    assert!(unknown(detect(&[b'a', 0, b'b'], Some("txt"))));
    assert!(unknown(detect(&[1, 2, 3, 4, 5, 6, b'a'], Some("log"))));
}

#[test]
fn html_disguised_as_spreadsheet_is_html() {
    let html = b"<html><body><table><tr><td>1</td></tr></table></body></html>";
    assert_eq!(detect(html, Some("xls")).unwrap(), Format::Html);
    assert_eq!(
        detect(b"\xEF\xBB\xBF\r\n<TABLE border=1>", Some("xlsx")).unwrap(),
        Format::Html
    );
    assert!(matches!(
        detect(html, Some("doc")),
        Err(ParseError::Corrupt(_))
    ));
}

#[test]
fn known_extension_with_wrong_content_is_corrupt() {
    assert!(matches!(
        detect(b"not a pdf", Some("pdf")),
        Err(ParseError::Corrupt(_))
    ));
    assert!(matches!(
        detect(b"text", Some("docx")),
        Err(ParseError::Corrupt(_))
    ));
    assert!(matches!(
        detect(b"a;b", Some("xls")),
        Err(ParseError::Corrupt(_))
    ));
}

#[test]
fn xml_with_table_element_is_xml_but_xhtml_is_html() {
    let xml = b"<?xml version=\"1.0\"?><data><table/></data>";
    assert_eq!(detect(xml, Some("xml")).unwrap(), Format::Xml);
    let xhtml = b"<?xml version=\"1.0\"?><html><body/></html>";
    assert_eq!(detect(xhtml, None).unwrap(), Format::Html);
}

#[test]
fn truncated_inputs_do_not_panic() {
    assert_eq!(detect(b"", None).unwrap(), Format::Txt);
    assert_eq!(detect(b"P", None).unwrap(), Format::Txt);
    assert_eq!(detect(b"PK", None).unwrap(), Format::Txt);
    assert!(matches!(
        detect(b"PK", Some("docx")),
        Err(ParseError::Corrupt(_))
    ));
}

#[test]
fn text_extensions_beat_bare_html_sniff() {
    let readme = b"<p align=\"center\">logo</p>\n<table><tr><td>x</td></tr></table>\n# T\n";
    assert_eq!(detect(readme, Some("md")).unwrap(), Format::Md);
    assert_eq!(detect(readme, Some("txt")).unwrap(), Format::Txt);
    assert_eq!(
        detect(b"<table><tr><td>1</td></tr></table>", Some("csv")).unwrap(),
        Format::Csv
    );
    // Without a text hint the bare `<table` still means Html.
    assert_eq!(
        detect(b"<table><tr><td>1</td></tr></table>", None).unwrap(),
        Format::Html
    );
    assert_eq!(
        detect(b"<table><tr><td>1</td></tr></table>", Some("html")).unwrap(),
        Format::Html
    );
}

#[test]
fn md_with_real_html_document_is_html() {
    assert_eq!(
        detect(b"<!DOCTYPE html>\n<p>x</p>", Some("md")).unwrap(),
        Format::Html
    );
    assert_eq!(
        detect(b"<HTML><body></body></HTML>", Some("txt")).unwrap(),
        Format::Html
    );
}

#[test]
fn xml_spreadsheet_2003_with_xls_extension_is_xml() {
    let book = b"<?xml version=\"1.0\"?>\n<?mso-application progid=\"Excel.Sheet\"?>\n<Workbook xmlns=\"urn:schemas-microsoft-com:office:spreadsheet\"><Worksheet><Table><Row><Cell><Data>1</Data></Cell></Row></Table></Worksheet></Workbook>";
    assert_eq!(detect(book, Some("xls")).unwrap(), Format::Xml);
    assert_eq!(detect(book, Some("xlsx")).unwrap(), Format::Xml);
    assert_eq!(detect(book, None).unwrap(), Format::Xml);
    assert_eq!(
        detect(b"<?xml version=\"1.0\"?><Workbook/>", Some("xls")).unwrap(),
        Format::Xml
    );
    assert_eq!(
        detect(b"<?xml version=\"1.0\"?><html><body/></html>", Some("xls")).unwrap(),
        Format::Html
    );
}

#[test]
fn bmp_and_webp_by_validated_magic() {
    // 14-byte file header (size 58, reserved 0, offset 54) + 40-byte BITMAPINFOHEADER prefix.
    let mut bmp = b"BM\x3a\x00\x00\x00\x00\x00\x00\x00\x36\x00\x00\x00\x28\x00\x00\x00".to_vec();
    bmp.resize(58, 0);
    assert_eq!(detect(&bmp, None).unwrap(), Format::Bmp);
    assert_eq!(
        detect(b"RIFF\x24\x00\x00\x00WEBPVP8 ", None).unwrap(),
        Format::Webp
    );
    assert_eq!(Format::from_extension("WEBP"), Some(Format::Webp));
    // Text starting with "BM" stays text, with or without an extension (I2).
    let csv = b"BMW,2020,Germany\nAudi,2019,Germany\n";
    assert_eq!(detect(csv, Some("csv")).unwrap(), Format::Csv);
    assert_eq!(
        detect(b"BMI is a ratio of weight to height.\n", None).unwrap(),
        Format::Txt
    );
    // Binary "BM" with a bad header is not a BMP.
    let mut bad = bmp.clone();
    bad[14] = 7;
    assert!(detect(&bad, None).is_err());
}
