//! Conservative recognition post-fixes. Only Latin->Cyrillic look-alikes in a Cyrillic context.

/// Latin letters whose glyphs match Cyrillic ones in common fonts, with their Cyrillic twin.
const HOMOGLYPHS: [(char, char); 19] = [
    ('A', 'А'),
    ('B', 'В'),
    ('C', 'С'),
    ('E', 'Е'),
    ('H', 'Н'),
    ('K', 'К'),
    ('M', 'М'),
    ('O', 'О'),
    ('P', 'Р'),
    ('T', 'Т'),
    ('X', 'Х'),
    ('Y', 'У'),
    ('a', 'а'),
    ('c', 'с'),
    ('e', 'е'),
    ('o', 'о'),
    ('p', 'р'),
    ('x', 'х'),
    ('y', 'у'),
];

fn twin(c: char) -> Option<char> {
    HOMOGLYPHS.iter().find(|(l, _)| *l == c).map(|(_, k)| *k)
}

fn is_cyr(c: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&c)
}

/// Rewrites look-alike Latin letters to Cyrillic in one line of OCR text.
/// A word is rewritten when all its Latin letters have Cyrillic twins and either the word already
/// has a Cyrillic letter, or it has no Cyrillic letter but at least `min_line_cyr` (0.6) of the
/// line's letters outside this word are Cyrillic. Words with any other Latin letter are kept.
pub fn fix_homoglyphs(line: &str, min_line_cyr: f32) -> String {
    let words: Vec<&str> = line.split(' ').collect();
    let letters = |w: &str| w.chars().filter(|c| c.is_alphabetic()).count();
    let cyr = |w: &str| w.chars().filter(|c| is_cyr(*c)).count();
    let (tot_l, tot_c): (usize, usize) = words
        .iter()
        .fold((0, 0), |(l, c), w| (l + letters(w), c + cyr(w)));
    words
        .iter()
        .map(|w| {
            let latin: Vec<char> = w.chars().filter(|c| c.is_ascii_alphabetic()).collect();
            if latin.is_empty() || latin.iter().any(|c| twin(*c).is_none()) {
                return (*w).to_string();
            }
            let (ol, oc) = (tot_l - letters(w), tot_c - cyr(w));
            let ctx = cyr(w) > 0 || (ol > 0 && oc as f32 / ol as f32 >= min_line_cyr);
            if !ctx {
                return (*w).to_string();
            }
            w.chars().map(|c| twin(c).unwrap_or(c)).collect()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Index just past a money-like token `\d{1,3}( \d{3})*\.\d{2}` starting at `i`, if any.
fn money_dot_end(c: &[char], i: usize) -> Option<usize> {
    let digits = |from: usize, n: usize| {
        (from..from + n).all(|k| c.get(k).is_some_and(|d| d.is_ascii_digit()))
    };
    let not_digit = |k: usize| !c.get(k).is_some_and(|d| d.is_ascii_digit());
    let mut j = i;
    while j < i + 3 && c.get(j).is_some_and(|d| d.is_ascii_digit()) {
        j += 1;
    }
    if j == i {
        return None;
    }
    while c.get(j) == Some(&' ') && digits(j + 1, 3) && not_digit(j + 4) {
        j += 4;
    }
    (c.get(j) == Some(&'.')
        && digits(j + 1, 2)
        && not_digit(j + 3)
        && !matches!(c.get(j + 3), Some('.' | ',')))
    .then_some(j + 3)
}

/// True when the line has a `\d+,\d{2}` number.
fn has_money_comma(c: &[char]) -> bool {
    c.windows(4).enumerate().any(|(i, w)| {
        w[0].is_ascii_digit()
            && w[1] == ','
            && w[2].is_ascii_digit()
            && w[3].is_ascii_digit()
            && !c.get(i + 4).is_some_and(|d| d.is_ascii_digit())
    })
}

/// Turns `1 250.75` into `1 250,75`, only when the same line already has a `\d+,\d{2}` number
/// (evidence that the document uses the decimal comma). Off by default in the pipeline.
pub fn fix_decimal_comma(line: &str) -> String {
    let c: Vec<char> = line.chars().collect();
    if !has_money_comma(&c) {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < c.len() {
        let starts =
            c[i].is_ascii_digit() && (i == 0 || !matches!(c[i - 1], '0'..='9' | '.' | ','));
        if let Some(end) = starts.then(|| money_dot_end(&c, i)).flatten() {
            out.extend(&c[i..end - 3]);
            out.push(',');
            out.extend(&c[end - 2..end]);
            i = end;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_word_is_fixed() {
        assert_eq!(fix_homoglyphs("Bыпуск нoмер", 0.6), "Выпуск номер");
    }

    #[test]
    fn all_latin_lookalike_word_needs_cyrillic_line() {
        assert_eq!(fix_homoglyphs("CKP по отрасли", 0.6), "СКР по отрасли");
        assert_eq!(fix_homoglyphs("CKP report", 0.6), "CKP report");
        assert_eq!(fix_homoglyphs("CKP", 0.6), "CKP", "no context");
    }

    #[test]
    fn real_latin_words_and_numbers_are_kept() {
        assert_eq!(
            fix_homoglyphs("Microsoft Word в отчёте", 0.6),
            "Microsoft Word в отчёте"
        );
        assert_eq!(
            fix_homoglyphs("ГОСТ 7.32-2017 стр. 5", 0.6),
            "ГОСТ 7.32-2017 стр. 5"
        );
    }

    #[test]
    fn decimal_comma_only_with_a_comma_number_on_the_line() {
        assert_eq!(
            fix_decimal_comma("Итого 1 250.75 из 3 000,00"),
            "Итого 1 250,75 из 3 000,00"
        );
        assert_eq!(
            fix_decimal_comma("п. 4.2 и 1250.75"),
            "п. 4.2 и 1250.75",
            "no comma evidence"
        );
        assert_eq!(
            fix_decimal_comma("ГОСТ 7.32 и 2,5"),
            "ГОСТ 7.32 и 2,5",
            "not a money-like number"
        );
    }

    #[test]
    fn dates_and_dotted_chains_are_not_money() {
        let line = "от 01.02.2024 на 1 250,00";
        assert_eq!(fix_decimal_comma(line), line);
    }
}
