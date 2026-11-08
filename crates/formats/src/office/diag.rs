//! Diagnostics: IR warnings, empty/low output, image-only sections.

use plaindoc_core::{Block, Document, Level, Section, Warning, WarningKind};

/// Inputs smaller than this are never reported as having little text.
pub(crate) const LOW_TEXT_MIN_INPUT_BYTES: u64 = 256 * 1024;
/// Fewer than one character per this many input bytes is little text.
pub(crate) const LOW_TEXT_BYTES_PER_CHAR: u64 = 1000;

/// Warnings from IR metadata: deduplicated `warnings` and `text_truncated`.
pub(crate) fn ir_warnings(meta: &office_oxide::ir::Metadata) -> Vec<Warning> {
    let mut lines: Vec<&str> = Vec::new();
    for s in &meta.warnings {
        if !lines.contains(&s.as_str()) {
            lines.push(s);
        }
    }
    let mut out = Vec::new();
    if meta.text_truncated {
        let pos = lines.iter().position(|s| {
            let l = s.to_lowercase();
            ["truncat", "limit", "cut"].iter().any(|k| l.contains(k))
        });
        let msg = match pos {
            Some(i) => lines.remove(i).to_string(),
            None => "office: text is incomplete (text_truncated)".to_string(),
        };
        out.push(Warning::new(WarningKind::Truncated, Level::Loss, msg));
    }
    out.extend(
        lines
            .into_iter()
            .map(|s| Warning::new(WarningKind::PartialContent, Level::Warn, s)),
    );
    out
}

/// Characters of all text in `blocks`, including cell and list-item blocks and OCR text of images; `alt` is not counted.
pub(crate) fn text_chars(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|b| match b {
            Block::Heading { text, .. }
            | Block::Paragraph { text, .. }
            | Block::Code { text, .. }
            | Block::Note { text, .. } => text.chars().count(),
            Block::Image { ocr_text, .. } => ocr_text.as_deref().map_or(0, |t| t.chars().count()),
            Block::List { items, .. } => items.iter().map(|i| text_chars(&i.blocks)).sum(),
            Block::Table { rows, .. } => rows.iter().flatten().map(|c| text_chars(&c.blocks)).sum(),
            _ => 0,
        })
        .sum()
}

/// Empty or suspiciously small output for an input of `input_len` bytes.
pub(crate) fn output_warnings(doc: &Document, input_len: u64) -> Vec<Warning> {
    let chars: usize = doc.sections.iter().map(|s| text_chars(&s.blocks)).sum();
    let mut out = Vec::new();
    if chars == 0 {
        out.push(Warning::new(
            WarningKind::EmptyOutput,
            Level::Warn,
            "office: no extractable text",
        ));
    }
    if input_len >= LOW_TEXT_MIN_INPUT_BYTES && (chars as u64) < input_len / LOW_TEXT_BYTES_PER_CHAR
    {
        out.push(Warning::new(
            WarningKind::PartialContent,
            Level::Info,
            format!("office: little text ({chars} chars) for {input_len} bytes"),
        ));
    }
    out
}

/// A section with images but no text.
pub(crate) fn section_warnings(s: &Section) -> Vec<Warning> {
    let has_image = s.blocks.iter().any(|b| matches!(b, Block::Image { .. }));
    if has_image && text_chars(&s.blocks) == 0 {
        vec![Warning::new(
            WarningKind::ImageTextPage,
            Level::Warn,
            "office: section has images but no text",
        )]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plaindoc_core::{Cell, ListItem, SectionKind};

    #[test]
    fn truncation_dedup_and_partial() {
        let mut m = office_oxide::ir::Metadata {
            text_truncated: true,
            ..Default::default()
        };
        let cut = "[output truncated: the document's text exceeds the 20 character budget (office_oxide::limits::set_max_text_chars)]";
        m.warnings = vec![cut.into(), cut.into(), "header counts disagree".into()];
        let w = ir_warnings(&m);
        assert_eq!(w.len(), 2);
        assert_eq!(
            (w[0].kind, w[0].level, w[0].message.as_str()),
            (WarningKind::Truncated, Level::Loss, cut)
        );
        assert_eq!(
            (w[1].kind, w[1].level),
            (WarningKind::PartialContent, Level::Warn)
        );
    }

    #[test]
    fn xls_style_truncation_without_text_still_reports() {
        let m = office_oxide::ir::Metadata {
            text_truncated: true,
            ..Default::default()
        };
        assert!(matches!(&ir_warnings(&m)[..], [w] if w.kind == WarningKind::Truncated));
    }

    #[test]
    fn empty_document_warns_empty_output() {
        let w = output_warnings(
            &Document::new(vec![Section::new(SectionKind::Slide, 0)]),
            40_000,
        );
        assert!(
            matches!(&w[..], [w] if (w.kind, w.level) == (WarningKind::EmptyOutput, Level::Warn))
        );
    }

    #[test]
    fn little_text_for_large_input_is_info() {
        let mut s = Section::new(SectionKind::Body, 0);
        s.blocks.push(Block::paragraph("abc", None));
        let doc = Document::new(vec![s]);
        assert!(output_warnings(&doc, LOW_TEXT_MIN_INPUT_BYTES - 1).is_empty());
        let w = output_warnings(&doc, LOW_TEXT_MIN_INPUT_BYTES);
        assert!(
            matches!(&w[..], [w] if (w.kind, w.level) == (WarningKind::PartialContent, Level::Info))
        );
    }

    #[test]
    fn image_only_section_warns() {
        let mut s = Section::new(SectionKind::Slide, 0);
        s.blocks.push(Block::Image {
            alt: Some("logo".into()),
            ocr_text: None,
            bbox: None,
            image: None,
        });
        assert!(matches!(&section_warnings(&s)[..], [w] if w.kind == WarningKind::ImageTextPage));
        s.blocks.push(Block::paragraph("x", None));
        assert!(section_warnings(&s).is_empty());
    }

    #[test]
    fn text_chars_counts_nested_cell_and_list_blocks() {
        let mut c = Cell::new("ab");
        c.blocks.push(Block::List {
            ordered: false,
            items: vec![ListItem::new("cd")],
            bbox: None,
        });
        assert_eq!(
            text_chars(&[Block::Table {
                rows: vec![vec![c]],
                header_rows: 0,
                bbox: None
            }]),
            4
        );
    }
}
