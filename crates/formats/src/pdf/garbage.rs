//! Mojibake score for unreadable PDF text layers.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GarbageScore {
    pub tokens: usize,
    pub ratio: f32,
}

const VOWELS: &str = "aeiouyAEIOUYаеёиоуыэюяАЕЁИОУЫЭЮЯіїєІЇЄ";
const CODE_MARKERS: &str = "\"'()=.:;<>/";
const SUSPECT_PUNCT: &str = "^\\[]{}|~`";

fn is_latin_or_cyrillic(c: char) -> bool {
    c.is_ascii_alphabetic() || ('\u{0400}'..='\u{04FF}').contains(&c)
}

fn is_private_use(c: char) -> bool {
    ('\u{E000}'..='\u{F8FF}').contains(&c)
}

/// None = token not scored; Some(true) = suspect.
fn token_verdict(tok: &str) -> Option<bool> {
    // Tokens come from `split_whitespace`, so whitespace controls (tab, newline) never reach here.
    let mut chars = tok.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && is_private_use(c)
    {
        // A lone private-use char is a Symbol-font bullet (Word/PowerPoint U+F0B7), not mojibake.
        return None;
    }
    if tok
        .chars()
        .any(|c| c == '\u{FFFD}' || is_private_use(c) || c.is_control())
    {
        return Some(true);
    }
    let t = tok.trim_matches(|c: char| !c.is_alphanumeric());
    if t.chars().any(|c| CODE_MARKERS.contains(c)) {
        return None;
    }
    let letters: Vec<char> = t.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() < 3 || t.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    if !letters.iter().all(|c| is_latin_or_cyrillic(*c)) {
        return None;
    }
    if t.chars().any(|c| SUSPECT_PUNCT.contains(c)) {
        return Some(true);
    }
    if letters.iter().all(|c| c.is_uppercase()) {
        return Some(false);
    }
    Some(!letters.iter().any(|c| VOWELS.contains(*c)))
}

/// Vowel-less Czech/Slovak words (smrt, vlk) are a known false-positive source.
pub(crate) fn garbage_score(text: &str) -> GarbageScore {
    let (tokens, bad) = text
        .split_whitespace()
        .filter_map(token_verdict)
        .fold((0usize, 0usize), |(n, bad), suspect| {
            (n + 1, bad + usize::from(suspect))
        });
    GarbageScore {
        tokens,
        ratio: if tokens == 0 {
            0.0
        } else {
            bad as f32 / tokens as f32
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOJIBAKE: &str = "= h ^h\\ hch lqzld h fiZgbb Wl hl _kl h\\uc^hd m f_glgZj m qlh i_j\\Z y dhehgdZ \
        Wnn_dlbgghklv jZ[hlu hl^_eZ ujhkeZ ^eZ]h^Zjy ghuf ijhp_kkZf b ZlhfzlbaZpbb";

    #[test]
    fn mojibake_scores_high() {
        let s = garbage_score(&MOJIBAKE.repeat(2)); // one copy = 16 scored tokens, ratio 0.625
        assert!(s.tokens >= 20, "{s:?}");
        assert!(s.ratio >= 0.20, "{s:?}");
    }

    #[test]
    fn normal_russian_and_english_score_low() {
        let ru = "Это тестовый документ на русском языке: Съешь ещё этих мягких французских булок, да выпей чаю. \
                  Эффективность работы отдела выросла благодаря новым процессам и автоматизации рутинных задач.";
        let en = "The code accompanying this chapter offers options to generate confidence intervals on fitted \
                  parameters. Confidence intervals sometimes form the basis of statistical inference.";
        assert!(garbage_score(ru).ratio < 0.05);
        assert!(garbage_score(en).ratio < 0.05);
    }

    #[test]
    fn other_scripts_and_code_are_not_flagged() {
        assert_eq!(
            garbage_score("Η γρήγορη καφέ αλεπού πηδάει πάνω από τον τεμπέλη σκύλο").tokens,
            0
        );
        assert_eq!(
            garbage_score("這是一個測試文件內容 これはテストです").tokens,
            0
        );
        let code = "let max_page_count = config.pages_total; fn parse_page(doc: &PdfDocument, idx: usize) \
                    -> Result<Vec<Block>, ParseError> { snake_case_ident another_value }";
        assert_eq!(garbage_score(code).ratio, 0.0);
    }

    #[test]
    fn replacement_and_private_use_chars_are_suspect() {
        let s = garbage_score("ab\u{FFFD}cd \u{E001}x");
        assert_eq!((s.tokens, s.ratio), (2, 1.0));
        let pua = "\u{E010}\u{E021}\u{E033} ".repeat(25);
        let s = garbage_score(&pua);
        assert_eq!((s.tokens, s.ratio), (25, 1.0));
    }

    #[test]
    fn lone_private_use_bullets_are_not_scored() {
        let page = "\u{F0B7} Revenue growth was strong this quarter\n".repeat(6);
        let s = garbage_score(&page);
        assert_eq!(s.ratio, 0.0, "{s:?}");
    }

    #[test]
    fn acronyms_and_numbers_are_not_suspect() {
        assert_eq!(garbage_score("NATO HTML CSS TCP 2024 3D x86").ratio, 0.0);
    }

    #[test]
    fn code_with_brackets_and_quotes_is_not_flagged() {
        let code = r#"
def collect(cfg, queue):
    result = []
    for idx in range(len(queue)):
        item = queue[idx]
        if item["status"] == "ok":
            result.append(item["value"])
        elif cfg["scale"]:
            result.append(item["value"] * cfg["scale"])
    return {"items": result, "count": len(result)}
"#;
        let s = garbage_score(code);
        assert!(s.ratio < 0.20, "{s:?}");
    }
}
