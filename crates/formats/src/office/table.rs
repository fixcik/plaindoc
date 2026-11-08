//! Tables: ordered cell blocks, spreadsheet values, rectangularity check.

use office_oxide::ir::{CellDataType, Table, TableCell};
use plaindoc_core::{Block, Cell, CellValue, Level, ScalarValue, Warning, WarningKind};

use super::blocks::{MapCtx, map_elements};
use super::text::sheet_text;

/// Days between the 1904 and 1900 date systems of spreadsheets.
pub(crate) const DATE1904_OFFSET: f64 = 1462.0;

/// `Block::Table`; cell content is mapped one level deeper, in IR order. The rows are taken out
/// of `t` and each IR cell is freed once mapped: a large sheet is not held twice.
pub(crate) fn map_table(t: &mut Table, ctx: &mut MapCtx) -> Block {
    let header_rows = t.rows.iter().take_while(|r| r.is_header).count();
    ctx.depth += 1;
    let mut rows = Vec::with_capacity(t.rows.len());
    for r in std::mem::take(&mut t.rows) {
        let mut cells = Vec::with_capacity(r.cells.len());
        for mut c in r.cells {
            let mut cell = Cell::new("").with_span(c.col_span, c.row_span);
            // The value reads the IR content as it is: before mapping takes nested tables out.
            cell.value = cell_value(&c, ctx.date1904);
            cell.blocks = map_elements(&mut c.content, ctx);
            cells.push(cell);
        }
        rows.push(cells);
    }
    ctx.depth -= 1;
    if !is_rectangular(&rows) {
        ctx.warnings.push(Warning::new(
            WarningKind::TableUncertain,
            Level::Warn,
            "office: table has ragged rows after spans",
        ));
    }
    Block::Table {
        rows,
        header_rows,
        bbox: None,
    }
}

/// Spreadsheet value of a cell; `None` for prose formats and cells without value data.
pub(crate) fn cell_value(c: &TableCell, date1904: bool) -> Option<CellValue> {
    let scalar = match c.data_type? {
        CellDataType::Number => c.raw_number.map(ScalarValue::Number),
        CellDataType::Date => c
            .raw_number
            .map(|r| ScalarValue::Date(r + if date1904 { DATE1904_OFFSET } else { 0.0 })),
        CellDataType::Boolean => c.raw_number.map(|r| ScalarValue::Bool(r != 0.0)),
        CellDataType::Error => Some(ScalarValue::Error(sheet_text(&c.content))),
        CellDataType::Text => Some(ScalarValue::Text(sheet_text(&c.content))),
    };
    let number_format = match c.data_type {
        Some(CellDataType::Number | CellDataType::Date) => c.number_format.clone(),
        _ => None,
    };
    if scalar.is_none() && c.formula.is_none() && number_format.is_none() {
        return None;
    }
    let mut v = CellValue::new(scalar);
    v.formula = c.formula.clone();
    v.number_format = number_format;
    Some(v)
}

/// Spans fill the grid without overlaps and every row has the same width.
pub(crate) fn is_rectangular(rows: &[Vec<Cell>]) -> bool {
    // Occupied columns per row: sorted, disjoint, non-adjacent half-open ranges
    // (no dense grid: spans come from the file and may be huge).
    let mut occ: Vec<Vec<(u64, u64)>> = vec![Vec::new(); rows.len()];
    for (r, row) in rows.iter().enumerate() {
        let mut col = 0u64;
        for c in row {
            let i = occ[r].partition_point(|&(_, end)| end <= col);
            if let Some(&(start, end)) = occ[r].get(i)
                && start <= col
            {
                col = end;
            }
            let span = (col, col + u64::from(c.colspan.max(1)));
            let last = r.saturating_add(c.rowspan.max(1) as usize).min(rows.len());
            for o in &mut occ[r..last] {
                if !occupy(o, span) {
                    return false;
                }
            }
            col = span.1;
        }
    }
    let width = |o: &Vec<(u64, u64)>| match o.as_slice() {
        [] => Some(0),
        [(0, end)] => Some(*end),
        _ => None,
    };
    let mut widths = occ.iter().map(width);
    match widths.next() {
        None => true,
        Some(None) => false,
        Some(first) => widths.all(|w| w == first),
    }
}

/// Adds `(s, e)` to the ranges; `false` if it overlaps an occupied one.
fn occupy(o: &mut Vec<(u64, u64)>, (s, e): (u64, u64)) -> bool {
    let i = o.partition_point(|&(_, end)| end <= s);
    if o.get(i).is_some_and(|&(start, _)| start < e) {
        return false;
    }
    let left = i > 0 && o[i - 1].1 == s;
    let right = o.get(i).is_some_and(|&(start, _)| start == e);
    match (left, right) {
        (true, true) => {
            o[i - 1].1 = o[i].1;
            o.remove(i);
        }
        (true, false) => o[i - 1].1 = e,
        (false, true) => o[i].0 = s,
        (false, false) => o.insert(i, (s, e)),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::test_ir::*;
    use office_oxide::ir::CellDataType as T;
    use office_oxide::ir::Element;
    fn sig(b: &Block) -> Vec<Vec<(String, u32, u32)>> {
        let Block::Table { rows, .. } = b else {
            panic!()
        };
        rows.iter()
            .map(|r| r.iter().map(|c| (c.text(), c.colspan, c.rowspan)).collect())
            .collect()
    }
    fn s(t: &str, c: u32, r: u32) -> (String, u32, u32) {
        (t.into(), c, r)
    }
    fn ir_table(e: Element) -> office_oxide::ir::Table {
        match &e {
            Element::Table(t) => t.clone(),
            _ => unreachable!(),
        }
    } // &e: Element implements Drop
    fn typed(
        t: Option<T>,
        raw: Option<f64>,
        f: Option<&str>,
        nf: Option<&str>,
        text: &str,
    ) -> office_oxide::ir::TableCell {
        let mut c = cell(text, 1, 1);
        c.data_type = t;
        c.raw_number = raw;
        c.formula = f.map(Into::into);
        c.number_format = nf.map(Into::into);
        c
    }
    fn val(v: Option<ScalarValue>, f: Option<&str>, nf: Option<&str>) -> Option<CellValue> {
        let mut cv = CellValue::new(v);
        cv.formula = f.map(Into::into);
        cv.number_format = nf.map(Into::into);
        Some(cv)
    }

    #[test]
    fn merged_cells_keep_spans_and_header_rows() {
        let mut t = ir_table(table(vec![
            row(
                vec![cell("H1", 1, 1), cell("H2", 1, 1), cell("H3", 1, 1)],
                true,
            ),
            row(vec![cell("X", 1, 2), cell("Y", 2, 1)], false),
            row(vec![cell("b2", 1, 1), cell("c2", 1, 1)], false),
        ]));
        let b = map_table(&mut t, &mut MapCtx::default());
        assert_eq!(
            sig(&b),
            vec![
                vec![s("H1", 1, 1), s("H2", 1, 1), s("H3", 1, 1)],
                vec![s("X", 1, 2), s("Y", 2, 1)],
                vec![s("b2", 1, 1), s("c2", 1, 1)]
            ]
        );
        assert!(matches!(b, Block::Table { header_rows: 1, .. }));
    }
    #[test]
    fn cell_keeps_order_paragraph_table_paragraph() {
        let mut c = cell("E", 1, 1);
        c.content.push(table(vec![row(
            vec![cell("1", 1, 1), cell("2", 1, 1)],
            false,
        )]));
        c.content.push(para("F"));
        let mut t = ir_table(table(vec![row(vec![c], false)]));
        let Block::Table { rows, .. } = map_table(&mut t, &mut MapCtx::default()) else {
            panic!()
        };
        assert!(
            matches!(&rows[0][0].blocks[..], [Block::Paragraph { text: e, .. }, Block::Table { .. }, Block::Paragraph { text: f, .. }] if e == "E" && f == "F")
        );
        assert!(!rows[0][0].is_simple());
    }
    #[test]
    fn rows_are_taken_out_and_values_read_the_whole_cell() {
        let mut c = typed(Some(T::Text), None, None, None, "a");
        c.content
            .push(table(vec![row(vec![cell("b", 1, 1)], false)]));
        let mut t = ir_table(table(vec![row(vec![c], false)]));
        let Block::Table { rows, .. } = map_table(&mut t, &mut MapCtx::default()) else {
            panic!()
        };
        assert!(t.rows.is_empty());
        assert_eq!(
            rows[0][0].value,
            val(Some(ScalarValue::Text("a\nb".into())), None, None)
        );
    }
    #[test]
    fn xlsx_values_are_not_poorer_than_ir() {
        assert_eq!(
            cell_value(
                &typed(Some(T::Number), Some(84.0), Some("B2*2"), None, "84"),
                false
            ),
            val(Some(ScalarValue::Number(84.0)), Some("B2*2"), None)
        );
        assert_eq!(
            cell_value(&typed(Some(T::Number), Some(42.0), None, None, "42"), false),
            val(Some(ScalarValue::Number(42.0)), None, None)
        );
        assert_eq!(
            cell_value(
                &typed(
                    Some(T::Date),
                    Some(38971.0),
                    None,
                    Some("dd.mm.yyyy"),
                    "11.09.2006"
                ),
                false
            ),
            val(Some(ScalarValue::Date(38971.0)), None, Some("dd.mm.yyyy"))
        );
        assert_eq!(
            cell_value(&typed(Some(T::Date), Some(37509.0), None, None, "x"), true),
            val(Some(ScalarValue::Date(38971.0)), None, None)
        );
        assert_eq!(
            cell_value(
                &typed(Some(T::Boolean), Some(1.0), None, None, "TRUE"),
                false
            ),
            val(Some(ScalarValue::Bool(true)), None, None)
        );
        assert_eq!(
            cell_value(&typed(Some(T::Error), None, None, None, "#DIV/0!"), false),
            val(Some(ScalarValue::Error("#DIV/0!".into())), None, None)
        );
        assert_eq!(
            cell_value(
                &typed(Some(T::Text), None, Some("A1&B1"), None, "ab"),
                false
            ),
            val(Some(ScalarValue::Text("ab".into())), Some("A1&B1"), None)
        );
        assert_eq!(
            cell_value(
                &typed(Some(T::Text), None, None, Some("General"), "a"),
                false
            ),
            val(Some(ScalarValue::Text("a".into())), None, None)
        );
        assert_eq!(cell_value(&cell("docx", 1, 1), false), None);
    }
    #[test]
    fn rectangularity() {
        let ok = |cells: Vec<Vec<(u32, u32)>>| {
            is_rectangular(
                &cells
                    .into_iter()
                    .map(|r| {
                        r.into_iter()
                            .map(|(c, rs)| Cell::new("").with_span(c, rs))
                            .collect()
                    })
                    .collect::<Vec<_>>(),
            )
        };
        assert!(ok(vec![
            vec![(1, 1), (1, 1), (1, 1)],
            vec![(1, 2), (2, 1)],
            vec![(1, 1), (1, 1)]
        ]));
        assert!(ok(vec![
            vec![(3, 1), (2, 1)],
            vec![(1, 1), (1, 1), (2, 1), (1, 1)],
            vec![(5, 1)]
        ])); // table-merges.doc
        assert!(!ok(vec![vec![(1, 1), (1, 1)], vec![(1, 1)]]));
    }
    #[test]
    fn ragged_table_warns_table_uncertain() {
        let mut t = ir_table(table(vec![
            row(vec![cell("a", 1, 1), cell("b", 1, 1)], false),
            row(vec![cell("c", 1, 1)], false),
        ]));
        let mut ctx = MapCtx::default();
        map_table(&mut t, &mut ctx);
        assert!(
            ctx.warnings
                .iter()
                .any(|w| w.kind == WarningKind::TableUncertain)
        );
    }
}
