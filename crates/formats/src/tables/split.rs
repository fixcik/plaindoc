//! Splitting words glued across a vertical ruling (`VALID|TO` → `VALIDTO`), by char positions.
//! Not in the prototype: off in [`super::TableConfig::reference`].

use super::PageInput;
use super::prims::{Prims, Word, regions};
use super::{InputWord, Rect};

/// A ruling this far inside a word cuts it.
const CUT_MARGIN: f64 = 0.5;

/// Words of the page with every word crossing a vertical ruling of a lined region (a table grid, not a
/// figure line) cut at that ruling.
pub(crate) fn split_glued(page: &PageInput, p: &Prims) -> Vec<Word> {
    let regs = regions(p);
    let vs: Vec<_> =
        p.vs.iter()
            .filter(|v| {
                regs.iter()
                    .any(|r| r.x0 <= v.x && v.x <= r.x1 && r.y0 - 3.0 <= v.a && v.b <= r.y1 + 3.0)
            })
            .collect();
    let mut out = Vec::with_capacity(page.words.len());
    for w in &page.words {
        let n = w.text.chars().count();
        let (x0, x1) = (w.rect.x, w.rect.x + w.rect.w);
        let cy = w.rect.top + w.rect.h / 2.0;
        let mut cuts: Vec<f64> = vs
            .iter()
            .filter(|v| v.a <= cy && cy <= v.b && x0 + CUT_MARGIN < v.x && v.x < x1 - CUT_MARGIN)
            .map(|v| v.x)
            .collect();
        if cuts.is_empty() || w.char_x.len() != n || n < 2 {
            out.push(Word::from_input(w));
            continue;
        }
        cuts.sort_by(f64::total_cmp);
        let chars: Vec<char> = w.text.chars().collect();
        let mut piece: Vec<usize> = Vec::new();
        let mut k = 0;
        let flush = |piece: &mut Vec<usize>, out: &mut Vec<Word>| {
            if piece.is_empty() {
                return;
            }
            let a = w.char_x[piece[0]].0;
            let b = w.char_x[piece[piece.len() - 1]].1;
            let text: String = piece.iter().map(|&i| chars[i]).collect();
            piece.clear();
            if text.trim().is_empty() {
                return;
            }
            out.push(Word::from_input(&InputWord {
                rect: Rect::new(a, w.rect.top, b - a, w.rect.h),
                text,
                font_size: w.font_size,
                char_x: Vec::new(),
                rotated: w.rotated,
            }));
        };
        for i in 0..n {
            let mid = (w.char_x[i].0 + w.char_x[i].1) / 2.0;
            while k < cuts.len() && mid > cuts[k] {
                flush(&mut piece, &mut out);
                k += 1;
            }
            piece.push(i);
        }
        flush(&mut piece, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::prims::prims;
    use crate::tables::testkit::{glued, vline};

    #[test]
    fn word_crossing_a_ruling_is_cut_at_char_boundary() {
        let mut page = PageInput {
            width: 600.0,
            height: 800.0,
            ..Default::default()
        };
        page.words.push(glued("VALIDTO", 100.0, 200.0, 7.0));
        page.paths.push(vline(135.5, 190.0, 215.0)); // between 'D' (x 128..135) and 'T' (135..142)
        page.paths.push(vline(80.0, 190.0, 215.0)); // the cell grid around it
        page.paths.push(vline(200.0, 190.0, 215.0));
        page.paths
            .push(crate::tables::testkit::hline(80.0, 200.0, 190.0));
        page.paths
            .push(crate::tables::testkit::hline(80.0, 200.0, 215.0));
        let w = split_glued(&page, &prims(&page));
        assert_eq!(
            w.iter().map(|w| w.t.as_str()).collect::<Vec<_>>(),
            ["VALID", "TO"]
        );
        assert!(w[0].x1 <= 135.5 && w[1].x0 >= 135.0);
    }

    #[test]
    fn words_without_chars_or_rulings_are_kept() {
        let mut page = PageInput {
            width: 600.0,
            height: 800.0,
            ..Default::default()
        };
        page.words.push(glued("VALIDTO", 100.0, 200.0, 7.0));
        assert_eq!(split_glued(&page, &prims(&page)).len(), 1);
        page.paths.push(vline(135.5, 190.0, 215.0));
        assert_eq!(
            split_glued(&page, &prims(&page)).len(),
            1,
            "a lone line (figure) is not a cell border"
        );
        page.paths.push(vline(80.0, 190.0, 215.0));
        page.paths.push(vline(200.0, 190.0, 215.0));
        page.paths
            .push(crate::tables::testkit::hline(80.0, 200.0, 190.0));
        page.words[0].char_x.clear();
        assert_eq!(
            split_glued(&page, &prims(&page)).len(),
            1,
            "no char positions: cannot cut"
        );
    }
}
