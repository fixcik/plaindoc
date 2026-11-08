//! Numbering as a heading signal (SP8b S2b): a one-line numbered paragraph ("3.1 Introduction",
//! "Chapter 3") with a weaker style contrast than [`super::headings::HEADING_THRESHOLD`] asks is
//! still a heading when it is bold or larger than the body. Numbering gives the level in
//! `layout::styled_level`. Docling 2.148 (MIT) also reads numbering for levels; logic only.

use super::headings::{CAPTION_PREFIXES, FontStats, Para, numbering_depth, passes_filters};

/// A caption: a caption word ("table", "рис.") followed by its number ("Table 2", "Fig. 3a",
/// "Таблица А.1", "Table IV"), not "Table of contents". `lower` is in lower case.
pub(crate) fn is_caption(lower: &str) -> bool {
    CAPTION_PREFIXES.iter().any(|c| {
        lower.strip_prefix(c).is_some_and(|rest| {
            if !(c.ends_with('.')
                || rest.is_empty()
                || rest.starts_with(|c: char| c.is_whitespace() || c == ':'))
            {
                return false;
            }
            let next = rest.split_whitespace().next().unwrap_or("");
            next.is_empty()
                || next.starts_with(|c: char| c.is_ascii_digit())
                || next.chars().filter(|c| c.is_alphabetic()).count() <= 1
                || next
                    .trim_end_matches(['.', ':'])
                    .chars()
                    .all(|c| "ivxlc".contains(c))
        })
    })
}

/// Level of a numbered heading: a chapter or a part ("Chapter 2", "Часть II") is 1, a section
/// number of depth `d` ("3", "3.1") is `d + 1`.
pub(crate) fn numbered_level(text: &str) -> Option<u8> {
    let d = numbering_depth(text)?;
    let first = text.split_whitespace().next().unwrap_or("").to_lowercase();
    Some(
        if ["chapter", "part", "глава", "часть"].contains(&first.as_str()) {
            1
        } else {
            (d + 1).min(6) as u8
        },
    )
}

fn ends_with_page_number(t: &str) -> bool {
    let words: Vec<&str> = t.split_whitespace().collect();
    words.len() >= 3
        && words.last().is_some_and(|w| {
            w.split('-').count() <= 2
                && w.split('-')
                    .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
}

/// Lowest neighbour score of a numbered heading.
const MIN_SCORE: f32 = 0.3;
/// Size over the document mode that counts as larger.
const LARGER: f32 = 1.1;

/// A numbered, one-line, heading-shaped paragraph with a style cue (bold or larger than the
/// mode) and score `s` at least [`MIN_SCORE`]. A bare list marker (`1.`, `a)`, `(i)`) is not
/// numbering here: `numbering_depth` reads section numbers only. `next_to_numbered`: a neighbour
/// paragraph is numbered too, as in a table of contents or a numbered list. A line of three
/// words or more ending in a page number ("7.0 Persons contacted 7-1", "40 Annual Report 2004")
/// is a contents entry or a running head; a lower-case word after the number ("10 years B") is
/// a figure label or a quantity.
pub(crate) fn numbered_heading(
    p: &Para,
    s: f32,
    stats: &FontStats,
    next_to_numbered: bool,
) -> bool {
    !next_to_numbered
        && !ends_with_page_number(&p.text)
        && !p
            .text
            .split_whitespace()
            .nth(1)
            .is_some_and(|w| w.starts_with(char::is_lowercase))
        && s >= MIN_SCORE
        && p.lines.len() == 1
        && numbering_depth(&p.text).is_some()
        && (p.bold || (stats.mode > 0.0 && p.size >= LARGER * stats.mode))
        && passes_filters(p, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(text: &str, size: f32, bold: bool) -> Para {
        Para {
            lines: 0..1,
            text: text.into(),
            size,
            bold,
            font: "Times".into(),
            color: (0, 0, 0),
            bbox: pdf_oxide::geometry::Rect::new(72.0, 700.0, 200.0, size),
        }
    }

    #[test]
    fn numbered_line_with_a_style_cue_is_a_heading() {
        let st = FontStats {
            mode: 10.0,
            above: vec![14.0],
        };
        assert!(numbered_heading(
            &para("3.1 Introduction", 14.0, false),
            0.85,
            &st,
            false
        ));
        assert!(numbered_heading(
            &para("Chapter 3", 10.0, true),
            0.4,
            &st,
            false
        ));
        assert!(
            !numbered_heading(&para("3.1 Introduction", 14.0, false), 0.85, &st, true),
            "a TOC line"
        );
        assert!(
            !numbered_heading(&para("3.1 Introduction", 10.0, false), 0.85, &st, false),
            "no cue"
        );
        assert!(!numbered_heading(
            &para("Introduction", 14.0, true),
            0.85,
            &st,
            false
        ));
        assert!(!numbered_heading(
            &para("3.1 Introduction", 14.0, true),
            0.1,
            &st,
            false
        ));
    }

    #[test]
    fn chapters_are_level_one_sections_by_depth() {
        assert_eq!(numbered_level("Chapter 2"), Some(1));
        assert_eq!(numbered_level("ЧАСТЬ II"), Some(1));
        assert_eq!(numbered_level("2. Experience"), Some(2));
        assert_eq!(numbered_level("2.1 Acme Widgets"), Some(3));
        assert_eq!(numbered_level("Summary"), None);
    }

    #[test]
    fn contents_entries_and_folio_lines_are_not_numbered_headings() {
        let st = FontStats {
            mode: 10.0,
            above: vec![14.0],
        };
        for t in [
            "7.0 Persons and agencies contacted 7-1",
            "40 Annual Report - Fiscal Year 2004",
        ] {
            assert!(
                !numbered_heading(&para(t, 14.0, true), 0.85, &st, false),
                "{t}"
            );
        }
        assert!(numbered_heading(
            &para("Chapter 3", 14.0, true),
            0.85,
            &st,
            false
        ));
        assert!(!numbered_heading(
            &para("10 years B", 14.0, true),
            0.85,
            &st,
            false
        ));
    }

    #[test]
    fn captions_need_their_number() {
        for c in [
            "table 2. results",
            "fig. 3a",
            "figure 12",
            "таблица а.1",
            "table iv",
            "рис. 5",
        ] {
            assert!(is_caption(c), "{c}");
        }
        for t in [
            "table of contents",
            "figures and tables",
            "схема работы",
            "tablet computers",
        ] {
            assert!(!is_caption(t), "{t}");
        }
    }
}
