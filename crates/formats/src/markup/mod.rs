//! Feature `markup` (D6, D8): HTML into structured blocks; XML Spreadsheet 2003
//! into sheets (К-25); any other XML is passed through as in the `text` parser.

use plaindoc_core::{Document, Format, Input, ParseOptions, Parser, Result};

mod charset;
mod guard;
mod html;
mod ssml;

use plaindoc_core::{Section, SectionKind};

use crate::common::flag_empty;
use crate::text::{decode_with, plain_document, read_text};

/// HTML (also HTML saved as `.xls`/`.xlsx`, D6) into one `Body` section.
pub struct HtmlParser;

impl Parser for HtmlParser {
    fn formats(&self) -> &[Format] {
        &[Format::Html]
    }

    fn parse(&self, input: Input, _format: Format, opts: &ParseOptions) -> Result<Document> {
        let bytes = read_text(input, opts)?;
        let declared = charset::declared_encoding(&bytes);
        let decoded = decode_with(&bytes, declared);
        drop(bytes);
        guard::check_nesting(&decoded.text)?;
        let pics = opts.images.enabled.then_some(opts.images.max_bytes);
        let conv = html::convert(&decoded.text, pics)?;
        let mut section = Section::new(SectionKind::Body, 0);
        section.blocks = conv.blocks;
        section.warnings = decoded.warnings;
        section.warnings.extend(conv.warnings);
        let mut doc = Document::new(vec![section]);
        doc.metadata.title = conv.title;
        doc.metadata.author = conv.author;
        flag_empty(&mut doc, "html");
        Ok(doc)
    }
}

/// XML: XML Spreadsheet 2003 becomes sheets (К-25), anything else stays one code block (D8).
pub struct XmlParser;

impl Parser for XmlParser {
    fn formats(&self) -> &[Format] {
        &[Format::Xml]
    }

    fn parse(&self, input: Input, format: Format, opts: &ParseOptions) -> Result<Document> {
        let ext = input.extension();
        let bytes = read_text(input, opts)?;
        let declared = charset::declared_encoding(&bytes);
        let decoded = decode_with(&bytes, declared);
        drop(bytes);
        if ssml::is_spreadsheet(&decoded.text)
            && let Some(mut doc) = ssml::parse(&decoded.text, opts)?
        {
            doc.warnings.splice(0..0, decoded.warnings);
            return Ok(doc);
        }
        Ok(plain_document(decoded, ext.as_deref(), format))
    }
}
