//! Helpers shared by the markup, odf and rtf parsers.

use plaindoc_core::{Block, Cell, Document, Level, Warning, WarningKind};

/// Nesting depth of blocks and source-tree recursion (D9).
#[cfg(any(feature = "markup", feature = "odf"))]
pub(crate) const MAX_DEPTH: usize = 128;
pub(crate) const MAX_COLSPAN: u32 = 1000;
pub(crate) const MAX_ROWSPAN: u32 = 65534;
/// Node budget of one XML document.
#[cfg(any(feature = "markup", feature = "odf"))]
pub(crate) const MAX_XML_NODES: u32 = 8_000_000;

/// Materialized cells (empty ones and cells of nested tables included) per
/// document, shared by every table of html, odt/ods, SpreadsheetML and rtf.
pub(crate) const MAX_CELLS: usize = 2_000_000;

/// The per-document cell budget: `Limit { "table cells", MAX_CELLS }` when spent.
pub(crate) struct CellBudget {
    left: usize,
}

impl Default for CellBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl CellBudget {
    pub(crate) fn new() -> Self {
        CellBudget { left: MAX_CELLS }
    }

    /// Charges `n` cells (callers compute `n` with saturating arithmetic).
    pub(crate) fn take(&mut self, n: usize) -> plaindoc_core::Result<()> {
        self.left = self
            .left
            .checked_sub(n)
            .ok_or(plaindoc_core::ParseError::Limit {
                kind: "table cells".into(),
                limit: MAX_CELLS as u64,
            })?;
        Ok(())
    }

    /// Cells charged so far: the difference around a nested parse is its deep cost.
    #[cfg(feature = "odf")]
    pub(crate) fn used(&self) -> usize {
        MAX_CELLS - self.left
    }
}

/// Spreadsheet caps of ODF and Excel: repeat and index attributes are clamped to them.
#[cfg(any(feature = "markup", feature = "odf"))]
pub(crate) const MAX_SHEET_COLS: usize = 16_384;
#[cfg(feature = "odf")]
pub(crate) const MAX_SHEET_ROWS: usize = 1_048_576;

/// A non-negative integer attribute clamped to `max`; digits past `usize` give
/// `max`, anything else gives `None`.
#[cfg(any(feature = "markup", feature = "odf"))]
pub(crate) fn clamp_repeat(raw: &str, max: usize) -> Option<usize> {
    let v = raw.trim();
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(v.parse::<usize>().unwrap_or(usize::MAX).min(max))
}

/// Ruling 4: images inside table cells are dropped; one `ContentDropped`/`Info` counts them.
pub(crate) fn cell_images_warning(src: &str, n: usize) -> Option<Warning> {
    (n > 0).then(|| {
        Warning::new(
            WarningKind::ContentDropped,
            Level::Info,
            format!("{src}: {n} images in table cells dropped"),
        )
    })
}

/// D9: sheets count as pages. `total` counts every sheet of the file (parsers
/// stop after `max` sheets and only count the rest); keeps the first `max`
/// sheets with `PartialContent`/`Warn`.
#[cfg(any(feature = "markup", feature = "odf"))]
pub(crate) fn apply_max_sheets(doc: &mut Document, max: Option<usize>, total: usize) {
    doc.metadata.page_count = Some(total);
    if let Some(n) = max
        && n < total
    {
        doc.sections.truncate(n);
        doc.warnings.push(Warning::new(
            WarningKind::PartialContent,
            Level::Warn,
            format!("processed {n} of {total} sheets (max_pages)"),
        ));
    }
}

/// colspan 1..=1000, rowspan 1..=min(65534, rows left including this one).
pub(crate) fn normalize_spans(rows: &mut [Vec<Cell>]) {
    let n = rows.len();
    for (i, row) in rows.iter_mut().enumerate() {
        let left = u32::try_from(n - i).unwrap_or(u32::MAX);
        for c in row {
            c.colspan = c.colspan.clamp(1, MAX_COLSPAN);
            c.rowspan = c.rowspan.clamp(1, MAX_ROWSPAN.min(left));
        }
    }
}

/// Places cells left to right, skipping positions covered by rowspans from
/// above. Rectangular: every row covers the same columns without holes and no
/// position is taken twice. An empty table is rectangular. Linear in cells plus
/// rows (covered columns are kept as merged runs, expiries in a heap), so it is
/// safe on raw, unclamped spans.
pub(crate) fn is_rectangular(rows: &[Vec<Cell>]) -> bool {
    use std::cmp::Reverse;
    use std::collections::{BTreeMap, BinaryHeap};

    // Covered columns as disjoint, merged runs: start -> end.
    let mut runs: BTreeMap<usize, usize> = BTreeMap::new();
    // (first row no longer covered, start, end) of every placed cell.
    let mut expiry: BinaryHeap<Reverse<(usize, usize, usize)>> = BinaryHeap::new();
    let mut width = None;
    for (i, row) in rows.iter().enumerate() {
        let expires = expiry.peek().is_some_and(|Reverse((r, ..))| *r <= i);
        if i > 0 && row.is_empty() && !expires {
            continue; // same coverage as the previous row, which passed
        }
        while let Some(&Reverse((r, s, e))) = expiry.peek() {
            if r > i {
                break;
            }
            expiry.pop();
            if let Some((&rs, &re)) = runs.range(..=s).next_back() {
                runs.remove(&rs);
                if rs < s {
                    runs.insert(rs, s);
                }
                if e < re {
                    runs.insert(e, re);
                }
            }
        }
        let mut col = 0usize;
        for c in row {
            while let Some((_, &re)) = runs.range(..=col).next_back()
                && re > col
            {
                col = re;
            }
            let end = col.saturating_add(c.colspan.max(1) as usize);
            if runs
                .range(..end)
                .next_back()
                .is_some_and(|(_, &re)| re > col)
            {
                return false;
            }
            let (mut s, mut e) = (col, end);
            if let Some((&ps, &pe)) = runs.range(..=s).next_back()
                && pe == s
            {
                runs.remove(&ps);
                s = ps;
            }
            if let Some(ne) = runs.remove(&e) {
                e = ne;
            }
            runs.insert(s, e);
            expiry.push(Reverse((
                i.saturating_add(c.rowspan.max(1) as usize),
                col,
                end,
            )));
            col = end;
        }
        let w = match (runs.len(), runs.iter().next()) {
            (0, _) => 0,
            (1, Some((&0, &e))) => e,
            _ => return false,
        };
        if width.is_some_and(|x| x != w) {
            return false;
        }
        width = Some(w);
    }
    true
}

/// Spreadsheet address from 0-based indexes: (0, 0) is "A1", (2, 27) is "AB3".
#[cfg(any(feature = "markup", feature = "odf"))]
pub(crate) fn cell_ref(row: usize, col: usize) -> String {
    let mut letters = Vec::new();
    let mut c = col.saturating_add(1);
    while c > 0 {
        let r = (c - 1) % 26;
        letters.push(b'A' + r as u8);
        c = (c - 1) / 26;
    }
    letters.reverse();
    format!(
        "{}{}",
        String::from_utf8_lossy(&letters),
        row.saturating_add(1)
    )
}

/// ISO date or date-time ("2024-01-31", "2024-01-31T12:00:00.5") as a serial
/// date in the 1900 system (days since 1899-12-30, time as the fraction).
#[cfg(any(feature = "markup", feature = "odf"))]
pub(crate) fn date_serial(iso: &str) -> Option<f64> {
    fn int(s: &str, max_len: usize) -> Option<i64> {
        (!s.is_empty() && s.len() <= max_len && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())?
    }
    fn num(s: &str) -> Option<f64> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
            .then(|| s.parse().ok())?
    }
    let (date, time) = iso.split_once('T').unwrap_or((iso, ""));
    let mut it = date.splitn(3, '-');
    let y = int(it.next()?, 4)?;
    let m = int(it.next()?, 2)?;
    let d = int(it.next()?, 2)?;
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => return None,
    };
    if !(1..=9999).contains(&y) || !(1..=month_days).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, m, d) - days_from_civil(1899, 12, 30);
    let mut secs = 0.0;
    if !time.is_empty() {
        let t = time.trim_end_matches('Z');
        let mut p = t.splitn(3, ':');
        let h = num(p.next()?)?;
        let mi = num(p.next().unwrap_or("0"))?;
        let s = num(p.next().unwrap_or("0"))?;
        if !(0.0..=24.0).contains(&h) || !(0.0..60.0).contains(&mi) || !(0.0..61.0).contains(&s) {
            return None;
        }
        secs = h * 3600.0 + mi * 60.0 + s;
    }
    Some(days as f64 + secs / 86400.0)
}

#[cfg(any(feature = "markup", feature = "odf"))]
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Empty `rows` give `None`; otherwise spans are normalized, a non-rectangular
/// grid is flagged `TableUncertain`, and `header_rows` is capped by the row count.
pub(crate) fn finish_table(
    mut rows: Vec<Vec<Cell>>,
    header_rows: usize,
    src: &str,
    warnings: &mut Vec<Warning>,
) -> Option<Block> {
    if rows.is_empty() {
        return None;
    }
    normalize_spans(&mut rows);
    if !is_rectangular(&rows) {
        warnings.push(Warning::new(
            WarningKind::TableUncertain,
            Level::Warn,
            format!("{src}: table rows have different widths"),
        ));
    }
    let header_rows = header_rows.min(rows.len());
    Some(Block::Table {
        rows,
        header_rows,
        bbox: None,
    })
}

/// Adds `EmptyOutput`/`Warn` when no section holds a block.
pub(crate) fn flag_empty(doc: &mut Document, src: &str) {
    if doc.sections.iter().all(|s| s.blocks.is_empty()) {
        doc.warnings.push(Warning::new(
            WarningKind::EmptyOutput,
            Level::Warn,
            format!("{src}: no content extracted"),
        ));
    }
}

#[cfg(any(feature = "markup", feature = "odf"))]
pub(crate) fn depth_warning(src: &str) -> Warning {
    Warning::new(
        WarningKind::ContentDropped,
        Level::Info,
        format!("{src}: content nested deeper than {MAX_DEPTH} levels was flattened to text"),
    )
}

/// Parses XML without DTDs and with a node budget.
#[cfg(any(feature = "markup", feature = "odf"))]
pub(crate) fn parse_xml<'a>(
    src: &str,
    text: &'a str,
) -> plaindoc_core::Result<roxmltree::Document<'a>> {
    let opts = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_XML_NODES,
        ..Default::default()
    };
    roxmltree::Document::parse_with_options(text, opts).map_err(|e| match e {
        roxmltree::Error::NodesLimitReached => plaindoc_core::ParseError::Limit {
            kind: format!("{src}: xml nodes"),
            limit: u64::from(MAX_XML_NODES),
        },
        e => plaindoc_core::ParseError::Corrupt(format!("{src}: {e}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(spans: Vec<Vec<(u32, u32)>>) -> Vec<Vec<Cell>> {
        spans
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|(c, rs)| Cell::new("x").with_span(c, rs))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn rectangularity_follows_spans() {
        assert!(is_rectangular(&grid(vec![
            vec![(1, 1), (1, 1), (1, 1)],
            vec![(1, 2), (2, 1)],
            vec![(1, 1), (1, 1)],
        ])));
        assert!(is_rectangular(&grid(vec![
            vec![(3, 1), (2, 1)],
            vec![(1, 1), (1, 1), (2, 1), (1, 1)],
            vec![(5, 1)],
        ])));
        assert!(is_rectangular(&[]));
        assert!(!is_rectangular(&grid(vec![
            vec![(1, 1), (1, 1)],
            vec![(1, 1)]
        ])));
        // a rowspan pushes the next row's cell one column right: overlap-free but wider
        assert!(!is_rectangular(&grid(vec![
            vec![(1, 2), (1, 1)],
            vec![(1, 1), (1, 1)],
        ])));
    }

    #[test]
    fn finish_table_clamps_spans_and_header_rows() {
        let mut w = Vec::new();
        let rows = grid(vec![vec![(5000, 9)], vec![]]);
        let Some(Block::Table {
            rows, header_rows, ..
        }) = finish_table(rows, 7, "t", &mut w)
        else {
            panic!()
        };
        assert_eq!((rows[0][0].colspan, rows[0][0].rowspan), (MAX_COLSPAN, 2));
        assert_eq!(header_rows, 2);
        assert!(w.is_empty(), "{w:?}");
        assert!(finish_table(Vec::new(), 0, "t", &mut w).is_none());
    }

    #[test]
    fn ragged_table_is_uncertain() {
        let mut w = Vec::new();
        finish_table(
            grid(vec![vec![(1, 1), (1, 1)], vec![(1, 1)]]),
            0,
            "html",
            &mut w,
        );
        assert_eq!(w[0].kind, WarningKind::TableUncertain);
        assert_eq!(w[0].message, "html: table rows have different widths");
    }

    #[test]
    fn cell_budget_is_shared_and_saturating() {
        let mut b = CellBudget::new();
        b.take(MAX_CELLS - 1).unwrap();
        let err = b.take(usize::MAX).unwrap_err();
        assert!(
            matches!(&err, plaindoc_core::ParseError::Limit { kind, limit: 2_000_000 } if kind == "table cells"),
            "{err:?}"
        );
        b.take(1).unwrap();
    }

    #[cfg(any(feature = "markup", feature = "odf"))]
    #[test]
    fn repeat_attributes_are_clamped() {
        assert_eq!(
            clamp_repeat("18446744073709551615", MAX_SHEET_COLS),
            Some(16_384)
        );
        assert_eq!(
            clamp_repeat("99999999999999999999999", 1_048_576),
            Some(1_048_576)
        );
        assert_eq!(clamp_repeat(" 3 ", 10), Some(3));
        assert_eq!(clamp_repeat("0", 10), Some(0));
        assert_eq!(clamp_repeat("-1", 10), None);
        assert_eq!(clamp_repeat("2px", 10), None);
        assert_eq!(cell_ref(usize::MAX, 0), format!("A{}", usize::MAX));
    }

    #[test]
    fn rectangularity_is_linear_and_safe_on_raw_spans() {
        let t = std::time::Instant::now();
        let mut rows = grid(vec![vec![(1000, 65534); 200]]);
        rows.extend(std::iter::repeat_with(Vec::new).take(65533));
        assert!(is_rectangular(&rows));
        assert!(is_rectangular(&grid(vec![vec![(u32::MAX, u32::MAX)]])));
        assert!(t.elapsed() < std::time::Duration::from_secs(1));
    }

    #[cfg(any(feature = "markup", feature = "odf"))]
    #[test]
    fn hostile_dates_are_rejected() {
        for bad in [
            "9223372036854775807-01-01",
            "0000-01-01",
            "10000-01-01",
            "2024-13-01",
            "2024-02-30",
            "2023-02-29",
            "2024-01-011",
            "2024-01-01T",
            "2024-01-01Tinf:00:00",
            "2024-01-01TNaN:00:00",
            "2024-01-01T25:00:00",
            "2024-01-01T10:60:00",
            "2024-01-01T-1:00:00",
        ] {
            let r = date_serial(bad);
            // a bare trailing "T" is an empty time and stays valid
            assert!(r.is_none() || bad == "2024-01-01T", "{bad}: {r:?}");
        }
        assert!(date_serial("2024-02-29").is_some());
    }

    #[cfg(any(feature = "markup", feature = "odf"))]
    #[test]
    fn serial_dates_and_refs() {
        assert_eq!(date_serial("2024-01-01"), Some(45292.0));
        assert_eq!(date_serial("1899-12-31"), Some(1.0));
        assert_eq!(date_serial("2024-01-01T12:00:00"), Some(45292.5));
        assert_eq!(date_serial("2024-01-01T06:00:00.000"), Some(45292.25));
        assert_eq!(date_serial("garbage"), None);
        assert_eq!(cell_ref(0, 0), "A1");
        assert_eq!(cell_ref(2, 27), "AB3");
        assert_eq!(cell_ref(9, 25), "Z10");
    }
}
