//! Text-layer quality features (port of ocr-need `pagefeat.text_features`, char/word part).
//!
//! Deviations from the Python reference, by design: PUA/U+FFFD (`pua_share`) and control chars
//! (`ctrl_share`) are counted separately (the reference lumps them into `pua_share`); `char` Unicode
//! predicates replace Python's `isalpha`/`isdigit` (identical on Latin, Cyrillic and ASCII digits).

use super::bigrams::Bigrams;

/// Dominant script of the text layer, chooses the bigram table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lang {
    Ru,
    En,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TextFeatures {
    /// Non-whitespace chars.
    pub nonws: usize,
    pub letter_share: f32,
    pub digit_share: f32,
    /// `1 - (letters + digits) / nonws`.
    pub punct_share: f32,
    pub cyr_share: f32,
    pub lat_share: f32,
    /// Private-use (U+E000..U+F8FF) and U+FFFD.
    pub pua_share: f32,
    /// Control chars below U+0020 that are not whitespace (NUL padding).
    pub ctrl_share: f32,
    pub upper_share: f32,
    pub n_words: usize,
    /// Words with >= 4 letters and no digit.
    pub n_alpha_words4: usize,
    /// Share of words with >= 3 letters that have no ru/en vowel.
    pub novowel_share: f32,
    pub lang: Option<Lang>,
    /// Mean log10 P(next | prev) over in-word letter pairs; `None` without pairs or without a
    /// bigram table.
    pub bigram_lp: Option<f32>,
    /// Letter pairs behind `bigram_lp`; 0 without a bigram table.
    pub bigram_n: usize,
}

/// Python `str.isspace` (adds U+001C..U+001F to `char::is_whitespace`).
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

fn is_cyr(c: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&c)
}

const VOWELS: &str = "аеёиоуыэюяaeiouy";

/// Mean log10 P over consecutive in-alphabet letters (lowercased, "ё" -> "е"); any other char
/// breaks the chain.
fn bigram(text: &str, lang: Lang, big: &Bigrams) -> (Option<f32>, usize) {
    let (alpha, table): (&[char], &dyn Fn(usize, usize) -> f32) = match lang {
        Lang::Ru => (&big.ru_alphabet, &|a, b| big.ru_logp[a][b]),
        Lang::En => (&big.en_alphabet, &|a, b| big.en_logp[a][b]),
    };
    let (mut s, mut n, mut prev) = (0f64, 0usize, None);
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = if c == 'ё' { 'е' } else { c };
        match alpha.iter().position(|a| *a == c) {
            None => prev = None,
            Some(i) => {
                if let Some(p) = prev {
                    s += table(p, i) as f64;
                    n += 1;
                }
                prev = Some(i);
            }
        }
    }
    ((n > 0).then(|| (s / n as f64) as f32), n)
}

impl TextFeatures {
    /// Features of `text`. Without `big` the bigram features stay empty (`bigram_lp = None`,
    /// `bigram_n = 0`) and the classifier decides on the remaining features.
    pub(crate) fn from_text(text: &str, big: Option<&Bigrams>) -> Self {
        let nonws: Vec<char> = text.chars().filter(|c| !py_space(*c)).collect();
        let n = nonws.len();
        let mut f = TextFeatures {
            nonws: n,
            ..Default::default()
        };
        let words: Vec<&str> = text.split(py_space).filter(|w| !w.is_empty()).collect();
        f.n_words = words.len();
        let alpha = |w: &str| w.chars().filter(|c| c.is_alphabetic()).count();
        f.n_alpha_words4 = words
            .iter()
            .filter(|w| alpha(w) >= 4 && !w.chars().any(char::is_numeric))
            .count();
        if n > 0 {
            let cnt = |p: &dyn Fn(char) -> bool| nonws.iter().filter(|c| p(**c)).count();
            let letters = cnt(&|c| c.is_alphabetic());
            let digits = cnt(&|c| c.is_numeric());
            let share = |k: usize| k as f32 / n as f32;
            f.letter_share = share(letters);
            f.digit_share = share(digits);
            f.punct_share = 1.0 - share(letters + digits);
            f.cyr_share = share(cnt(&is_cyr));
            f.lat_share = share(cnt(&|c| c.is_ascii_alphabetic()));
            f.pua_share = share(cnt(&|c| {
                ('\u{E000}'..='\u{F8FF}').contains(&c) || c == '\u{FFFD}'
            }));
            f.ctrl_share = share(cnt(&|c| (c as u32) < 32));
            let up = cnt(&char::is_uppercase);
            f.upper_share = if letters > 0 {
                up as f32 / letters as f32
            } else {
                0.0
            };
        }
        let long: Vec<&&str> = words.iter().filter(|w| alpha(w) >= 3).collect();
        if !long.is_empty() {
            let nov = long
                .iter()
                .filter(|w| {
                    !w.chars()
                        .flat_map(char::to_lowercase)
                        .any(|c| VOWELS.contains(c))
                })
                .count();
            f.novowel_share = nov as f32 / long.len() as f32;
        }
        let lang = if f.cyr_share >= f.lat_share {
            Lang::Ru
        } else {
            Lang::En
        };
        f.lang = Some(lang);
        if let Some(big) = big {
            (f.bigram_lp, f.bigram_n) = bigram(text, lang, big);
        }
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_prose_is_clean() {
        const T: &str = "Настоящий порядок определяет правила предоставления субсидий.";
        let f = TextFeatures::from_text(T, None);
        assert_eq!(f.lang, Some(Lang::Ru));
        assert!(f.letter_share > 0.95 && f.novowel_share == 0.0);
        assert_eq!((f.bigram_lp, f.bigram_n), (None, 0));
        if let Some(big) = Bigrams::bundled() {
            assert!(TextFeatures::from_text(T, Some(&big)).bigram_lp.unwrap() > -1.3);
        }
    }

    #[test]
    fn pua_and_control_are_separate() {
        let f = TextFeatures::from_text("\u{E001}\u{E002}ab\u{0}\u{1}", None);
        assert!((f.pua_share - 2.0 / 6.0).abs() < 1e-6);
        assert!((f.ctrl_share - 2.0 / 6.0).abs() < 1e-6);
    }

    #[test]
    fn empty_text_has_no_bigram() {
        let f = TextFeatures::from_text("  \n ", None);
        assert_eq!((f.nonws, f.n_words, f.bigram_lp), (0, 0, None));
    }
}
