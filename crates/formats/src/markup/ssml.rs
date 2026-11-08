//! XML Spreadsheet 2003 (SpreadsheetML, Excel and 1C exports named `.xls`) into sheets (Р-10).

use plaindoc_core::{
    Block, Cell, CellValue, Document, Level, NoteKind, ParseOptions, Result, ScalarValue, Section,
    SectionKind, Warning, WarningKind,
};
use roxmltree::Node;

use crate::common::{
    CellBudget, MAX_COLSPAN, MAX_ROWSPAN, MAX_SHEET_COLS, apply_max_sheets, cell_ref, clamp_repeat,
    date_serial, flag_empty, normalize_spans, parse_xml,
};

const SS: &str = "urn:schemas-microsoft-com:office:spreadsheet";

/// Namespace and `Workbook` within the first 4 KiB.
pub(crate) fn is_spreadsheet(text: &str) -> bool {
    let head = &text.as_bytes()[..text.len().min(4096)];
    let has = |p: &str| head.windows(p.len()).any(|w| w == p.as_bytes());
    has(SS) && has("Workbook")
}

/// `None` when the root element is not `Workbook` in the SpreadsheetML namespace.
pub(crate) fn parse(text: &str, opts: &ParseOptions) -> Result<Option<Document>> {
    let xml = parse_xml("xml", text)?;
    let root = xml.root_element();
    if !(root.tag_name().name() == "Workbook" && root.tag_name().namespace() == Some(SS)) {
        return Ok(None);
    }
    let sheets: Vec<Node> = root.children().filter(|n| is(n, "Worksheet")).collect();
    let total = sheets.len();
    let mut budget = CellBudget::new();
    let mut sections = Vec::new();
    for (i, ws) in sheets.into_iter().enumerate() {
        if opts.max_pages.is_some_and(|m| i >= m) {
            break;
        }
        sections.push(sheet(ws, i, &mut budget)?);
    }
    let mut doc = Document::new(sections);
    apply_max_sheets(&mut doc, opts.max_pages, total);
    flag_empty(&mut doc, "xml");
    Ok(Some(doc))
}

fn is(n: &Node, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name
}

fn num(n: &Node, name: &str, max: usize) -> Option<usize> {
    clamp_repeat(n.attribute((SS, name))?, max)
}

fn text_of(n: Node) -> String {
    n.descendants()
        .filter(Node::is_text)
        .filter_map(|t| t.text())
        .collect()
}

fn value(data: &Node, text: &str) -> ScalarValue {
    let t = text.trim();
    let typed = match data.attribute((SS, "Type")).unwrap_or("String") {
        "Number" => t
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(ScalarValue::Number),
        "Boolean" => Some(ScalarValue::Bool(matches!(t, "1" | "true" | "TRUE"))),
        "DateTime" => date_serial(t).map(ScalarValue::Date),
        "Error" => Some(ScalarValue::Error(t.to_string())),
        _ => None,
    };
    typed.unwrap_or_else(|| ScalarValue::Text(text.to_string()))
}

fn blank(c: &Cell) -> bool {
    c.text().is_empty()
        && c.value.as_ref().is_none_or(|v| {
            v.formula.is_none()
                && v.value
                    .as_ref()
                    .is_none_or(|s| *s == ScalarValue::Text(String::new()))
        })
}

fn sheet(ws: Node, index: usize, budget: &mut CellBudget) -> Result<Section> {
    let mut sec = Section::new(SectionKind::Sheet, index);
    sec.name = ws.attribute((SS, "Name")).map(str::to_string);
    if ws
        .children()
        .filter(|n| is(n, "WorksheetOptions"))
        .flat_map(|o| o.children())
        .any(|n| is(&n, "Visible") && text_of(n).trim().starts_with("SheetHidden"))
    {
        sec.warnings.push(Warning::new(
            WarningKind::HiddenContent,
            Level::Info,
            "xml: hidden sheet, content extracted although the author hid it",
        ));
    }
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut notes = Vec::new();
    // Per column: the first row that no MergeDown covers any more.
    let mut covered: Vec<usize> = Vec::new();
    let table_rows = ws
        .children()
        .filter(|n| is(n, "Table"))
        .flat_map(|t| t.children().filter(|n| is(n, "Row")));
    for row in table_rows {
        if let Some(i) = num(&row, "Index", usize::MAX) {
            let gap = i.saturating_sub(1).saturating_sub(rows.len());
            budget.take(gap)?;
            rows.extend(std::iter::repeat_with(Vec::new).take(gap));
        }
        let r = rows.len();
        let mut out = Vec::new();
        let mut col = 0usize;
        for cell in row.children().filter(|n| is(n, "Cell")) {
            let target = num(&cell, "Index", MAX_SHEET_COLS)
                .map_or(col, |i| i.saturating_sub(1))
                .max(col);
            let start = col;
            // Gap positions get empty cells, covered ones are skipped; both are charged.
            while col < MAX_SHEET_COLS && (col < target || covered.get(col).is_some_and(|&c| c > r))
            {
                if covered.get(col).is_none_or(|&c| c <= r) {
                    out.push(Cell::new(""));
                }
                col += 1;
            }
            budget.take(col - start)?;
            if col >= MAX_SHEET_COLS {
                break;
            }
            budget.take(1)?;
            let across = num(&cell, "MergeAcross", MAX_COLSPAN as usize - 1).unwrap_or(0);
            let down = num(&cell, "MergeDown", MAX_ROWSPAN as usize - 1).unwrap_or(0);
            let data = cell.children().find(|n| is(n, "Data"));
            let formula = cell
                .attribute((SS, "Formula"))
                .map(|f| f.strip_prefix('=').unwrap_or(f).to_string());
            let t = data.map(text_of).unwrap_or_default();
            let scalar = data.map(|d| value(&d, &t));
            let mut c = Cell::new(t).with_span(across as u32 + 1, down as u32 + 1);
            if scalar.is_some() || formula.is_some() {
                let mut cv = CellValue::new(scalar);
                cv.formula = formula;
                c.value = Some(cv);
            }
            if let Some(cm) = cell.children().find(|n| is(n, "Comment")) {
                let t = text_of(cm);
                if !t.trim().is_empty() {
                    notes.push(Block::Note {
                        kind: NoteKind::Comment,
                        text: format!("{}: {}", cell_ref(r, col), t.trim()),
                        bbox: None,
                    });
                }
            }
            let end = col.saturating_add(across + 1).min(MAX_SHEET_COLS);
            if down > 0 {
                budget.take(end - col)?;
                if covered.len() < end {
                    covered.resize(end, 0);
                }
                covered[col..end].fill(r + down + 1);
            }
            out.push(c);
            col = end;
        }
        rows.push(out);
    }
    while rows.last().is_some_and(|r| r.iter().all(blank)) {
        rows.pop();
    }
    if !rows.is_empty() {
        // sparse sheets are legal: no rectangularity check (Р-8, Р-12)
        normalize_spans(&mut rows);
        sec.blocks.push(Block::Table {
            header_rows: 1.min(rows.len()),
            rows,
            bbox: None,
        });
    }
    sec.blocks.extend(notes);
    Ok(sec)
}
