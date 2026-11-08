//! Style signals of section titles beyond size and weight (SP8b S2c): a short line in capitals
//! does not join the line under it, and a letter-spaced title (`К О Н Т А К Т Ы`) reads as the
//! word.

use std::borrow::Cow;

use pdf_oxide::layout::TextSpan;

use super::headings::{Para, StyledLine};

/// Longest line (non-whitespace chars) taken as a capitals title.
const MAX_CAPS_TITLE: usize = 60;

/// At least three letters, all upper case.
pub(crate) fn is_caps(t: &str) -> bool {
    let letters: Vec<char> = t.chars().filter(|c| c.is_alphabetic()).collect();
    letters.len() >= 3 && letters.iter().all(|c| c.is_uppercase())
}

fn caps_title(l: &StyledLine) -> bool {
    is_caps(&l.text) && l.text.chars().filter(|c| !c.is_whitespace()).count() <= MAX_CAPS_TITLE
}

/// Lines `a` (above) and `b` differ in case: one a short capitals line, the other not capitals.
/// Such lines are separate paragraphs ("EDUCATION" over a bold "Test State University").
pub(crate) fn case_break(a: &StyledLine, b: &StyledLine) -> bool {
    (caps_title(a) && !is_caps(&b.text)) || (caps_title(b) && !is_caps(&a.text))
}

/// Longest capitals title (non-whitespace chars) and words in it.
const MAX_CAPS_HEADING: usize = 40;
const MAX_CAPS_WORDS: usize = 4;
/// Smallest capitals title against the body size: section labels are often small.
const MIN_CAPS_OF_BODY: f32 = 0.85;
/// A gap between words wider than this many font sizes: a row of labels (chart legend), not a
/// title.
const MAX_GAP_FS: f32 = 1.5;
/// Channel spread (max - min of r, g, b) of an accent colour; greys are below.
const ACCENT_SPREAD: u8 = 64;

fn accent((r, g, b): (u8, u8, u8)) -> bool {
    r.max(g).max(b) - r.min(g).min(b) >= ACCENT_SPREAD
}

/// A section title by case alone: one line of at most four capitalised words (letter-spaced
/// read as words), no digits, no closing punctuation, bold or in an accent colour, not much
/// smaller than the body (`body`, the char-weighted size mode), no wide gap inside (`line` is its
/// line). Grey small-caps labels (contact field names) stay text.
pub(crate) fn caps_heading(p: &Para, line: &StyledLine, body: f32) -> bool {
    let t = title_text(p, std::slice::from_ref(line), p.lines.start);
    // letters whose words cannot be told apart count as one word
    let t = if letter_spaced(&t) {
        Cow::Owned(t.split_whitespace().collect::<String>())
    } else {
        t
    };
    let n = t.chars().filter(|c| !c.is_whitespace()).count();
    p.lines.len() == 1
        && is_caps(&t)
        && n <= MAX_CAPS_HEADING
        && t.split_whitespace().count() <= MAX_CAPS_WORDS
        && !t.chars().any(|c| c.is_ascii_digit())
        && !t.trim_end().ends_with(['.', ',', ';', ':'])
        && (p.bold || accent(p.color))
        && p.size.is_finite()
        && line.gap <= MAX_GAP_FS * line.size
        && (body <= 0.0 || p.size >= MIN_CAPS_OF_BODY * body)
}

/// Text that names a link or a repository rather than a section: a URL, an e-mail address or a
/// domain anywhere, or a first word that is a lower-case identifier (`tiny-parser`,
/// `owner/repo`, `my_tool`). Docling makes such lines headings on resumes; we do not.
pub(crate) fn link_shaped(t: &str) -> bool {
    const DOMAINS: [&str; 8] = [".com", ".org", ".net", ".io", ".ru", ".dev", ".app", ".me"];
    let lower = t.to_lowercase();
    let first = t.split_whitespace().next().unwrap_or("");
    lower.split_whitespace().any(|w| {
        let w = w.trim_end_matches(['.', ',', ')']);
        w.contains("://")
            || w.starts_with("www.")
            || (w.contains('@') && w.len() > 1)
            || DOMAINS.iter().any(|d| w.ends_with(d) && w.len() > d.len())
    }) || (first.contains(['-', '_', '/'])
        && first.chars().any(|c| c.is_ascii_lowercase())
        && first
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_/.".contains(c)))
}

/// Three or more single letters separated by whitespace.
pub(crate) fn letter_spaced(t: &str) -> bool {
    let single = |w: &str| {
        let mut c = w.chars();
        c.next().is_some_and(char::is_alphabetic) && c.next().is_none()
    };
    t.split_whitespace().count() >= 3 && t.split_whitespace().all(single)
}

/// A letter-spaced line as words when its text shows the word breaks: a run of two or more
/// whitespace chars (any: U+00A0, thin spaces) separates words, letters join. Other text, and a
/// letter-spaced line with one kind of separator only (words not known), as is.
pub(crate) fn despaced(t: &str) -> Cow<'_, str> {
    if !letter_spaced(t) {
        return Cow::Borrowed(t);
    }
    let mut words: Vec<String> = vec![String::new()];
    let mut blank = 0;
    for c in t.trim().chars() {
        if c.is_whitespace() {
            blank += 1;
            continue;
        }
        if blank >= 2 {
            words.push(String::new());
        }
        blank = 0;
        if let Some(w) = words.last_mut() {
            w.push(c);
        }
    }
    if words.len() < 2 {
        return Cow::Borrowed(t);
    }
    Cow::Owned(words.join(" "))
}

/// Gap between letters that separates words: this many times the median letter gap and wider
/// than it by this share of the font size.
const WORD_GAP_RATIO: f32 = 1.6;
const WORD_GAP_FS: f32 = 0.25;

/// Words of a letter-spaced line (`text`, its `spans`): the text's own word breaks
/// ([`despaced`]), else the glyph gaps (a gap [`WORD_GAP_RATIO`] times the median and
/// [`WORD_GAP_FS`] font sizes wider separates words). `None` when the line is not letter-spaced
/// or nothing tells the words apart.
pub(crate) fn spaced_words(text: &str, spans: &[&TextSpan]) -> Option<String> {
    if !letter_spaced(text) {
        return None;
    }
    if let Cow::Owned(w) = despaced(text) {
        return Some(w);
    }
    let size = spans.iter().map(|s| s.font_size).fold(0.0f32, f32::max);
    let glyphs: Vec<(char, f32, f32)> = spans
        .iter()
        .flat_map(|s| s.to_chars())
        .filter(|c| !c.char.is_whitespace())
        .map(|c| (c.char, c.bbox.x, c.bbox.x + c.bbox.width))
        .collect();
    if glyphs.len() < 3 {
        return None;
    }
    let gaps: Vec<f32> = glyphs.windows(2).map(|w| w[1].1 - w[0].2).collect();
    let mut sorted = gaps.clone();
    sorted.sort_by(f32::total_cmp);
    let m = sorted[sorted.len() / 2].max(0.0);
    let wide = |g: f32| g > WORD_GAP_RATIO * m && g > m + WORD_GAP_FS * size;
    if !gaps.iter().copied().any(wide) {
        return None;
    }
    let mut out = String::new();
    for (i, g) in glyphs.iter().enumerate() {
        if i > 0 && wide(gaps[i - 1]) {
            out.push(' ');
        }
        out.push(g.0);
    }
    Some(out)
}

/// The text a heading is written with: the words of a letter-spaced one-line paragraph (from
/// `lines`, the region's lines starting at `offset`), else [`despaced`] text.
pub(crate) fn title_text<'a>(p: &'a Para, lines: &'a [StyledLine], offset: usize) -> Cow<'a, str> {
    match lines
        .get(p.lines.start - offset)
        .and_then(|l| l.words.as_deref())
    {
        Some(w) if p.lines.len() == 1 => Cow::Borrowed(w),
        _ => despaced(&p.text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaced_letters_read_as_words() {
        // one kind of separator: the words are not known, the text stays
        assert_eq!(despaced("О П Ы Т Р А Б О Т Ы"), "О П Ы Т Р А Б О Т Ы");
        assert_eq!(despaced("О П Ы Т  Р А Б О Т Ы"), "ОПЫТ РАБОТЫ");
        assert_eq!(
            despaced("О\u{a0}П\u{a0}Ы\u{a0}Т\u{a0}\u{a0}Р\u{a0}А\u{a0}Б"),
            "ОПЫТ РАБ"
        );
        assert_eq!(despaced("О С\u{2009}\u{2009}Е Б Е"), "ОС ЕБЕ");
        assert_eq!(despaced("A B"), "A B");
        assert_eq!(despaced("Part I A"), "Part I A");
        assert_eq!(despaced("1 2 3"), "1 2 3");
    }

    fn line(text: &str, size: f32, bold: bool, color: (u8, u8, u8), gap: f32) -> StyledLine {
        StyledLine {
            text: text.into(),
            size,
            bold,
            font: "Helvetica".into(),
            color,
            bbox: pdf_oxide::geometry::Rect::new(40.0, 400.0, 80.0, size),
            gap,
            words: None,
        }
    }

    fn para(l: &StyledLine) -> Para {
        super::super::headings::paragraphs(std::slice::from_ref(l)).remove(0)
    }

    #[test]
    fn caps_line_in_bold_or_accent_colour_is_a_title_grey_label_is_not() {
        let blue = (31, 86, 168);
        let title = line("WORK EXPERIENCE", 8.0, false, blue, 0.0);
        assert!(caps_heading(&para(&title), &title, 8.9));
        let bold = line("К О Н Т А К Т Ы", 11.0, true, (34, 34, 34), 0.0);
        assert!(caps_heading(&para(&bold), &bold, 10.5));
        let label = line("PHONE", 7.4, false, (95, 104, 120), 0.0);
        assert!(!caps_heading(&para(&label), &label, 8.9), "grey label");
        let legend = line("OFTEN SOMETIMES", 9.7, true, (68, 65, 65), 40.0);
        assert!(
            !caps_heading(&para(&legend), &legend, 10.0),
            "wide gap: a legend row"
        );
        let long = line("THE QUICK BROWN FOX JUMPS", 11.0, true, (0, 0, 0), 0.0);
        assert!(!caps_heading(&para(&long), &long, 10.0), "five words");
    }

    #[test]
    fn a_caps_title_does_not_join_the_bold_line_under_it() {
        let mut a = line("EDUCATION", 11.0, true, (34, 34, 34), 0.0);
        let mut b = line("Test State University", 10.5, true, (34, 34, 34), 0.0);
        a.bbox.y = 400.0;
        b.bbox.y = 386.0;
        assert!(case_break(&a, &b));
        assert_eq!(super::super::headings::paragraphs(&[a, b]).len(), 2);
    }

    /// Letters as separate spans, `gaps[i]` pt after letter `i`.
    fn spaced_line(letters: &str, gaps: &[f32]) -> StyledLine {
        let mut x = 40.0;
        let spans: Vec<TextSpan> = letters
            .chars()
            .enumerate()
            .map(|(i, c)| {
                let s = TextSpan {
                    text: c.to_string(),
                    bbox: pdf_oxide::geometry::Rect::new(x, 400.0, 7.0, 11.0),
                    font_size: 11.0,
                    ..Default::default()
                };
                x += 7.0 + gaps.get(i).copied().unwrap_or(0.0);
                s
            })
            .collect();
        super::super::headings::styled_lines(&spans).remove(0)
    }

    #[test]
    fn letter_spaced_words_from_the_glyph_gaps_text_kept_when_unknown() {
        let l = spaced_line("ОСЕБЕ", &[12.0, 4.0, 4.0, 4.0]);
        assert_eq!(l.text, "О С Е Б Е", "the line text is not changed");
        assert_eq!(l.words.as_deref(), Some("О СЕБЕ"));
        let l = spaced_line(
            "ОПЫТРАБОТЫ",
            &[4.0, 4.0, 4.0, 13.0, 4.0, 4.0, 4.0, 4.0, 4.0],
        );
        assert_eq!(l.words.as_deref(), Some("ОПЫТ РАБОТЫ"));
        let l = spaced_line("ОСЕБЕ", &[4.0; 4]);
        assert_eq!(l.words, None, "uniform gaps: words unknown");
        let p = super::super::headings::paragraphs(std::slice::from_ref(&l)).remove(0);
        assert_eq!(title_text(&p, std::slice::from_ref(&l), 0), "О С Е Б Е");
    }

    #[test]
    fn links_and_repository_names_are_not_titles() {
        for t in [
            "sample-cli Northwind Samples",
            "github.com/jane",
            "owner/repo",
            "jane@example.test",
            "https://example.test/x",
            "my_tool",
        ] {
            assert!(link_shaped(t), "{t}");
        }
        for t in [
            "EXPERIENCE",
            "3.1 Introduction",
            "Results and discussion",
            "Part I: Play",
            "E-commerce strategy",
            "Input/Output",
        ] {
            assert!(!link_shaped(t), "{t}");
        }
    }
}
