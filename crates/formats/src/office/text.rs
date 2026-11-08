//! Flat text of IR elements (notes, headers/footers, cell errors, flattening past the depth cap).

use office_oxide::ir::{Element, InlineContent, List, TabStop, inline_to_text};

use super::inline::para_text;

/// Lines joined by `\n`; empty lines are dropped.
pub(crate) fn elements_text(els: &[Element]) -> String {
    let mut lines = Vec::new();
    push_elements(els, 0, false, &mut lines);
    lines.join("\n")
}

/// [`elements_text`] with tabs and leading spaces kept: spreadsheet cell values are data.
pub(crate) fn sheet_text(els: &[Element]) -> String {
    let mut lines = Vec::new();
    push_elements(els, 0, true, &mut lines);
    lines.join("\n")
}

fn line(content: &[InlineContent], tabs: &[TabStop], raw: bool) -> String {
    if raw {
        inline_to_text(content)
    } else {
        para_text(content, tabs, false, None)
    }
}

/// Flat text of a list, as [`elements_text`] gives for a single `Element::List`.
pub(crate) fn list_text(l: &List) -> String {
    let mut lines = Vec::new();
    push_list(l, 0, false, &mut lines);
    lines.join("\n")
}

fn push_line(s: String, indent: usize, out: &mut Vec<String>) {
    if !s.trim().is_empty() {
        out.push(format!("{}{s}", "  ".repeat(indent)));
    }
}

fn push_elements(els: &[Element], indent: usize, raw: bool, out: &mut Vec<String>) {
    for e in els {
        match e {
            Element::Heading(h) => push_line(line(&h.content, &h.tabs, raw), indent, out),
            Element::Paragraph(p) => push_line(line(&p.content, &p.tabs, raw), indent, out),
            Element::CodeBlock(c) => push_line(c.content.clone(), indent, out),
            Element::Table(t) => {
                for r in &t.rows {
                    let cells: Vec<String> = r
                        .cells
                        .iter()
                        .map(|c| {
                            let mut v = Vec::new();
                            push_elements(&c.content, 0, raw, &mut v);
                            v.join("\n")
                        })
                        .collect();
                    push_line(cells.join(" | "), indent, out);
                }
            }
            Element::List(l) => push_list(l, indent, raw, out),
            Element::TextBox(tb) => push_elements(&tb.content, indent, raw, out),
            Element::Footnote(n) | Element::Endnote(n) => {
                push_elements(&n.content, indent, raw, out)
            }
            _ => {}
        }
    }
}

fn push_list(l: &List, indent: usize, raw: bool, out: &mut Vec<String>) {
    for it in &l.items {
        push_elements(&it.content, indent, raw, out);
        if let Some(n) = &it.nested {
            push_list(n, indent + 1, raw, out);
        }
    }
}
