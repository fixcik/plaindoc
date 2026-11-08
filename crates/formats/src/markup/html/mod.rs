//! HTML into blocks (decision Р-6): a tree walk with explicit limits. Recursion is bounded
//! by `MAX_DEPTH`; deeper subtrees (and headings, code, titles) are flattened without recursion.

use plaindoc_core::{Block, Cell, Level, ListItem, ParseError, Result, Warning, WarningKind};

use ego_tree::NodeRef;
use scraper::Node;
use scraper::node::Element;

mod img;
mod sink;
use sink::parse_html;

use crate::common::{CellBudget, MAX_DEPTH, cell_images_warning, depth_warning, finish_table};

pub(crate) struct Converted {
    pub blocks: Vec<Block>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub warnings: Vec<Warning>,
}

/// `pics`: pictures on, with the largest one decoded (`None`: `<img>` is a bare mark).
pub(crate) fn convert(text: &str, pics: Option<u64>) -> Result<Converted> {
    convert_with(text, CellBudget::new(), pics)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Skip,
    Break,
    Rule,
    Heading(u8),
    Block,
    List(bool),
    Table,
    Pre,
    Img,
    Inline,
}

fn kind(name: &str) -> Kind {
    match name {
        "head" | "script" | "style" | "template" | "noscript" | "iframe" | "object" | "embed"
        | "svg" | "math" | "canvas" | "audio" | "video" | "select" | "textarea" | "input"
        | "title" | "meta" | "link" | "base" | "param" | "source" | "track" | "datalist" => {
            Kind::Skip
        }
        "br" => Kind::Break,
        "hr" => Kind::Rule,
        "h1" => Kind::Heading(1),
        "h2" => Kind::Heading(2),
        "h3" => Kind::Heading(3),
        "h4" => Kind::Heading(4),
        "h5" => Kind::Heading(5),
        "h6" => Kind::Heading(6),
        "ul" | "menu" | "dir" => Kind::List(false),
        "ol" => Kind::List(true),
        "table" => Kind::Table,
        "pre" | "listing" | "xmp" => Kind::Pre,
        "img" => Kind::Img,
        "p" | "div" | "section" | "article" | "li" | "dt" | "dd" | "blockquote" | "header"
        | "footer" | "nav" | "aside" | "main" | "form" | "fieldset" | "figure" | "figcaption"
        | "address" | "details" | "summary" | "dl" | "center" | "body" | "html" | "hgroup"
        | "legend" | "dialog" | "caption" | "tr" | "td" | "th" | "thead" | "tbody" | "tfoot" => {
            Kind::Block
        }
        _ => Kind::Inline,
    }
}

fn is_hidden(el: &Element) -> bool {
    if el.attr("hidden").is_some() {
        return true;
    }
    el.attr("style").is_some_and(|s| {
        let s: String = s
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect();
        s.contains("display:none") || s.contains("visibility:hidden")
    })
}

/// Appends text collapsing ASCII whitespace and NBSP into single spaces.
fn push_text(run: &mut String, text: &str) {
    for c in text.chars() {
        if c.is_ascii_whitespace() || c == '\u{a0}' {
            if !(run.is_empty() || run.ends_with(' ') || run.ends_with('\n')) {
                run.push(' ');
            }
        } else {
            run.push(c);
        }
    }
}

/// Trims every line; `None` when nothing is left.
fn tidy(run: &str) -> Option<String> {
    let t = run
        .split('\n')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    let t = t.trim_matches('\n');
    (!t.is_empty()).then(|| t.to_string())
}

fn flush(run: &mut String, out: &mut Vec<Block>) {
    if let Some(text) = tidy(run) {
        out.push(Block::paragraph(text, None));
    }
    run.clear();
}

enum Item<'a> {
    Node(NodeRef<'a, Node>),
    Space,
}

/// Text of a subtree without recursion; skipped and hidden subtrees are left out.
fn flat_text(node: NodeRef<Node>, pre: bool, hidden: &mut usize) -> String {
    let mut out = String::new();
    let mut stack = vec![Item::Node(node)];
    while let Some(item) = stack.pop() {
        let n = match item {
            Item::Space => {
                push_text(&mut out, " ");
                continue;
            }
            Item::Node(n) => n,
        };
        match n.value() {
            Node::Text(t) => {
                if pre {
                    out.push_str(t);
                } else {
                    push_text(&mut out, t);
                }
            }
            Node::Element(el) => {
                let k = kind(el.name());
                if k == Kind::Skip {
                    continue;
                }
                if is_hidden(el) {
                    *hidden += 1;
                    continue;
                }
                if k == Kind::Break {
                    if pre {
                        out.push('\n');
                    } else {
                        push_text(&mut out, " ");
                    }
                    continue;
                }
                if !pre && !matches!(k, Kind::Inline | Kind::Img) {
                    push_text(&mut out, " ");
                    stack.push(Item::Space);
                }
                stack.extend(n.children().rev().map(Item::Node));
            }
            _ => {}
        }
    }
    out
}

/// Leading digits of an attribute (`"2px"` is 2); `None` when there are none.
fn span(el: &Element, name: &str) -> Option<u32> {
    let v = el.attr(name)?.trim_start();
    let digits = v.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0).then(|| v[..digits].parse().unwrap_or(u32::MAX))
}

fn item(blocks: Vec<Block>) -> ListItem {
    let mut i = ListItem::new("");
    i.blocks = blocks;
    i
}

fn code_lang(pre: NodeRef<Node>) -> Option<String> {
    let from = |el: &Element| {
        el.attr("class")?.split_whitespace().find_map(|c| {
            let l = c.strip_prefix("language-").or(c.strip_prefix("lang-"))?;
            (!l.is_empty()).then(|| l.to_string())
        })
    };
    if let Node::Element(el) = pre.value()
        && let Some(l) = from(el)
    {
        return Some(l);
    }
    pre.children().find_map(|c| match c.value() {
        Node::Element(el) if el.name() == "code" => from(el),
        _ => None,
    })
}

#[derive(Default)]
struct Walker {
    hidden: usize,
    deep: bool,
    in_cell: usize,
    warnings: Vec<Warning>,
    /// Images wait here so that they follow their paragraph.
    floating: Vec<Block>,
    cell_images: usize,
    budget: CellBudget,
    error: Option<ParseError>,
    pics: Option<u64>,
}

impl Walker {
    fn flush(&mut self, run: &mut String, out: &mut Vec<Block>) {
        flush(run, out);
        out.append(&mut self.floating);
    }

    fn walk_children(
        &mut self,
        node: NodeRef<Node>,
        depth: usize,
        run: &mut String,
        out: &mut Vec<Block>,
    ) {
        for c in node.children() {
            if self.error.is_some() {
                return;
            }
            match c.value() {
                Node::Text(t) => push_text(run, t),
                Node::Element(_) => self.element(c, depth, run, out),
                _ => {}
            }
        }
    }

    fn container(&mut self, node: NodeRef<Node>, depth: usize) -> Vec<Block> {
        let (mut run, mut out) = (String::new(), Vec::new());
        self.walk_children(node, depth, &mut run, &mut out);
        self.flush(&mut run, &mut out);
        out
    }

    fn element(&mut self, n: NodeRef<Node>, depth: usize, run: &mut String, out: &mut Vec<Block>) {
        let Node::Element(el) = n.value() else {
            return;
        };
        let k = kind(el.name());
        if k == Kind::Skip {
            return;
        }
        if is_hidden(el) {
            self.hidden += 1;
            return;
        }
        if depth >= MAX_DEPTH {
            self.deep = true;
            let t = flat_text(n, false, &mut self.hidden);
            push_text(run, &t);
            return;
        }
        match k {
            Kind::Skip => {}
            Kind::Break => run.push('\n'),
            Kind::Rule => self.flush(run, out),
            Kind::Heading(level) => {
                self.flush(run, out);
                let text = flat_text(n, false, &mut self.hidden);
                if let Some(text) = tidy(&text) {
                    out.push(Block::heading(level, text, None));
                }
            }
            Kind::Block => {
                self.flush(run, out);
                self.walk_children(n, depth + 1, run, out);
                self.flush(run, out);
            }
            Kind::List(ordered) => {
                self.flush(run, out);
                if let Some(b) = self.list(n, ordered, depth) {
                    out.push(b);
                }
            }
            Kind::Table => {
                self.flush(run, out);
                self.table(n, depth, out);
            }
            Kind::Pre => {
                self.flush(run, out);
                let text = flat_text(n, true, &mut self.hidden);
                let text = text.trim_end();
                if !text.trim().is_empty() {
                    out.push(Block::Code {
                        lang: code_lang(n),
                        text: format!("{text}\n"),
                        bbox: None,
                    });
                }
            }
            Kind::Img => {
                if self.in_cell > 0 {
                    self.cell_images += 1;
                } else {
                    let alt = el
                        .attr("alt")
                        .map(str::trim)
                        .filter(|a| !a.is_empty())
                        .map(str::to_string);
                    self.floating.push(Block::Image {
                        alt,
                        ocr_text: None,
                        bbox: None,
                        image: self.pics.map(|m| img::image_info(el.attr("src"), m)),
                    });
                }
            }
            Kind::Inline => self.walk_children(n, depth + 1, run, out),
        }
    }

    fn list(&mut self, n: NodeRef<Node>, ordered: bool, depth: usize) -> Option<Block> {
        let mut items: Vec<ListItem> = Vec::new();
        for c in n.children() {
            if self.error.is_some() {
                return None;
            }
            let Node::Element(el) = c.value() else {
                continue;
            };
            let k = kind(el.name());
            if k == Kind::Skip {
                continue;
            }
            if is_hidden(el) {
                self.hidden += 1;
                continue;
            }
            if el.name() == "li" {
                let blocks = self.container(c, depth + 2);
                if !blocks.is_empty() {
                    items.push(item(blocks));
                }
            } else if let Kind::List(o) = k {
                // a list straight inside a list belongs to the previous item
                let b = if depth + 1 >= MAX_DEPTH {
                    self.deep = true;
                    let t = flat_text(c, false, &mut self.hidden);
                    tidy(&t).map(|text| Block::paragraph(text, None))
                } else {
                    self.list(c, o, depth + 1)
                };
                if let Some(b) = b {
                    match items.last_mut() {
                        Some(last) => last.blocks.push(b),
                        None => items.push(item(vec![b])),
                    }
                }
            } else {
                let (mut run, mut blocks) = (String::new(), Vec::new());
                self.element(c, depth + 1, &mut run, &mut blocks);
                self.flush(&mut run, &mut blocks);
                if !blocks.is_empty() {
                    items.push(item(blocks));
                }
            }
        }
        (!items.is_empty()).then_some(Block::List {
            ordered,
            items,
            bbox: None,
        })
    }

    fn table(&mut self, n: NodeRef<Node>, depth: usize, out: &mut Vec<Block>) {
        // (is thead, is tfoot, rows)
        let mut groups: Vec<(bool, bool, Vec<NodeRef<Node>>)> = Vec::new();
        let mut loose = false;
        for c in n.children() {
            let Node::Element(el) = c.value() else {
                continue;
            };
            let name = el.name();
            if !matches!(name, "thead" | "tbody" | "tfoot" | "tr" | "caption") {
                continue;
            }
            if is_hidden(el) {
                self.hidden += 1;
                continue;
            }
            match name {
                "caption" => {
                    let t = flat_text(c, false, &mut self.hidden);
                    if let Some(text) = tidy(&t) {
                        out.push(Block::paragraph(text, None));
                    }
                }
                "tr" => {
                    if !loose {
                        groups.push((false, false, Vec::new()));
                        loose = true;
                    }
                    if let Some(g) = groups.last_mut() {
                        g.2.push(c);
                    }
                }
                _ => {
                    loose = false;
                    let mut trs = Vec::new();
                    for r in c.children() {
                        if let Node::Element(re) = r.value()
                            && re.name() == "tr"
                        {
                            if is_hidden(re) {
                                self.hidden += 1;
                            } else {
                                trs.push(r);
                            }
                        }
                    }
                    groups.push((name == "thead", name == "tfoot", trs));
                }
            }
        }
        let first_is_thead = groups.first().is_some_and(|g| g.0);
        groups.sort_by_key(|g| g.1); // stable: tfoot goes last
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut all_th: Vec<bool> = Vec::new();
        let mut thead_rows = 0;
        for (is_head, _, trs) in &groups {
            let total = trs.len();
            for (i, tr) in trs.iter().enumerate() {
                let left = u32::try_from(total - i).unwrap_or(u32::MAX);
                let mut row = Vec::new();
                let mut th_only = true;
                for c in tr.children() {
                    let Node::Element(el) = c.value() else {
                        continue;
                    };
                    if !matches!(el.name(), "td" | "th") {
                        continue;
                    }
                    if is_hidden(el) {
                        self.hidden += 1;
                        continue;
                    }
                    if let Err(e) = self.budget.take(1) {
                        self.error = Some(e);
                        return;
                    }
                    th_only &= el.name() == "th";
                    let colspan = span(el, "colspan").filter(|c| *c > 0).unwrap_or(1);
                    let rowspan = match span(el, "rowspan") {
                        None => 1,
                        Some(0) => left,
                        Some(r) => r.min(left),
                    };
                    self.in_cell += 1;
                    let blocks = self.container(c, depth + 4);
                    self.in_cell -= 1;
                    if self.error.is_some() {
                        return;
                    }
                    let mut cell = Cell::new("").with_span(colspan, rowspan);
                    cell.blocks = blocks;
                    row.push(cell);
                }
                if !row.is_empty() {
                    if *is_head {
                        thead_rows += 1;
                    }
                    all_th.push(th_only);
                    rows.push(row);
                }
            }
        }
        if rows.len() == 1 && rows[0].len() == 1 {
            // a one-cell table is layout
            if let Some(cell) = rows.pop().and_then(|mut r| r.pop()) {
                out.extend(cell.blocks);
            }
            return;
        }
        let header = if first_is_thead {
            thead_rows
        } else {
            all_th.iter().take_while(|b| **b).count()
        };
        let mut w = Vec::new();
        let block = finish_table(rows, header, "html", &mut w);
        for x in w {
            if !self.warnings.contains(&x) {
                self.warnings.push(x);
            }
        }
        out.extend(block);
    }
}

fn convert_with(text: &str, budget: CellBudget, pics: Option<u64>) -> Result<Converted> {
    let doc = parse_html(text)?;
    let mut w = Walker {
        budget,
        pics,
        ..Walker::default()
    };
    let root = doc.tree.root();
    let blocks = w.container(root, 0);
    if let Some(e) = w.error {
        return Err(e);
    }
    let (mut title, mut author) = (None, None);
    for n in root.descendants() {
        if title.is_some() && author.is_some() {
            break;
        }
        let Node::Element(el) = n.value() else {
            continue;
        };
        if title.is_none()
            && el.name() == "title"
            && n.parent()
                .is_some_and(|p| matches!(p.value(), Node::Element(pe) if pe.name() == "head"))
        {
            let t: String = n
                .children()
                .filter_map(|c| c.value().as_text().map(|t| &**t))
                .collect();
            let mut s = String::new();
            push_text(&mut s, &t);
            title = Some(s.trim().to_string()).filter(|t| !t.is_empty());
        } else if author.is_none()
            && el.name() == "meta"
            && el
                .attr("name")
                .is_some_and(|n| n.eq_ignore_ascii_case("author"))
        {
            author = el
                .attr("content")
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty());
        }
    }
    let mut warnings = w.warnings;
    if w.hidden > 0 {
        warnings.push(Warning::new(
            WarningKind::HiddenContent,
            Level::Info,
            format!("html: {} hidden elements skipped", w.hidden),
        ));
    }
    if w.deep {
        warnings.push(depth_warning("html"));
    }
    warnings.extend(cell_images_warning("html", w.cell_images));
    Ok(Converted {
        blocks,
        title,
        author,
        warnings,
    })
}

#[cfg(test)]
#[path = "html_test.rs"]
mod tests;
