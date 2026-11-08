//! Feature `odf`: OpenDocument text (odt) and spreadsheets (ods).

mod body;
mod ns;
mod package;
mod pictures;
mod styles;
mod value;

use std::collections::HashSet;

use plaindoc_core::{
    Block, Document, Format, Input, Level, NoteKind, ParseOptions, Parser, Result, Section,
    SectionKind, Warning, WarningKind,
};
use roxmltree::Node;

use crate::common::{
    apply_max_sheets, cell_images_warning, depth_warning, flag_empty, normalize_spans, parse_xml,
};
use body::{Ctx, blocks, table_rows};
use ns::{DC, META, OFFICE, STYLE, TABLE, is};
use styles::Styles;

/// odt into one `Body` section, ods into one `Sheet` per table.
pub struct OdfParser;

impl Parser for OdfParser {
    fn formats(&self) -> &[Format] {
        &[Format::Odt, Format::Ods]
    }

    fn parse(&self, input: Input, _format: Format, opts: &ParseOptions) -> Result<Document> {
        let mut pkg = package::open(input, opts.max_text_bytes)?;
        let content = parse_xml("odf content.xml", &pkg.content)?;
        let mut warnings = Vec::new();
        let styles_doc = pkg
            .styles
            .as_deref()
            .and_then(|s| match parse_xml("odf styles.xml", s) {
                Ok(d) => Some(d),
                Err(e) => {
                    warnings.push(Warning::new(
                        WarningKind::PartialContent,
                        Level::Info,
                        format!("{e}; styles ignored"),
                    ));
                    None
                }
            });
        let mut docs = vec![&content];
        docs.extend(styles_doc.as_ref());
        let styles = Styles::collect(&docs);
        let mut ctx = Ctx::new(&styles, opts.max_text_bytes);
        ctx.pics = opts.images.enabled.then_some(opts.images.max_bytes);

        let body = content
            .root_element()
            .children()
            .find(|c| is(c, OFFICE, "body"));
        let first = body.and_then(|b| b.children().find(|c| c.is_element()));
        // The body decides the kind: spreadsheets go to `sheets`, the rest is text.
        let mut total = None;
        let sections = match first {
            Some(f) if is(&f, OFFICE, "spreadsheet") => {
                let (s, n) = sheets(&mut ctx, f, opts.max_pages)?;
                total = Some(n);
                s
            }
            Some(f) => vec![text_section(&mut ctx, f, styles_doc.as_ref())],
            None => vec![text_section_empty()],
        };
        if let Some(e) = ctx.error.take() {
            return Err(e);
        }

        let mut sections = sections;
        pictures::resolve(&mut sections, &mut pkg, &opts.images);
        let mut doc = Document::new(sections);
        doc.warnings = warnings;
        if let Some(n) = total {
            apply_max_sheets(&mut doc, opts.max_pages, n);
        }
        doc.warnings.append(&mut ctx.warnings);
        if ctx.deep {
            doc.warnings.push(depth_warning("odf"));
        }
        if ctx.hidden > 0 {
            doc.warnings.push(Warning::new(
                WarningKind::HiddenContent,
                Level::Info,
                format!("odf: {} hidden elements skipped", ctx.hidden),
            ));
        }
        doc.warnings
            .extend(cell_images_warning("odf", ctx.cell_images));
        if let Some(meta) = pkg.meta.as_deref()
            && let Ok(m) = parse_xml("odf meta.xml", meta)
        {
            let text = |ns: &str, name: &str| {
                m.descendants()
                    .find(|n| is(n, ns, name))
                    .and_then(|n| n.text())
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
            };
            doc.metadata.title = text(DC, "title");
            doc.metadata.author = text(META, "initial-creator").or_else(|| text(DC, "creator"));
        }
        flag_empty(&mut doc, "odf");
        Ok(doc)
    }
}

/// The first `max` sheets and the number of all sheets (the rest are only counted, I3).
fn sheets(
    ctx: &mut Ctx<'_>,
    body: Node<'_, '_>,
    max: Option<usize>,
) -> Result<(Vec<Section>, usize)> {
    let tables: Vec<_> = body.children().filter(|c| is(c, TABLE, "table")).collect();
    let mut out = Vec::new();
    for &t in tables.iter().take(max.unwrap_or(usize::MAX)) {
        let mut s = Section::new(SectionKind::Sheet, out.len());
        s.name = t.attribute((TABLE, "name")).map(str::to_string);
        if ctx.styles.table_hidden(t.attribute((TABLE, "style-name"))) {
            s.warnings.push(Warning::new(
                WarningKind::HiddenContent,
                Level::Info,
                "hidden sheet",
            ));
        }
        let cell_images = ctx.cell_images;
        let (mut rows, header) = table_rows(ctx, t, 1, true)?;
        if !rows.is_empty() {
            normalize_spans(&mut rows);
            s.blocks.push(Block::Table {
                header_rows: header.max(1).min(rows.len()),
                rows,
                bbox: None,
            });
        }
        if let Some(max) = ctx.pics {
            // A sheet's pictures (anchored in cells or on the sheet) follow its table.
            ctx.cell_images = cell_images;
            s.blocks.extend(pictures::sheet_pictures(t, max));
        }
        s.blocks.append(&mut ctx.notes);
        out.push(s);
    }
    Ok((out, tables.len()))
}

fn text_section_empty() -> Section {
    Section::new(SectionKind::Body, 0)
}

/// Header notes, the body, leftover notes, footer notes.
fn text_section(
    ctx: &mut Ctx<'_>,
    body: Node<'_, '_>,
    styles: Option<&roxmltree::Document>,
) -> Section {
    let mut section = text_section_empty();
    let headers = styles.map(|d| page_notes(ctx, d, "header", NoteKind::Header));
    section.blocks.extend(headers.unwrap_or_default());
    section.blocks.extend(blocks(ctx, body, 0, true));
    section.blocks.append(&mut ctx.notes);
    let footers = styles.map(|d| page_notes(ctx, d, "footer", NoteKind::Footer));
    section.blocks.extend(footers.unwrap_or_default());
    section
}

/// `style:header*`/`style:footer*` of every master page, without duplicates
/// and without `style:display="false"` (Р-8).
fn page_notes(
    ctx: &mut Ctx<'_>,
    styles: &roxmltree::Document,
    prefix: &str,
    kind: NoteKind,
) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for page in styles.descendants().filter(|n| is(n, STYLE, "master-page")) {
        for h in page.children().filter(|c| {
            c.is_element()
                && c.tag_name().namespace() == Some(STYLE)
                && (c.tag_name().name() == prefix
                    || c.tag_name()
                        .name()
                        .strip_prefix(prefix)
                        .is_some_and(|r| r.starts_with('-')))
                && c.attribute((STYLE, "display")) != Some("false")
        }) {
            let text = blocks(ctx, h, 1, false)
                .iter()
                .map(block_text)
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() && seen.insert(text.clone()) {
                out.push(Block::Note {
                    kind,
                    text,
                    bbox: None,
                });
            }
        }
    }
    out
}

fn block_text(b: &Block) -> String {
    match b {
        Block::Heading { text, .. } | Block::Paragraph { text, .. } => text.clone(),
        _ => String::new(),
    }
}
