//! ODF spreadsheet cell values (decision Р-9).

use plaindoc_core::{CellValue, ScalarValue};
use roxmltree::Node;

use super::ns::{CALCEXT, OFFICE, TABLE};
use crate::common::date_serial;

/// Typed value and formula of a `table:table-cell`; `text` is the cell's displayed text.
/// `None` when the cell carries neither a value type nor a formula.
pub(crate) fn cell_value(cell: &Node, text: &str) -> Option<CellValue> {
    let formula = cell.attribute((TABLE, "formula")).map(|f| {
        let f = match f.split_once(":=") {
            Some((p, rest)) if p.bytes().all(|b| b.is_ascii_alphanumeric()) => rest,
            _ => f.strip_prefix('=').unwrap_or(f),
        };
        f.to_string()
    });
    let error = || Some(ScalarValue::Error(text.to_string()));
    let ty = cell.attribute((OFFICE, "value-type"));
    let value = if cell.attribute((CALCEXT, "value-type")) == Some("error") {
        error()
    } else {
        let num = |a: &str| {
            cell.attribute((OFFICE, a))
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite())
        };
        match ty {
            Some("float" | "percentage" | "currency") => num("value").map(ScalarValue::Number),
            Some("date") => cell
                .attribute((OFFICE, "date-value"))
                .and_then(date_serial)
                .map(ScalarValue::Date),
            Some("time") => cell
                .attribute((OFFICE, "time-value"))
                .and_then(duration_days)
                .map(ScalarValue::Date),
            Some("boolean") => match cell.attribute((OFFICE, "boolean-value")) {
                Some("true") => Some(ScalarValue::Bool(true)),
                Some("false") => Some(ScalarValue::Bool(false)),
                _ => None,
            },
            Some("string") => Some(ScalarValue::Text(
                cell.attribute((OFFICE, "string-value"))
                    .unwrap_or(text)
                    .to_string(),
            )),
            _ if formula.is_some() && (text.starts_with('#') || text.starts_with("Err:")) => {
                error()
            }
            _ => None,
        }
    };
    if ty.is_none() && formula.is_none() && value.is_none() {
        return None;
    }
    let mut c = CellValue::new(value);
    c.formula = formula;
    Some(c)
}

/// Time-only ISO duration (`PT12H00M00S`) as a fraction of a day; `None` for date parts.
pub(crate) fn duration_days(s: &str) -> Option<f64> {
    let mut rest = s.strip_prefix("PT")?;
    let mut secs = 0.0;
    let mut any = false;
    while !rest.is_empty() {
        let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
        let n: f64 = rest[..end].parse().ok()?;
        let unit = match rest.as_bytes()[end] {
            b'H' => 3600.0,
            b'M' => 60.0,
            b'S' => 1.0,
            _ => return None,
        };
        secs += n * unit;
        any = true;
        rest = &rest[end + 1..];
    }
    (any && secs.is_finite()).then_some(secs / 86400.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cv(attrs: &str, text: &str) -> Option<CellValue> {
        let xml = format!("<c xmlns:office=\"{OFFICE}\" xmlns:table=\"{TABLE}\" {attrs}/>");
        let doc = roxmltree::Document::parse(&xml).unwrap();
        cell_value(&doc.root_element(), text)
    }
    fn v(value: Option<ScalarValue>, formula: Option<&str>) -> Option<CellValue> {
        let mut c = CellValue::new(value);
        c.formula = formula.map(Into::into);
        Some(c)
    }

    #[test]
    fn value_types() {
        use ScalarValue::*;
        assert_eq!(
            cv(r#"office:value-type="float" office:value="1.5""#, "1,5"),
            v(Some(Number(1.5)), None)
        );
        assert_eq!(
            cv(
                r#"office:value-type="percentage" office:value="0.25""#,
                "25%"
            ),
            v(Some(Number(0.25)), None)
        );
        assert_eq!(
            cv(r#"office:value-type="currency" office:value="10""#, "10 ₽"),
            v(Some(Number(10.0)), None)
        );
        assert_eq!(
            cv(
                r#"office:value-type="date" office:date-value="2024-01-01""#,
                "01.01.24"
            ),
            v(Some(Date(45292.0)), None)
        );
        assert_eq!(
            cv(
                r#"office:value-type="time" office:time-value="PT12H00M00S""#,
                "12:00"
            ),
            v(Some(Date(0.5)), None)
        );
        assert_eq!(
            cv(
                r#"office:value-type="boolean" office:boolean-value="true""#,
                "TRUE"
            ),
            v(Some(Bool(true)), None)
        );
        assert_eq!(
            cv(r#"office:value-type="string""#, "abc"),
            v(Some(Text("abc".into())), None)
        );
        assert_eq!(
            cv(
                r#"table:formula="of:=[.A1]*2" office:value-type="float" office:value="4""#,
                "4"
            ),
            v(Some(Number(4.0)), Some("[.A1]*2"))
        );
        assert_eq!(
            cv(r#"table:formula="of:=1/0""#, "#DIV/0!"),
            v(Some(Error("#DIV/0!".into())), Some("1/0"))
        );
        // LibreOffice: explicit error type over a float 0 (I4)
        assert_eq!(
            cv(
                &format!(
                    r#"xmlns:calcext="{CALCEXT}" table:formula="of:=1/0" office:value-type="float" office:value="0" calcext:value-type="error""#
                ),
                "#DIV/0!"
            ),
            v(Some(Error("#DIV/0!".into())), Some("1/0"))
        );
        assert_eq!(cv("", "plain"), None);
        assert_eq!(duration_days("PT36H"), Some(1.5));
        assert_eq!(duration_days("P1D"), None);
    }
}
