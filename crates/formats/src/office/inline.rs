//! Paragraph text of IR inline content: tabs, fill-in fields and note citations.
//!
//! Tabs at the start or end of a line are dropped (at the start of a Markdown line they open a
//! code block); a run of tabs inside a line is one tab, the column boundary. A tab to a stop with
//! an underscore leader, and the fill line of a form field (see `form::is_field`), become
//! [`FIELD`]. A footnote/endnote citation becomes a label `[1]` on the word it annotates, and the
//! note body starts with the same label.

use std::collections::HashMap;

use office_oxide::ir::{InlineContent, TabLeader, TabStop, VerticalAlign};

/// An empty fill-in field of a form.
pub(crate) const FIELD: &str = "_____";

/// Labels of the cited footnotes (`false`) and endnotes (`true`), in citation order.
#[derive(Default)]
pub(crate) struct NoteLabels {
    /// `(endnote, note id)` -> `(label, custom mark)`.
    seen: HashMap<(bool, u32), (String, Option<String>)>,
    count: [u32; 2],
}

impl NoteLabels {
    /// Label of a citation: the custom mark (`customMarkFollows`) or the next number.
    fn cite(&mut self, end: bool, id: u32, mark: Option<String>) -> String {
        if let Some((label, _)) = self.seen.get(&(end, id)) {
            return label.clone();
        }
        let label = match &mark {
            Some(m) => format!("[{m}]"),
            None => {
                self.count[usize::from(end)] += 1;
                format!("[{}]", self.count[usize::from(end)])
            }
        };
        self.seen.insert((end, id), (label.clone(), mark));
        label
    }

    /// Body of a note with its label in front (the custom mark the body repeats is dropped);
    /// unchanged for a note that was not cited.
    pub(crate) fn body(&self, end: bool, id: u32, text: String) -> String {
        let Some((label, mark)) = self.seen.get(&(end, id)) else {
            return text;
        };
        let t = text.trim_start();
        let t = mark
            .as_deref()
            .and_then(|m| t.strip_prefix(m))
            .unwrap_or(t)
            .trim_start();
        format!("{label} {t}")
    }
}

/// Custom mark of a citation: a short superscript span right after the reference.
fn custom_mark(next: Option<&&InlineContent>) -> Option<String> {
    let Some(InlineContent::Text(s)) = next else {
        return None;
    };
    let m = s.text.trim();
    (s.vertical_align == Some(VerticalAlign::Superscript)
        && !m.is_empty()
        && m.chars().count() <= 3)
        .then(|| m.to_string())
}

/// Text of a paragraph. `notes`: label citations (body text); `field`: the paragraph is the
/// fill line of a form field.
pub(crate) fn para_text(
    content: &[InlineContent],
    tabs: &[TabStop],
    field: bool,
    mut notes: Option<&mut NoteLabels>,
) -> String {
    let mut out = String::new();
    let mut it = content.iter().peekable();
    while let Some(ic) = it.next() {
        match ic {
            InlineContent::Text(s) => out.push_str(&s.text),
            InlineContent::LineBreak => out.push('\n'),
            InlineContent::FootnoteRef(r) | InlineContent::EndnoteRef(r) => {
                let Some(notes) = notes.as_deref_mut() else {
                    continue;
                };
                let mark = custom_mark(it.peek());
                if mark.is_some() {
                    it.next();
                }
                let end = matches!(ic, InlineContent::EndnoteRef(_));
                // The label sits on the word it annotates.
                out.truncate(out.trim_end_matches([' ', '\u{a0}']).len());
                out.push_str(&notes.cite(end, r.note_id, mark));
            }
            _ => {}
        }
    }
    let mut stops: Vec<&TabStop> = tabs.iter().collect();
    stops.sort_by_key(|t| t.position_twips);
    let lines: Vec<&str> = out.split('\n').collect();
    let last = lines.len() - 1;
    let mut placed = false;
    let mut lines: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| layout_line(l, &stops, field && i == last, &mut placed))
        .collect();
    if field && !placed {
        let l = lines.last_mut().expect("split gives a line");
        let t = l.trim_end_matches([' ', '\u{a0}']);
        // Underscores typed by hand are the field already.
        *l = if t.ends_with('_') {
            t.to_string()
        } else {
            join_field(t, "")
        };
    }
    lines.join("\n")
}

/// One line: tab runs inside it are one tab; a run reaching a stop with an underscore leader
/// (the n-th tab of the line goes to the n-th stop), or the last run of a field line, is
/// [`FIELD`]. Whitespace at the start and tabs at the end are dropped.
fn layout_line(line: &str, stops: &[&TabStop], field: bool, placed: &mut bool) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut leaders: Vec<bool> = Vec::new();
    let mut rest = line;
    let mut tab_no = 0;
    while let Some(i) = rest.find('\t') {
        parts.push(&rest[..i]);
        let after = rest[i..].trim_start_matches('\t');
        let n = rest.len() - i - after.len();
        leaders.push((tab_no..tab_no + n).any(|k| {
            stops
                .get(k)
                .is_some_and(|t| t.leader == TabLeader::Underscore)
        }));
        tab_no += n;
        rest = after;
    }
    parts.push(rest);
    let gaps = leaders.len();
    let is_field = |k: usize| leaders[k - 1] || (field && k == gaps);
    // Tabs at the end of the line (not fields) separate nothing: dropped with their spaces.
    let mut keep = parts.len();
    while keep > 1 && parts[keep - 1].trim().is_empty() && !is_field(keep - 1) {
        keep -= 1;
    }
    let mut out = parts[0].to_string();
    for (k, piece) in parts.iter().enumerate().take(keep).skip(1) {
        if is_field(k) {
            out = join_field(out.trim_end_matches([' ', '\u{a0}']), piece);
            *placed = true;
        } else {
            out.push('\t');
            out.push_str(piece);
        }
    }
    if keep < parts.len() {
        out.truncate(out.trim_end_matches(' ').len());
    }
    out.trim_start_matches([' ', '\t']).to_string()
}

/// `before FIELD after`: spaces around the field, except before `. , ; :` that end the line.
fn join_field(before: &str, after: &str) -> String {
    let after = after.trim_start();
    let mut s = before.to_string();
    if !s.is_empty() && !s.ends_with('\n') {
        s.push(' ');
    }
    s.push_str(FIELD);
    if !after.is_empty() && !after.starts_with(['.', ',', ';', ':']) {
        s.push(' ');
    }
    s.push_str(after);
    s
}

#[cfg(test)]
#[path = "forms_test.rs"]
mod tests;
