use plaindoc_core::{
    Block, Document, Format, Input, ParseOptions, Parser, Result, Section, SectionKind,
};

use super::{Decoded, decode, read_text};

/// Plain text, Markdown, xml and source code, kept as is (D8).
pub struct PlainTextParser;

impl Parser for PlainTextParser {
    fn formats(&self) -> &[Format] {
        &[Format::Txt, Format::Md, Format::Xml]
    }

    fn parse(&self, input: Input, format: Format, opts: &ParseOptions) -> Result<Document> {
        let ext = input.extension();
        let bytes = read_text(input, opts)?;
        Ok(plain_document(decode(&bytes), ext.as_deref(), format))
    }
}

/// One `Body` section from decoded text: Markdown as a paragraph, anything else
/// as a code block (D8).
pub(crate) fn plain_document(decoded: Decoded, ext: Option<&str>, format: Format) -> Document {
    let mut section = Section::new(SectionKind::Body, 0);
    section.warnings = decoded.warnings;
    let text = decoded.text.trim_end();
    if !text.trim().is_empty() {
        section.blocks.push(if format == Format::Md {
            Block::paragraph(text.to_string(), None)
        } else {
            Block::Code {
                lang: code_language(ext, text).map(str::to_string),
                text: format!("{text}\n"),
                bbox: None,
            }
        });
    }
    let mut doc = Document::new(vec![section]);
    doc.metadata.format = Some(format);
    doc
}

/// Code block language: extension table first, then content heuristics.
pub fn code_language(ext: Option<&str>, text: &str) -> Option<&'static str> {
    let by_ext = match ext.unwrap_or_default() {
        "json" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "xml" => Some("xml"),
        "toml" => Some("toml"),
        "ini" | "cfg" | "conf" => Some("ini"),
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" | "mjs" => Some("javascript"),
        "ts" => Some("typescript"),
        "sh" | "bash" => Some("bash"),
        "sql" => Some("sql"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "hpp" => Some("cpp"),
        "java" => Some("java"),
        "go" => Some("go"),
        "rb" => Some("ruby"),
        _ => None,
    };
    by_ext.or_else(|| sniff_language(text))
}

fn sniff_language(text: &str) -> Option<&'static str> {
    if let Some(rest) = text.strip_prefix("#!") {
        let line = rest.lines().next().unwrap_or_default();
        return if line.contains("python") {
            Some("python")
        } else if line.contains("node") {
            Some("javascript")
        } else if line.contains("sh") {
            Some("bash")
        } else {
            None
        };
    }
    if text.trim_start().starts_with("<?xml") {
        return Some("xml");
    }
    let t = text.trim();
    if (t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']')) {
        return Some("json");
    }
    None
}
