//! odt body and tables (decision Р-8); `table_rows` is shared with ods.

use plaindoc_core::{Block, Cell, ListItem, NoteKind, ParseError, Result, ScalarValue, Warning};
use roxmltree::Node;

use super::ns::{DC, DRAW, OFFICE, TABLE, TEXT, is};
use super::styles::Styles;
use super::value::cell_value;
use crate::common::{
    CellBudget, MAX_COLSPAN, MAX_DEPTH, MAX_ROWSPAN, MAX_SHEET_COLS, MAX_SHEET_ROWS, cell_ref,
    clamp_repeat, finish_table,
};

/// State of one document walk.
pub(crate) struct Ctx<'s> {
    pub styles: &'s Styles,
    /// Footnotes, endnotes and comments, written after the current top-level block.
    pub notes: Vec<Block>,
    pub deep: bool,
    pub hidden: usize,
    pub cell_images: usize,
    pub warnings: Vec<Warning>,
    pub error: Option<ParseError>,
    budget: CellBudget,
    /// Bytes of text built so far, repeat copies included (R07), and their limit.
    bytes: u64,
    bytes_limit: u64,
    /// Images and text boxes of the paragraph being read, written after it.
    floating: Vec<Block>,
    /// Enclosing lists: style name and 1-based level.
    lists: Vec<(Option<String>, usize)>,
    in_cell: usize,
    /// Address of the spreadsheet cell being read (comments are prefixed with it).
    pub cell_ref: Option<String>,
    /// Pictures on: the largest one decoded inline (`ImageOptions::max_bytes`); `None`: off.
    pub pics: Option<u64>,
}

impl<'s> Ctx<'s> {
    pub(crate) fn new(styles: &'s Styles, bytes_limit: u64) -> Self {
        Ctx {
            styles,
            notes: Vec::new(),
            deep: false,
            hidden: 0,
            cell_images: 0,
            warnings: Vec::new(),
            error: None,
            budget: CellBudget::new(),
            bytes: 0,
            bytes_limit,
            floating: Vec::new(),
            lists: Vec::new(),
            in_cell: 0,
            cell_ref: None,
            pics: None,
        }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<()> {
        self.budget.take(n)
    }

    /// Charges `n` bytes of expanded text: `Limit { "odf expanded text", max_text_bytes }`.
    fn charge(&mut self, n: u64) -> Result<()> {
        self.bytes = self.bytes.saturating_add(n);
        if self.bytes > self.bytes_limit {
            return Err(ParseError::Limit {
                kind: "odf expanded text".into(),
                limit: self.bytes_limit,
            });
        }
        Ok(())
    }

    fn charge_soft(&mut self, n: usize) {
        if let Err(e) = self.charge(n as u64) {
            self.error.get_or_insert(e);
        }
    }
}

/// Hidden text (ruling 5): `text:hidden-text` and paragraphs holding a
/// `text:hidden-paragraph` field whose last state is `text:is-hidden="true"`,
/// and `text:section text:display="none"`.
fn hidden(ctx: &mut Ctx<'_>, n: &Node<'_, '_>) -> bool {
    let on = |f: &Node<'_, '_>| f.attribute((TEXT, "is-hidden")) == Some("true");
    let h = (is(n, TEXT, "hidden-text") && on(n))
        || ((is(n, TEXT, "p") || is(n, TEXT, "h"))
            && n.children()
                .any(|f| is(&f, TEXT, "hidden-paragraph") && on(&f)))
        || (is(n, TEXT, "section") && n.attribute((TEXT, "display")) == Some("none"));
    ctx.hidden += usize::from(h);
    h
}

/// Paragraph text under ODF whitespace rules: runs of spaces collapse, leading
/// ones go; `text:s`, `text:tab` and `text:line-break` are literal.
struct Run {
    text: String,
    ws: bool,
}

impl Run {
    fn new() -> Self {
        Run {
            text: String::new(),
            ws: true,
        }
    }

    fn raw(&mut self, s: &str) {
        for ch in s.chars() {
            if matches!(ch, ' ' | '\t' | '\n' | '\r') {
                if !self.ws {
                    self.text.push(' ');
                    self.ws = true;
                }
            } else {
                self.text.push(ch);
                self.ws = false;
            }
        }
    }

    fn exact(&mut self, s: &str, ws: bool) {
        self.text.push_str(s);
        self.ws = ws;
    }

    fn finish(self) -> String {
        self.text.trim().to_string()
    }
}

/// Text of a subtree as flat text, iteratively, honouring the hidden and skip rules.
/// `n` itself was already checked by the caller.
fn flat(ctx: &mut Ctx<'_>, n: &Node<'_, '_>, run: &mut Run) {
    let mut stack: Vec<Node<'_, '_>> = n.children().rev().collect();
    while let Some(c) = stack.pop() {
        if c.is_text() {
            run.raw(c.text().unwrap_or(""));
        } else if c.is_element() && !hidden(ctx, &c) && !skipped(&c) {
            stack.extend(c.children().rev());
        }
    }
}

/// Elements whose text is not content.
fn skipped(n: &Node<'_, '_>) -> bool {
    is(n, DC, "creator")
        || (n.tag_name().namespace() == Some(TEXT)
            && (matches!(n.tag_name().name(), "tracked-changes" | "note-citation")
                || n.tag_name().name().ends_with("-source")))
}

fn inline(ctx: &mut Ctx<'_>, n: &Node<'_, '_>, depth: usize, run: &mut Run) {
    if depth >= MAX_DEPTH {
        ctx.deep = true;
        flat(ctx, n, run);
        return;
    }
    for c in n.children() {
        if c.is_text() {
            run.raw(c.text().unwrap_or(""));
            continue;
        }
        if !c.is_element() || hidden(ctx, &c) {
            continue;
        }
        let name = c.tag_name().name();
        match c.tag_name().namespace() {
            Some(TEXT) => match name {
                "s" => {
                    let k = c
                        .attribute((TEXT, "c"))
                        .and_then(|v| clamp_repeat(v, 1024))
                        .unwrap_or(1)
                        .max(1);
                    ctx.charge_soft(k);
                    if ctx.error.is_some() {
                        return;
                    }
                    run.exact(&" ".repeat(k), false);
                }
                "tab" => run.exact("\t", true),
                "line-break" => run.exact("\n", true),
                "note" => note(ctx, &c, depth),
                _ => inline(ctx, &c, depth + 1, run),
            },
            Some(OFFICE) if name == "annotation" => note(ctx, &c, depth),
            Some(DRAW) if name == "frame" => frame(ctx, &c, depth),
            _ => inline(ctx, &c, depth + 1, run),
        }
    }
}

fn block_text(b: &Block) -> Option<String> {
    match b {
        Block::Heading { text, .. } | Block::Paragraph { text, .. } | Block::Note { text, .. } => {
            Some(text.clone())
        }
        Block::List { items, .. } => Some(
            items
                .iter()
                .map(ListItem::text)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Block::Table { rows, .. } => Some(
            rows.iter()
                .flatten()
                .map(Cell::text)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

fn plain(bs: &[Block]) -> String {
    bs.iter()
        .filter_map(block_text)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text:note` and `office:annotation` go to `ctx.notes`; the marker leaves no text.
fn note(ctx: &mut Ctx<'_>, n: &Node<'_, '_>, depth: usize) {
    let (kind, body) = if is(n, TEXT, "note") {
        let kind = if n.attribute((TEXT, "note-class")) == Some("endnote") {
            NoteKind::Endnote
        } else {
            NoteKind::Footnote
        };
        (kind, n.children().find(|c| is(c, TEXT, "note-body")))
    } else {
        (NoteKind::Comment, Some(*n))
    };
    let Some(body) = body else { return };
    let mut text = plain(&blocks(ctx, body, depth + 1, false));
    if text.is_empty() {
        return;
    }
    if kind == NoteKind::Comment
        && let Some(r) = &ctx.cell_ref
    {
        text = format!("{r}: {text}");
    }
    ctx.notes.push(Block::Note {
        kind,
        text,
        bbox: None,
    });
}

/// `draw:frame`: an image (dropped and counted inside table cells, ruling 4) or a text box.
fn frame(ctx: &mut Ctx<'_>, f: &Node<'_, '_>, depth: usize) {
    for c in f.children().filter(|c| c.is_element()) {
        if is(&c, DRAW, "image") {
            if ctx.in_cell > 0 {
                ctx.cell_images += 1;
                continue;
            }
            ctx.floating.push(Block::Image {
                alt: super::pictures::frame_alt(f),
                ocr_text: None,
                bbox: None,
                image: ctx.pics.map(|max| super::pictures::image_info(&c, max)),
            });
        } else if is(&c, DRAW, "text-box") {
            let bs = blocks(ctx, c, depth + 1, false);
            ctx.floating.extend(bs);
        }
    }
}

/// A paragraph or heading: its text and the blocks floating out of it.
fn paragraph(ctx: &mut Ctx<'_>, n: &Node<'_, '_>, depth: usize) -> (String, Vec<Block>) {
    let saved = std::mem::take(&mut ctx.floating);
    let mut run = Run::new();
    inline(ctx, n, depth, &mut run);
    let floating = std::mem::replace(&mut ctx.floating, saved);
    let text = run.finish();
    ctx.charge_soft(text.len());
    (text, floating)
}

fn list(ctx: &mut Ctx<'_>, n: &Node<'_, '_>, depth: usize) -> Option<Block> {
    let (parent_style, parent_level) = ctx.lists.last().cloned().unwrap_or((None, 0));
    let style = n
        .attribute((TEXT, "style-name"))
        .map(str::to_string)
        .or(parent_style);
    let level = parent_level + 1;
    let ordered = ctx.styles.list_ordered(style.as_deref(), level);
    ctx.lists.push((style, level));
    let mut items = Vec::new();
    for item in n.children() {
        if is(&item, TEXT, "list-item") || is(&item, TEXT, "list-header") {
            let bs = blocks(ctx, item, depth + 1, false);
            if !bs.is_empty() {
                let mut li = ListItem::new("");
                li.blocks = bs;
                items.push(li);
            }
        }
    }
    ctx.lists.pop();
    (!items.is_empty()).then_some(Block::List {
        ordered,
        items,
        bbox: None,
    })
}

fn container(name: &str) -> bool {
    matches!(name, "section" | "table-of-content" | "bibliography")
        || name.ends_with("-index")
        || name.starts_with("index-")
}

pub(crate) fn blocks(ctx: &mut Ctx<'_>, n: Node<'_, '_>, depth: usize, top: bool) -> Vec<Block> {
    let mut out = Vec::new();
    if ctx.error.is_some() {
        return out;
    }
    if depth >= MAX_DEPTH {
        // `inline` flattens at this depth: the whole subtree becomes one paragraph.
        let (text, _) = paragraph(ctx, &n, depth);
        if !text.is_empty() {
            out.push(Block::paragraph(text, None));
        }
        return out;
    }
    for c in n.children().filter(|c| c.is_element()) {
        if hidden(ctx, &c) {
            continue;
        }
        let name = c.tag_name().name();
        match c.tag_name().namespace() {
            Some(TEXT) => match name {
                "p" | "h" => {
                    let (text, floating) = paragraph(ctx, &c, depth);
                    if !text.is_empty() {
                        out.push(if name == "h" {
                            let level = c
                                .attribute((TEXT, "outline-level"))
                                .and_then(|v| clamp_repeat(v, 6))
                                .unwrap_or(1)
                                .clamp(1, 6) as u8;
                            Block::heading(level, text, None)
                        } else {
                            Block::paragraph(text, None)
                        });
                    }
                    out.extend(floating);
                }
                "list" => out.extend(list(ctx, &c, depth)),
                n if container(n) => out.extend(blocks(ctx, c, depth + 1, top)),
                _ => {}
            },
            Some(TABLE) if name == "table" => match table_rows(ctx, c, depth, false) {
                Ok((rows, header)) => {
                    out.extend(finish_table(rows, header, "odf", &mut ctx.warnings));
                }
                Err(e) => {
                    ctx.error.get_or_insert(e);
                    return out;
                }
            },
            // A cell comment is a direct child of `table:table-cell`.
            Some(OFFICE) if name == "annotation" => note(ctx, &c, depth),
            Some(DRAW) if name == "frame" => {
                frame(ctx, &c, depth);
                out.append(&mut ctx.floating);
            }
            _ => {}
        }
        if ctx.error.is_some() {
            return out;
        }
        if top {
            out.append(&mut ctx.notes);
        }
    }
    out
}

/// A repeat attribute, 1..=`max` (C3: never overflows the budget arithmetic).
fn count(n: &Node<'_, '_>, name: &str, max: usize) -> usize {
    n.attribute((TABLE, name))
        .and_then(|v| clamp_repeat(v, max))
        .unwrap_or(1)
        .max(1)
}

/// Rows of a `table:table` and the number of rows in `table:table-header-rows`.
///
/// Repeats (`number-columns-repeated`, `number-rows-repeated`) are expanded
/// within the cell budget. With `trim` (spreadsheets) empty cells and rows are
/// expanded only when followed by content, so LibreOffice's trailing
/// `number-columns-repeated="16384"` / `number-rows-repeated="1048576"` cost
/// nothing. `table:covered-table-cell` is skipped (HTML model, К-6).
pub(crate) fn table_rows(
    ctx: &mut Ctx<'_>,
    table: Node<'_, '_>,
    depth: usize,
    trim: bool,
) -> Result<(Vec<Vec<Cell>>, usize)> {
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut header = 0;
    let mut pending_rows = 0usize;
    // Row elements in order, with a flag for header rows.
    let mut stack: Vec<(Node<'_, '_>, bool)> = table.children().rev().map(|c| (c, false)).collect();
    while let Some((r, in_header)) = stack.pop() {
        if is(&r, TABLE, "table-header-rows")
            || is(&r, TABLE, "table-rows")
            || is(&r, TABLE, "table-row-group")
        {
            let h = in_header || is(&r, TABLE, "table-header-rows");
            stack.extend(r.children().rev().map(|c| (c, h)));
            continue;
        }
        if !is(&r, TABLE, "table-row") {
            continue;
        }
        let repeat = count(&r, "number-rows-repeated", MAX_SHEET_ROWS);
        let row_index = rows.len().saturating_add(pending_rows);
        let row_start = ctx.budget.used();
        let row_bytes = ctx.bytes;
        let mut cells: Vec<Cell> = Vec::new();
        let mut pending_cells = 0usize;
        let mut col = 0usize;
        for c in r.children().filter(|c| c.is_element()) {
            let n = count(&c, "number-columns-repeated", MAX_SHEET_COLS);
            if is(&c, TABLE, "covered-table-cell") {
                col = col.saturating_add(n);
                continue;
            }
            if !is(&c, TABLE, "table-cell") {
                continue;
            }
            ctx.in_cell += 1;
            ctx.cell_ref = trim.then(|| cell_ref(row_index, col));
            let before = ctx.budget.used();
            let before_bytes = ctx.bytes;
            let blocks = blocks(ctx, c, depth + 1, false);
            // Cells of tables nested in this cell: every repeated copy costs them again (C2).
            let nested = ctx.budget.used() - before;
            ctx.cell_ref = None;
            ctx.in_cell -= 1;
            if let Some(e) = ctx.error.take() {
                return Err(e);
            }
            let mut cell = Cell::new("").with_span(
                count(&c, "number-columns-spanned", MAX_COLSPAN as usize) as u32,
                count(&c, "number-rows-spanned", MAX_ROWSPAN as usize) as u32,
            );
            cell.blocks = blocks;
            cell.value = cell_value(&c, &cell.text());
            if let Some(v) = &cell.value {
                // Strings of the value are copied with the cell (R07).
                let text = match &v.value {
                    Some(ScalarValue::Text(t) | ScalarValue::Error(t)) => t.len(),
                    _ => 0,
                };
                let formula = v.formula.as_ref().map_or(0, String::len);
                ctx.charge((text + formula) as u64)?;
            }
            let empty = cell.blocks.is_empty() && cell.value.is_none() && !cell.is_merged();
            if trim && empty {
                pending_cells = pending_cells.saturating_add(n);
            } else {
                let copies = nested.saturating_mul(n - 1);
                ctx.take(pending_cells.saturating_add(n).saturating_add(copies))?;
                ctx.charge((ctx.bytes - before_bytes).saturating_mul(n as u64 - 1))?;
                cells.extend((0..pending_cells).map(|_| Cell::new("")));
                pending_cells = 0;
                cells.extend(std::iter::repeat_n(cell, n));
            }
            col = col.saturating_add(n);
        }
        if trim && cells.is_empty() {
            pending_rows = pending_rows.saturating_add(repeat);
            continue;
        }
        // Deep cost of one row (its cells and their nested tables); an empty row costs 1.
        let row_cost = (ctx.budget.used() - row_start).max(1);
        let copies = if cells.is_empty() { repeat } else { repeat - 1 };
        ctx.take(pending_rows.saturating_add(row_cost.saturating_mul(copies)))?;
        ctx.charge((ctx.bytes - row_bytes).saturating_mul(repeat as u64 - 1))?;
        rows.extend((0..pending_rows).map(|_| Vec::new()));
        pending_rows = 0;
        if in_header {
            header += repeat;
        }
        rows.extend(std::iter::repeat_n(cells, repeat));
    }
    Ok((rows, header))
}
