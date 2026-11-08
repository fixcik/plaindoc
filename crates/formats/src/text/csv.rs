use plaindoc_core::{
    Block, Cell, Document, Format, Input, Level, ParseError, ParseOptions, Parser, Result, Section,
    SectionKind, Warning, WarningKind,
};

use super::{decode, read_text};

/// Parses csv and tsv into a single `Sheet` section with one table.
pub struct CsvParser;

impl Parser for CsvParser {
    fn formats(&self) -> &[Format] {
        &[Format::Csv, Format::Tsv]
    }

    fn parse(&self, input: Input, format: Format, opts: &ParseOptions) -> Result<Document> {
        let bytes = read_text(input, opts)?;
        let decoded = decode(&bytes);
        let (text, sep) = strip_sep_line(&decoded.text);
        let delimiter = if format == Format::Tsv {
            b'\t'
        } else {
            sep.unwrap_or_else(|| sniff_delimiter(text))
        };
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .delimiter(delimiter)
            .from_reader(text.as_bytes());
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        for record in reader.records() {
            let record = record.map_err(|e| ParseError::Corrupt(e.to_string()))?;
            rows.push(record.iter().map(Cell::new).collect());
        }

        let mut section = Section::new(SectionKind::Sheet, 0);
        section.warnings = decoded.warnings;
        if !rows.is_empty() {
            let max = rows.iter().map(Vec::len).max().unwrap_or(0);
            let min = rows.iter().map(Vec::len).min().unwrap_or(0);
            if min != max {
                for row in &mut rows {
                    row.resize_with(max, || Cell::new(""));
                }
                section.warnings.push(Warning::new(
                    WarningKind::TableUncertain,
                    Level::Warn,
                    format!("rows have {min} to {max} columns; short rows padded"),
                ));
            }
            section.blocks.push(Block::Table {
                rows,
                header_rows: 1,
                bbox: None,
            });
        }
        Ok(Document::new(vec![section]))
    }
}

/// Excel's `sep=X` first line: returns the text without it and the delimiter.
fn strip_sep_line(text: &str) -> (&str, Option<u8>) {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    match first
        .trim_end_matches('\r')
        .strip_prefix("sep=")
        .map(str::as_bytes)
    {
        Some(&[d]) if matches!(d, b';' | b',' | b'\t') => (rest, Some(d)),
        _ => (text, None),
    }
}

/// Counts `;`, tab and `,` outside double quotes on the first ~10 non-empty lines.
/// Prefers a delimiter with the same non-zero count on every line (ties: `;` > tab > `,`);
/// otherwise the most frequent on the first line (same tie order).
fn sniff_delimiter(text: &str) -> u8 {
    const CANDIDATES: [u8; 3] = *b";\t,";
    let counts: Vec<[usize; 3]> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(10)
        .map(|line| {
            let mut c = [0usize; 3];
            let mut quoted = false;
            for b in line.bytes() {
                match b {
                    b'"' => quoted = !quoted,
                    _ if quoted => {}
                    _ => {
                        if let Some(i) = CANDIDATES.iter().position(|&d| d == b) {
                            c[i] += 1;
                        }
                    }
                }
            }
            c
        })
        .collect();
    let Some(first) = counts.first() else {
        return b',';
    };
    if let Some(i) = (0..3).find(|&i| first[i] > 0 && counts.iter().all(|c| c[i] == first[i])) {
        return CANDIDATES[i];
    }
    let max = *first.iter().max().unwrap_or(&0);
    match (0..3).find(|&i| first[i] == max && max > 0) {
        Some(i) => CANDIDATES[i],
        None => b',',
    }
}
