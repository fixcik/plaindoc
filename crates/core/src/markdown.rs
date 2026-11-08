use crate::{
    Block, Cell, Document, Format, Level, ListItem, NoteKind, Section, SectionKind, Warning,
};
use crate::{md_alt, md_images, md_links};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MarkdownOptions {
    pub warnings: bool,
    /// Emit running headers, footers and page numbers (`Note` of kind `Header`, `Footer`,
    /// `PageNumber`) as HTML comments. Off by default: they stay in the model and in JSON, but
    /// would only add noise to the text. `Marginal` notes and footnotes are always emitted.
    pub headers_footers: bool,
    /// How pages (`SectionKind::Page`) are separated. Sheets and slides always get a comment.
    pub page_separator: PageSeparator,
    /// Directory of the stored pictures relative to the Markdown file (`img-3fa9c2`): a picture
    /// is `![alt](image_dir/imageN.ext)`. `None`: `![alt](imageN.ext)`.
    pub image_dir: Option<String>,
}

/// Page separator in Markdown. The page number stays in JSON (`section.index`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageSeparator {
    /// A horizontal rule `---` between pages (not before the first one, not after the last).
    #[default]
    Rule,
    /// An HTML comment `<!-- page N -->` before every page.
    Comment,
    /// Nothing.
    None,
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        Self {
            warnings: true,
            headers_footers: false,
            page_separator: PageSeparator::Rule,
            image_dir: None,
        }
    }
}

impl Document {
    pub fn to_markdown(&self) -> String {
        self.to_markdown_with(&MarkdownOptions::default())
    }

    pub fn to_markdown_with(&self, opts: &MarkdownOptions) -> String {
        let mut parts = Vec::new();
        if opts.warnings {
            push_warnings(&mut parts, &self.warnings);
        }
        // Markdown sources are passed through as is: their text is already Markdown
        let escape = self.metadata.format != Some(Format::Md);
        let mut first_page = true;
        for s in &self.sections {
            if s.kind == SectionKind::Page {
                if opts.page_separator == PageSeparator::Rule && !first_page {
                    parts.push("---".to_string());
                }
                first_page = false;
            }
            render_section(&mut parts, s, opts, escape);
        }
        parts.retain(|p| !p.is_empty());
        if parts.is_empty() {
            return String::new();
        }
        let mut out = parts.join("\n\n");
        out.push('\n');
        out
    }
}

/// Text for an HTML comment: no newlines, no `--` (which could close it).
fn comment_text(s: &str) -> String {
    let mut t = s.replace(['\r', '\n'], " ");
    while t.contains("--") {
        t = t.replace("--", "- -");
    }
    t
}

fn push_warnings(parts: &mut Vec<String>, warnings: &[Warning]) {
    for w in warnings {
        let lvl = match w.level {
            Level::Info => continue,
            Level::Warn => "warn",
            Level::Loss => "loss",
        };
        parts.push(format!(
            "<!-- warning({lvl}) {:?}: {} -->",
            w.kind,
            comment_text(&w.message)
        ));
    }
}

fn render_section(parts: &mut Vec<String>, s: &Section, opts: &MarkdownOptions, escape: bool) {
    let kind = match s.kind {
        SectionKind::Page if opts.page_separator == PageSeparator::Comment => Some("page"),
        SectionKind::Page | SectionKind::Body => None,
        SectionKind::Sheet => Some("sheet"),
        SectionKind::Slide => Some("slide"),
    };
    if let Some(kind) = kind {
        let name = s
            .name
            .as_deref()
            .map(|n| format!(": {}", comment_text(n)))
            .unwrap_or_default();
        parts.push(format!("<!-- {kind} {}{name} -->", s.index + 1));
    }
    if opts.warnings {
        push_warnings(parts, &s.warnings);
    }
    let dir = opts.image_dir.as_deref();
    let shown: Vec<&Block> = s
        .blocks
        .iter()
        .filter(|b| opts.headers_footers || !is_header_footer(b))
        .collect();
    for (i, b) in shown.iter().enumerate() {
        parts.push(match b {
            Block::Image { image: Some(_), .. } => {
                let prev = i.checked_sub(1).map(|p| shown[p]);
                let alt = md_alt::resolve(prev, b, shown.get(i + 1).copied());
                let mut one = (*b).clone();
                if let Block::Image { alt: a, .. } = &mut one {
                    *a = alt;
                }
                render_block(&one, escape, opts.headers_footers, dir)
            }
            _ => render_block(b, escape, opts.headers_footers, dir),
        });
    }
}

/// Header, footer and page-number notes: emitted only with [`MarkdownOptions::headers_footers`].
fn is_header_footer(b: &Block) -> bool {
    matches!(
        b,
        Block::Note {
            kind: NoteKind::Header | NoteKind::Footer | NoteKind::PageNumber,
            ..
        }
    )
}

/// `hf`: [`MarkdownOptions::headers_footers`], applied to list items (PPTX placeholders can be
/// bulleted).
fn render_block(b: &Block, escape: bool, hf: bool, dir: Option<&str>) -> String {
    match b {
        Block::Heading {
            level, text, links, ..
        } => format!(
            "{} {}",
            "#".repeat(usize::from((*level).clamp(1, 6))),
            md_links::markdown(text, links).replace(['\r', '\n'], " ")
        ),
        Block::Paragraph { text, links, .. } if escape => {
            escape_start(&md_links::markdown(text, links))
        }
        Block::Paragraph { text, .. } => text.clone(),
        Block::List { ordered, items, .. } => {
            let mut out = String::new();
            render_items(&mut out, items, *ordered, 0, escape, hf, dir);
            out.truncate(out.trim_end_matches('\n').len());
            out
        }
        Block::Table {
            rows, header_rows, ..
        } => render_table(rows, *header_rows, dir),
        Block::Image {
            alt,
            ocr_text,
            image,
            ..
        } => {
            let linked = image
                .as_deref()
                .and_then(|i| md_images::markdown(alt.as_deref(), i, dir));
            let mut out = match (linked, alt) {
                (Some(l), _) => l,
                (None, Some(a)) => format!("<!-- image: {} -->", comment_text(a)),
                (None, None) => "<!-- image -->".to_string(),
            };
            if let Some(t) = ocr_text.as_deref().filter(|t| !t.is_empty()) {
                out.push_str("\n\n");
                out.push_str(t);
            }
            out
        }
        Block::Code { lang, text, .. } => {
            let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
            let fence = "`".repeat(longest.max(2) + 1);
            let body = text.strip_suffix('\n').unwrap_or(text);
            format!("{fence}{}\n{body}\n{fence}", lang.as_deref().unwrap_or(""))
        }
        Block::Note { kind, text, .. } => {
            let comment = |tag: &str| format!("<!-- {tag}: {} -->", comment_text(text));
            let quote = |label: &str| {
                let body = text.replace("\r\n", "\n").replace('\n', "\n> ");
                format!("> {label}: {body}")
            };
            match kind {
                NoteKind::Header => comment("header"),
                NoteKind::Footer => comment("footer"),
                NoteKind::PageNumber => comment("page-number"),
                NoteKind::Marginal => comment("marginal"),
                NoteKind::Footnote => quote("Footnote"),
                NoteKind::Endnote => quote("Endnote"),
                NoteKind::Comment => quote("Comment"),
                NoteKind::SpeakerNotes => quote("Notes"),
            }
        }
    }
}

/// A paragraph that would start a heading, quote, table or ordered list (`#`, `>`, `|`, `1.`,
/// `1)`), or that is only `_`, `-`, `*` and spaces (a thematic break or a list item, e.g. an empty
/// form field `___`), gets a backslash escape there.
fn escape_start(text: &str) -> String {
    let lead = text.len() - text.trim_start_matches([' ', '\t']).len();
    let body = &text[lead..];
    let digits = body.len() - body.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let rule = !body.is_empty()
        && body
            .chars()
            .all(|c| matches!(c, '_' | '-' | '*' | ' ' | '\t'));
    let at = if body.starts_with(['#', '>', '|']) || rule {
        lead
    } else if (1..=9).contains(&digits)
        && body[digits..].starts_with(['.', ')'])
        && body[digits + 1..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
    {
        lead + digits
    } else {
        return text.to_string();
    };
    format!("{}\\{}", &text[..at], &text[at..])
}

fn render_items(
    out: &mut String,
    items: &[ListItem],
    ordered: bool,
    indent: usize,
    escape: bool,
    hf: bool,
    dir: Option<&str>,
) {
    let mut i = 0;
    for item in items {
        let blocks: Vec<&Block> = item
            .blocks
            .iter()
            .filter(|b| hf || !is_header_footer(b))
            .collect();
        if blocks.is_empty() && !item.blocks.is_empty() {
            continue;
        }
        i += 1;
        let marker = if ordered {
            format!("{i}. ")
        } else {
            "- ".to_string()
        };
        let pad = " ".repeat(indent);
        let inner = indent + marker.len();
        let ipad = " ".repeat(inner);
        let mut bare = false;
        let rest = match blocks.first() {
            Some(Block::Paragraph { text, links, .. }) => {
                let cont = format!("\n{ipad}");
                let text = md_links::markdown(text, links);
                let _ = writeln!(out, "{pad}{marker}{}", text.replace('\n', &cont));
                &blocks[1..]
            }
            _ => {
                // Content must follow the bare marker directly (no blank line),
                // otherwise CommonMark makes an empty item.
                bare = true;
                let _ = writeln!(out, "{pad}{}", marker.trim_end());
                &blocks[..]
            }
        };
        for (n, b) in rest.iter().enumerate() {
            match b {
                // D5: nested list keeps its own `ordered`
                Block::List { ordered, items, .. } => {
                    render_items(out, items, *ordered, inner, escape, hf, dir)
                }
                other => {
                    if !(bare && n == 0) {
                        out.push('\n');
                    }
                    for line in render_block(other, escape, hf, dir).lines() {
                        if line.is_empty() {
                            out.push('\n');
                        } else {
                            let _ = writeln!(out, "{ipad}{line}");
                        }
                    }
                }
            }
        }
    }
}

fn gfm_cell(text: &str) -> String {
    gfm_escape(text.trim())
}

fn gfm_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('<', "\\<")
        .replace("\r\n", "<br>")
        .replace(['\r', '\n'], "<br>")
}

/// A cell's text for GFM; with links, each paragraph's pieces are escaped and the links spliced in.
fn gfm_linked_cell(c: &Cell) -> String {
    let linked = c
        .blocks
        .iter()
        .any(|b| matches!(b, Block::Paragraph { links, .. } if !links.is_empty()));
    if !linked {
        return gfm_cell(&c.text());
    }
    let paras: Vec<String> = c
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph { text, links, .. } => Some(md_links::gfm(text, links, gfm_escape)),
            _ => None,
        })
        .collect();
    paras.join("<br>").trim().to_string()
}

fn render_table(rows: &[Vec<Cell>], header_rows: usize, dir: Option<&str>) -> String {
    if rows.is_empty() {
        return String::new();
    }
    if needs_html(rows, header_rows) {
        return render_html_table(rows, header_rows, dir);
    }
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let line = |cells: &[String]| format!("| {} |", cells.join(" | "));
    let row_text = |r: &[Cell]| -> Vec<String> {
        (0..cols)
            .map(|i| r.get(i).map(gfm_linked_cell).unwrap_or_default())
            .collect()
    };
    let (head, body) = if header_rows == 1 {
        (row_text(&rows[0]), &rows[1..])
    } else {
        (vec![String::new(); cols], rows)
    };
    let mut lines = vec![line(&head), line(&vec!["---".to_string(); cols])];
    lines.extend(body.iter().map(|r| line(&row_text(r))));
    lines.join("\n")
}

fn needs_html(rows: &[Vec<Cell>], header_rows: usize) -> bool {
    header_rows > 1
        || rows
            .iter()
            .flatten()
            .any(|c| c.is_merged() || !c.is_simple())
}

/// Escapes text for HTML and turns every line break (`\r\n`, `\n`, lone `\r`)
/// into `<br>`, so a blank line can never end the HTML block.
/// Note: HTML rendering recurses with nesting depth (hand-built Documents are unbounded).
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "<br>")
}

fn render_html_table(rows: &[Vec<Cell>], header_rows: usize, dir: Option<&str>) -> String {
    let header_rows = header_rows.min(rows.len());
    let mut out = String::from("<table>\n");
    for (i, row) in rows.iter().enumerate() {
        if i == 0 && header_rows > 0 {
            out.push_str("<thead>\n");
        }
        if i == header_rows {
            out.push_str("<tbody>\n");
        }
        out.push_str("<tr>");
        let tag = if i < header_rows { "th" } else { "td" };
        for c in row {
            let _ = write!(out, "<{tag}");
            if c.colspan > 1 {
                let _ = write!(out, " colspan=\"{}\"", c.colspan);
            }
            if c.rowspan > 1 {
                let _ = write!(out, " rowspan=\"{}\"", c.rowspan);
            }
            let _ = write!(out, ">{}</{tag}>", html_inline_blocks(&c.blocks, dir));
        }
        out.push_str("</tr>\n");
        if i + 1 == header_rows {
            out.push_str("</thead>\n");
        }
    }
    if rows.len() > header_rows {
        out.push_str("</tbody>\n");
    }
    out.push_str("</table>");
    out
}

fn html_block(b: &Block, dir: Option<&str>) -> String {
    match b {
        Block::Table {
            rows, header_rows, ..
        } => render_html_table(rows, *header_rows, dir),
        Block::List { ordered, items, .. } => html_list(items, *ordered, dir),
        Block::Heading { text, .. }
        | Block::Paragraph { text, .. }
        | Block::Code { text, .. }
        | Block::Note { text, .. } => format!("<p>{}</p>", html_escape(text)),
        Block::Image {
            alt,
            ocr_text,
            image,
            ..
        } => {
            let t = ocr_text.as_deref().or(alt.as_deref()).unwrap_or("");
            let p = format!("<p>{}</p>", html_escape(t));
            match image
                .as_deref()
                .and_then(|i| md_images::html(alt.as_deref(), i, dir, html_escape))
            {
                Some(img) if ocr_text.as_deref().is_some_and(|t| !t.is_empty()) => img + &p,
                Some(img) => img,
                None => p,
            }
        }
    }
}

/// Cell / list-item content in order; consecutive paragraphs are joined by `<br>`.
fn html_inline_blocks(blocks: &[Block], dir: Option<&str>) -> String {
    let mut out = String::new();
    let mut prev_para = false;
    for b in blocks {
        match b {
            Block::Paragraph { text, links, .. } => {
                if prev_para {
                    out.push_str("<br>");
                }
                out.push_str(&md_links::html(text, links, html_escape));
                prev_para = true;
            }
            other => {
                out.push_str(&html_block(other, dir));
                prev_para = false;
            }
        }
    }
    out
}

fn html_list(items: &[ListItem], ordered: bool, dir: Option<&str>) -> String {
    let tag = if ordered { "ol" } else { "ul" };
    let mut out = format!("<{tag}>");
    for item in items {
        let _ = write!(out, "<li>{}</li>", html_inline_blocks(&item.blocks, dir));
    }
    let _ = write!(out, "</{tag}>");
    out
}
