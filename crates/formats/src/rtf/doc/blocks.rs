//! Interpreter, second half: paragraph, list, table and document assembly.

use super::*;

impl Interp {
    pub(super) fn end_para(&mut self) {
        match self.top().target {
            Target::Main => self.end_main_para(),
            Target::Note(i) => self.notes[i].text.push('\n'),
            _ => {}
        }
    }

    /// Trimmed text of the current run; `run` keeps its capacity.
    pub(super) fn take_run(&mut self) -> String {
        let text = tidy(&self.run);
        self.run.clear();
        text
    }

    pub(super) fn end_main_para(&mut self) {
        let text = self.take_run();
        let label = std::mem::take(&mut self.label);
        if self.intbl {
            self.close_lists();
            if !text.is_empty() {
                self.cell.push(Block::paragraph(text, None));
            }
            self.table_notes.append(&mut self.pending);
            return;
        }
        self.close_table();
        let is_item = self.ls.is_some() || !label.trim().is_empty();
        if is_item && !text.is_empty() {
            self.push_item(self.ilvl, ordered_label(&label), text);
        } else if !text.is_empty() || (!is_item && !self.pending.is_empty()) {
            self.close_lists();
            if !text.is_empty() {
                self.out.push(match self.outline {
                    Some(n) => Block::heading(n + 1, text, None),
                    None => Block::paragraph(text, None),
                });
            }
        }
        // pending pictures and notes follow this paragraph; inside a list the
        // pictures join the current item and the notes wait for the list's end
        if let Some(item) = self.lists.last_mut().and_then(|(_, it)| it.last_mut()) {
            for b in std::mem::take(&mut self.pending) {
                if matches!(b, Block::Image { .. }) {
                    item.blocks.push(b);
                } else {
                    self.list_notes.push(b);
                }
            }
        } else {
            self.out.append(&mut self.pending);
        }
    }

    pub(super) fn push_item(&mut self, level: usize, ordered: bool, text: String) {
        let level = level.min(self.lists.len());
        while self.lists.len() > level + 1 {
            self.close_top_list();
        }
        if self.lists.len() == level {
            self.lists.push((ordered, Vec::new()));
        }
        self.lists[level].1.push(ListItem::new(text));
    }

    pub(super) fn close_top_list(&mut self) {
        let Some((ordered, items)) = self.lists.pop() else {
            return;
        };
        let block = Block::List {
            ordered,
            items,
            bbox: None,
        };
        match self.lists.last_mut().and_then(|(_, it)| it.last_mut()) {
            Some(parent) => parent.blocks.push(block),
            None => self.out.push(block),
        }
    }

    pub(super) fn close_lists(&mut self) {
        while !self.lists.is_empty() {
            self.close_top_list();
        }
        self.out.append(&mut self.list_notes);
    }

    pub(super) fn end_cell(&mut self) {
        self.close_lists();
        let text = self.take_run();
        if !text.is_empty() {
            self.cell.push(Block::paragraph(text, None));
        }
        self.row.push(std::mem::take(&mut self.cell));
        self.table_notes.append(&mut self.pending);
        if let Err(e) = self.budget.take(1) {
            self.error.get_or_insert(e);
        }
    }

    pub(super) fn end_row(&mut self) {
        if !self.cell.is_empty() || !self.run.trim().is_empty() {
            self.end_cell();
        } else {
            self.run.clear(); // a whitespace-only run is not rescanned on every row
        }
        self.table_notes.append(&mut self.pending);
        let mut cells: Vec<Cell> = Vec::new();
        let r = self.table.len();
        for (i, blocks) in std::mem::take(&mut self.row).into_iter().enumerate() {
            let def = self.defs.get(i).copied().unwrap_or_default();
            if def.hcont
                && let Some(prev) = cells.last_mut()
            {
                prev.colspan += 1;
                continue;
            }
            if def.vcont
                && let Some(&(or, oc)) = self.vorigin.get(&def.right)
                && let Some(origin) = self.table.get_mut(or).and_then(|row| row.get_mut(oc))
            {
                origin.rowspan += 1;
                continue;
            }
            if def.vmerge {
                self.vorigin.insert(def.right, (r, cells.len()));
            } else {
                self.vorigin.remove(&def.right);
            }
            let mut cell = Cell::new("");
            cell.blocks = blocks;
            cells.push(cell);
        }
        if cells.is_empty() {
            return;
        }
        if self.row_header && self.header_rows == r {
            self.header_rows += 1;
        }
        self.table.push(cells);
    }

    pub(super) fn close_table(&mut self) {
        if !self.row.is_empty() || !self.cell.is_empty() {
            self.end_row();
        }
        let rows = std::mem::take(&mut self.table);
        let header_rows = std::mem::take(&mut self.header_rows);
        self.vorigin.clear();
        if let Some(b) = finish_table(rows, header_rows, "rtf", &mut self.warnings) {
            self.out.push(b);
        }
        // notes of table cells follow the table
        self.out.append(&mut self.table_notes);
    }

    pub(super) fn finish(mut self) -> Parsed {
        let unclosed = self.groups.len();
        while !self.groups.is_empty() {
            self.close();
        }
        // the last paragraph is read in the main text even when its group was cut off
        self.end_main_para();
        self.close_lists();
        self.close_table();
        self.out.append(&mut self.pending);
        if unclosed > 0 {
            self.warnings.push(Warning::new(
                WarningKind::Truncated,
                Level::Warn,
                format!("rtf: {unclosed} groups not closed (truncated file?)"),
            ));
        }
        self.warnings
            .extend(cell_images_warning("rtf", self.cell_images));
        if self.nested {
            self.warnings.push(Warning::new(
                WarningKind::ContentDropped,
                Level::Info,
                "rtf: nested tables were flattened into the cell",
            ));
        }
        let mut blocks = self.headers;
        blocks.extend(self.out);
        blocks.extend(self.footers);
        let opt = |s: String| Some(tidy(&s)).filter(|s| !s.is_empty());
        Parsed {
            blocks,
            title: opt(self.title),
            author: opt(self.author),
            warnings: self.warnings,
        }
    }
}
