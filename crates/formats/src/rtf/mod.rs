//! Feature `rtf`: a small RTF reader of our own (tokenizer + interpreter).

mod charset;
mod doc;
mod lexer;

use plaindoc_core::{Document, Format, Input, ParseOptions, Parser, Result, Section, SectionKind};

use crate::common::flag_empty;
use crate::read::read_limited;

/// Group nesting beyond this is `Limit { "rtf group depth" }`.
pub(crate) const MAX_GROUP_DEPTH: usize = 1024;
/// Own input cap of RTF (ruling 6); `max_size` can only lower it.
pub(crate) const MAX_RTF_BYTES: u64 = 256 << 20;

fn rtf_limit(opts: &ParseOptions) -> u64 {
    opts.max_size
        .map_or(MAX_RTF_BYTES, |m| m.min(MAX_RTF_BYTES))
}

/// RTF into one `Body` section.
pub struct RtfParser;

impl Parser for RtfParser {
    fn formats(&self) -> &[Format] {
        &[Format::Rtf]
    }

    fn parse(&self, input: Input, _format: Format, opts: &ParseOptions) -> Result<Document> {
        let bytes = read_limited(input, rtf_limit(opts))?;
        let pics = opts.images.enabled.then_some(opts.images.max_bytes);
        let parsed = doc::parse(&bytes, pics)?;
        let mut section = Section::new(SectionKind::Body, 0);
        section.blocks = parsed.blocks;
        section.warnings = parsed.warnings;
        let mut doc = Document::new(vec![section]);
        doc.metadata.title = parsed.title;
        doc.metadata.author = parsed.author;
        flag_empty(&mut doc, "rtf");
        Ok(doc)
    }
}
