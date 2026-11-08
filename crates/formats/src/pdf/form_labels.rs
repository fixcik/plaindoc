//! Field labels of forms (questionnaires, applications: "ФИО", "ПАСПОРТ", "1. Фамилия" over the
//! value). Such a label is short and styled like a title, but it heads a field, not a section:
//! the capitals and numbering heading rules skip it.
//!
//! A label is a short one-line paragraph, not larger than the body, followed by its value (a
//! short one-line paragraph that is not a sentence, or nothing but a line or blank) and then by
//! another label of the same style, or preceded by one (label, value, label, value ...); or
//! followed directly by a paragraph of its own style (labels of empty fields). All of them start at the same x (one field column: a sidebar title next to a
//! main-column title in reading order is not a field). A section title is followed by its body
//! instead. The rule guards the capitals and numbering heading rules (the score rule keeps its
//! own filters; bold labels it promoted before SP8b stay).

use super::headings::{Para, numbering_depth};
use super::headings_style::is_caps;

/// Longest label and longest value (non-whitespace chars).
const MAX_LABEL: usize = 40;
const MAX_VALUE: usize = 60;
/// A label is at most this share of the body size.
const MAX_OF_BODY: f32 = 1.05;
/// Rows of one field start within this many font sizes of each other in x.
const ALIGN_FS: f32 = 2.0;

fn chars(p: &Para) -> usize {
    p.text.chars().filter(|c| !c.is_whitespace()).count()
}

/// Ends like a sentence: a label or a field value does not.
fn sentence(p: &Para) -> bool {
    p.text.trim_end().ends_with(['.', '!', '?'])
}

fn short(p: &Para, max: usize) -> bool {
    p.lines.len() == 1 && chars(p) <= max
}

/// Same size (±10 %), weight, colour and case (numbered rows may differ in case: "1. ФИО",
/// "2. Дата рождения").
fn same_style(a: &Para, b: &Para) -> bool {
    (a.size - b.size).abs() <= 0.1 * a.size.max(b.size)
        && a.bold == b.bold
        && a.color == b.color
        && (is_caps(&a.text) == is_caps(&b.text)
            || (numbering_depth(&a.text).is_some() && numbering_depth(&b.text).is_some()))
}

/// For each paragraph of the page (reading order), whether it is a row of a form field (a label,
/// or a value: the pattern is symmetric). A label is not larger than the body (`body`, the
/// char-weighted size mode) and its value is not a sentence: a title over a one-line sentence
/// ("ЦЕЛИ" / "Быстрый разбор документов." / "СРОКИ") is a heading.
pub(crate) fn field_labels(paras: &[&Para], body: f32) -> Vec<bool> {
    let n = paras.len();
    let at = |i: usize| paras.get(i).copied();
    (0..n)
        .map(|i| {
            let p = paras[i];
            if !short(p, MAX_LABEL) || sentence(p) || (body > 0.0 && p.size > MAX_OF_BODY * body) {
                return false;
            }
            // the rows of one field column start at the same x
            let aligned = |q: &Para| (q.bbox.x - p.bbox.x).abs() < ALIGN_FS * p.size.max(q.size);
            let value = |j: usize| {
                at(j).is_some_and(|v| {
                    short(v, MAX_VALUE) && !same_style(p, v) && aligned(v) && !sentence(v)
                })
            };
            let label = |j: Option<usize>| {
                j.and_then(at)
                    .is_some_and(|q| short(q, MAX_LABEL) && same_style(p, q) && aligned(q))
            };
            let next_label = label(Some(i + 2)) && value(i + 1);
            let prev_label = i >= 2 && label(Some(i - 2)) && value(i - 1);
            let empty_fields = label(Some(i + 1)) || (i >= 1 && label(Some(i - 1)));
            (value(i + 1) && (next_label || prev_label))
                || (prev_label && at(i + 1).is_none())
                || empty_fields
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(text: &str, size: f32, bold: bool, color: (u8, u8, u8)) -> Para {
        Para {
            lines: 0..1,
            text: text.into(),
            size,
            bold,
            font: "Helvetica".into(),
            color,
            bbox: pdf_oxide::geometry::Rect::new(40.0, 400.0, 100.0, size),
        }
    }

    #[test]
    fn labels_over_values_are_labels_section_titles_are_not() {
        let blue = (31, 86, 168);
        let black = (0, 0, 0);
        let form = [
            para("ФИО", 9.0, false, blue),
            para("Иванов Иван Иванович", 11.0, false, black),
            para("ПАСПОРТ", 9.0, false, blue),
            para("4500 123456", 11.0, false, black),
            para("ТЕЛЕФОН", 9.0, false, blue),
            para("+7 000 000-00-00", 11.0, false, black),
        ];
        let refs: Vec<&Para> = form.iter().collect();
        let l = field_labels(&refs, 11.0);
        assert!(l[0] && l[2] && l[4], "{l:?}");

        let body = "Опытный разработчик, интересуюсь качеством кода. Люблю небольшие команды и ясные задачи.";
        let mut long = para(body, 10.5, false, black);
        long.lines = 0..3;
        let resume = [
            para("О СЕБЕ", 11.0, true, black),
            long.clone(),
            para("ОБРАЗОВАНИЕ", 11.0, true, black),
            para("Тестовый университет", 10.5, true, black),
            para("Прикладная информатика, 2012", 9.5, false, (102, 102, 102)),
            para("ДОПОЛНИТЕЛЬНО", 11.0, true, black),
            long,
        ];
        let refs: Vec<&Para> = resume.iter().collect();
        assert!(field_labels(&refs, 10.5).iter().all(|l| !l));

        // bold capitals of the body size over one short line each: section titles
        let short_doc = [
            para("ЦЕЛИ", 11.0, true, black),
            para("Снизить затраты на треть.", 11.0, false, black),
            para("СРОКИ", 11.0, true, black),
            para("Два квартала.", 11.0, false, black),
            para("РИСКИ", 11.0, true, black),
            para("Нехватка людей.", 11.0, false, black),
        ];
        let refs: Vec<&Para> = short_doc.iter().collect();
        assert!(field_labels(&refs, 11.0).iter().all(|l| !l));
    }

    #[test]
    fn labels_of_empty_fields_in_a_row() {
        let b = (0, 0, 0);
        let rows = [
            para("1. Фамилия", 10.0, true, b),
            para("2. Имя", 10.0, true, b),
            para("3. Отчество", 10.0, true, b),
        ];
        let refs: Vec<&Para> = rows.iter().collect();
        assert_eq!(field_labels(&refs, 11.0), [true, true, true]);
    }
}
