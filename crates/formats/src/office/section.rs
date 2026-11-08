//! Sections and the whole document: headers/footers, speaker notes, `DocumentIR` -> `Document`.

use office_oxide::DocumentFormat;
use office_oxide::ir::{DocumentIR, Element, HeaderFooter, Section};
use plaindoc_core::{
    Block, Document, Level, NoteKind, Section as CoreSection, SectionKind, Warning, WarningKind,
};

use super::blocks::{MapCtx, map_elements};
use super::pictures::Pictures;
use super::text::elements_text;

fn notes(kind: NoteKind, variants: [&Option<HeaderFooter>; 3]) -> Vec<Block> {
    let mut seen: Vec<String> = Vec::new();
    for hf in variants.into_iter().flatten() {
        let text = elements_text(&hf.content);
        if !text.trim().is_empty() && !seen.contains(&text) {
            seen.push(text);
        }
    }
    seen.into_iter()
        .map(|text| Block::Note {
            kind,
            text,
            bbox: None,
        })
        .collect()
}

/// `(headers, footers)`: default, first-page and even-page variants, empty and repeated texts dropped.
pub(crate) fn header_footer_notes(sec: &Section) -> (Vec<Block>, Vec<Block>) {
    (
        notes(
            NoteKind::Header,
            [&sec.header, &sec.first_page_header, &sec.even_page_header],
        ),
        notes(
            NoteKind::Footer,
            [&sec.footer, &sec.first_page_footer, &sec.even_page_footer],
        ),
    )
}

/// Speaker notes of a slide, if any text.
pub(crate) fn speaker_notes(sec: &Section) -> Option<Block> {
    let text = elements_text(sec.speaker_notes.as_deref()?);
    (!text.trim().is_empty()).then_some(Block::Note {
        kind: NoteKind::SpeakerNotes,
        text,
        bbox: None,
    })
}

/// Kind of the sections of a document of the given format.
pub(crate) fn section_kind(f: DocumentFormat) -> SectionKind {
    match f {
        DocumentFormat::Xlsx | DocumentFormat::Xls => SectionKind::Sheet,
        DocumentFormat::Pptx | DocumentFormat::Ppt => SectionKind::Slide,
        _ => SectionKind::Body,
    }
}

/// Noun for a section of `kind` in warnings: "slide" for slides, "sheet" otherwise.
fn page_noun(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::Slide => "slide",
        _ => "sheet",
    }
}

/// Maps one IR section: headers, body, footers, speaker notes.
pub(crate) fn map_section(
    sec: &Section,
    kind: SectionKind,
    index: usize,
    ctx: &mut MapCtx,
) -> CoreSection {
    let mut out = CoreSection::new(kind, index);
    // In docx/doc the title is derived from the first heading and is already in the blocks.
    if kind != SectionKind::Body {
        out.name = sec.title.clone();
    }
    let (headers, footers) = header_footer_notes(sec);
    out.blocks.extend(headers);
    let mut els: &[Element] = &sec.elements;
    let mut body = Vec::new();
    if std::mem::take(&mut ctx.find_title)
        && let Some((n, title)) = super::title::leading_title(els, &mut ctx.notes)
    {
        body.push(Block::heading(1, title, None));
        els = &els[n..];
    }
    body.extend(map_elements(els, ctx));
    if ctx.pics.enabled && kind == SectionKind::Sheet {
        // office_oxide puts the drawing's pictures before the cells: show them after the table.
        let (pics, rest): (Vec<Block>, Vec<Block>) = body
            .into_iter()
            .partition(|b| matches!(b, Block::Image { .. }));
        body = rest;
        body.extend(pics);
    }
    out.blocks.extend(body);
    out.blocks.extend(footers);
    // Header/footer pictures (logos, seals) after the body: the office OCR pairs the section's
    // image blocks with its body images in order, so they must not come first.
    let hf: Vec<&[Element]> = [
        &sec.header,
        &sec.first_page_header,
        &sec.even_page_header,
        &sec.footer,
        &sec.first_page_footer,
        &sec.even_page_footer,
    ]
    .into_iter()
    .flatten()
    .map(|h| h.content.as_slice())
    .collect();
    out.blocks.extend(ctx.pics.header_footer(&hf));
    out.blocks.extend(speaker_notes(sec));
    if sec.hidden {
        out.warnings.push(Warning::new(
            WarningKind::HiddenContent,
            Level::Info,
            format!(
                "office: hidden {}, content extracted although the author hid it",
                page_noun(kind)
            ),
        ));
    }
    out
}

/// Maps the whole IR; sheets and slides count as pages for `max_pages`.
pub(crate) fn map_document(
    ir: &DocumentIR,
    date1904: bool,
    max_pages: Option<usize>,
    pics: Pictures,
) -> (Document, Vec<Warning>) {
    let kind = section_kind(ir.metadata.format);
    let mut ctx = MapCtx {
        format: ir.metadata.format,
        date1904,
        pics,
        find_title: ir.metadata.format == DocumentFormat::Docx
            && !ir
                .sections
                .iter()
                .any(|s| s.elements.iter().any(|e| matches!(e, Element::Heading(_)))),
        ..Default::default()
    };
    // office_oxide appends the master text as a trailing "Slide Master" section: not a slide.
    let master = kind == SectionKind::Slide
        && ir.sections.last().is_some_and(|s| {
            s.title.as_deref() == Some(office_oxide::pptx::layout::MASTER_TEXT_SECTION_TITLE)
        });
    let (slides, tail) = ir
        .sections
        .split_at(ir.sections.len() - usize::from(master));
    let total = slides.len();
    let paged = kind != SectionKind::Body;
    let n = if paged {
        total.min(max_pages.unwrap_or(usize::MAX))
    } else {
        total
    };
    let sections = slides[..n]
        .iter()
        .chain(tail)
        .enumerate()
        .map(|(i, s)| map_section(s, kind, i, &mut ctx))
        .collect();
    let mut doc = Document::new(sections);
    doc.metadata.title = ir.metadata.title.clone();
    doc.metadata.author = ir.metadata.author.clone();
    if paged {
        doc.metadata.page_count = Some(total);
        if n < total {
            ctx.warnings.push(Warning::new(
                WarningKind::PartialContent,
                Level::Warn,
                format!(
                    "office: processed {n} of {total} {}s (max_pages)",
                    page_noun(kind)
                ),
            ));
        }
    }
    (doc, ctx.warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::test_ir::*;
    #[test]
    fn header_variants_dedup_and_notes() {
        use office_oxide::ir::HeaderFooter;
        let mut s = section(None, vec![]);
        s.header = Some(HeaderFooter {
            content: vec![para("H")],
        });
        s.first_page_header = Some(HeaderFooter {
            content: vec![para("H")],
        });
        s.even_page_header = Some(HeaderFooter {
            content: vec![para("Even")],
        });
        s.footer = Some(HeaderFooter {
            content: vec![para(" ")],
        });
        s.speaker_notes = Some(vec![para("Say hi")]);
        let (h, f) = header_footer_notes(&s);
        let texts: Vec<_> = h
            .iter()
            .map(|b| match b {
                Block::Note {
                    kind: NoteKind::Header,
                    text,
                    ..
                } => text.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(texts, ["H", "Even"]);
        assert!(f.is_empty());
        assert!(
            matches!(speaker_notes(&s), Some(Block::Note { kind: NoteKind::SpeakerNotes, text, .. }) if text == "Say hi")
        );
    }

    #[test]
    fn kinds_and_names() {
        use office_oxide::DocumentFormat as D;
        assert_eq!(
            (
                section_kind(D::Xls),
                section_kind(D::Ppt),
                section_kind(D::Doc)
            ),
            (SectionKind::Sheet, SectionKind::Slide, SectionKind::Body)
        );
        let s = map_section(
            &section(Some("Report"), vec![heading(1, "Report")]),
            SectionKind::Body,
            0,
            &mut MapCtx::default(),
        );
        assert_eq!(s.name, None);
        let s = map_section(
            &section(Some("Data"), vec![]),
            SectionKind::Sheet,
            3,
            &mut MapCtx::default(),
        );
        assert_eq!((s.name.as_deref(), s.index), (Some("Data"), 3));
    }

    #[test]
    fn hidden_sheet_gets_info_warning() {
        let mut sec = section(Some("Secret"), vec![para("x")]);
        sec.hidden = true;
        let s = map_section(&sec, SectionKind::Sheet, 0, &mut MapCtx::default());
        assert!(
            s.warnings
                .iter()
                .any(|w| (w.kind, w.level) == (WarningKind::HiddenContent, Level::Info))
        );
    }

    #[test]
    fn block_order_headers_body_footers_notes() {
        let mut sec = section(Some("S"), vec![para("body")]);
        sec.header = Some(HeaderFooter {
            content: vec![para("h")],
        });
        sec.footer = Some(HeaderFooter {
            content: vec![para("f")],
        });
        sec.speaker_notes = Some(vec![para("n")]);
        let s = map_section(&sec, SectionKind::Slide, 0, &mut MapCtx::default());
        let kinds: Vec<_> = s
            .blocks
            .iter()
            .map(|b| match b {
                Block::Note {
                    kind: NoteKind::Header,
                    ..
                } => "h",
                Block::Paragraph { .. } => "p",
                Block::Note {
                    kind: NoteKind::Footer,
                    ..
                } => "f",
                Block::Note {
                    kind: NoteKind::SpeakerNotes,
                    ..
                } => "n",
                _ => "?",
            })
            .collect();
        assert_eq!(kinds, ["h", "p", "f", "n"]);
    }

    fn ir(format: DocumentFormat, n: usize) -> DocumentIR {
        let mut ir = DocumentIR::default();
        ir.metadata.format = format;
        ir.sections = (0..n)
            .map(|i| section(Some(&format!("S{i}")), vec![para("x")]))
            .collect();
        ir
    }

    #[test]
    fn max_pages_limits_sheets() {
        let (doc, warns) = map_document(
            &ir(DocumentFormat::Xlsx, 3),
            false,
            Some(2),
            Pictures::default(),
        );
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.metadata.page_count, Some(3));
        let partial: Vec<_> = warns
            .iter()
            .filter(|w| (w.kind, w.level) == (WarningKind::PartialContent, Level::Warn))
            .collect();
        assert_eq!(partial.len(), 1);
        assert_eq!(
            partial[0].message,
            "office: processed 2 of 3 sheets (max_pages)"
        );
    }

    #[test]
    fn max_pages_ignored_for_body() {
        let (doc, warns) = map_document(
            &ir(DocumentFormat::Docx, 2),
            false,
            Some(1),
            Pictures::default(),
        );
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.metadata.page_count, None);
        assert!(warns.is_empty());
        assert_eq!(doc.sections[1].index, 1);
    }

    #[test]
    fn slide_master_is_not_a_page() {
        let mut d = ir(DocumentFormat::Pptx, 3);
        d.sections.push(section(
            Some(office_oxide::pptx::layout::MASTER_TEXT_SECTION_TITLE),
            vec![para("m")],
        ));
        let (doc, warns) = map_document(&d, false, Some(3), Pictures::default());
        assert_eq!(doc.sections.len(), 4);
        assert_eq!(doc.metadata.page_count, Some(3));
        assert!(warns.is_empty());
        let (doc, warns) = map_document(&d, false, Some(1), Pictures::default());
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[1].index, 1);
        assert_eq!(doc.sections[1].name.as_deref(), Some("Slide Master"));
        assert_eq!(doc.metadata.page_count, Some(3));
        assert_eq!(warns.len(), 1);
        assert_eq!(warns[0].kind, WarningKind::PartialContent);
        assert_eq!(
            warns[0].message,
            "office: processed 1 of 3 slides (max_pages)"
        );
    }
}
