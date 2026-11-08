//! Builders for detector unit tests.

use super::prims::Word;
use super::{InputPath, InputWord, PageInput, Rect};

/// Internal word of width `6 * chars` and height `fs`, top at `top`.
pub(crate) fn word(t: &str, x: f64, top: f64, fs: f64) -> Word {
    Word::from_input(&input_word(t, x, top, fs))
}

pub(crate) fn input_word(t: &str, x: f64, top: f64, fs: f64) -> InputWord {
    InputWord {
        rect: Rect::new(x, top, 6.0 * t.chars().count() as f64, fs),
        text: t.into(),
        font_size: fs,
        char_x: Vec::new(),
        rotated: false,
    }
}

/// Word with per-char positions, each char `cw` wide.
pub(crate) fn glued(t: &str, x: f64, top: f64, cw: f64) -> InputWord {
    let n = t.chars().count();
    InputWord {
        rect: Rect::new(x, top, cw * n as f64, 10.0),
        text: t.into(),
        font_size: 10.0,
        char_x: (0..n)
            .map(|i| (x + cw * i as f64, x + cw * (i + 1) as f64))
            .collect(),
        rotated: false,
    }
}

/// 0.5-pt filled ruling from `a` to `b` at `y`.
pub(crate) fn hline(a: f64, b: f64, y: f64) -> InputPath {
    InputPath {
        rect: Rect::new(a, y - 0.25, b - a, 0.5),
        stroke: false,
        fill: true,
    }
}

pub(crate) fn vline(x: f64, a: f64, b: f64) -> InputPath {
    InputPath {
        rect: Rect::new(x - 0.25, a, 0.5, b - a),
        stroke: false,
        fill: true,
    }
}

/// Page 600×800 with full rulings at `xs` × `ys` and words `(text, x, top)` of size 10.
pub(crate) fn grid_page(xs: &[f64], ys: &[f64], words: &[(&str, f64, f64)]) -> PageInput {
    let mut p = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    if let (Some(&x0), Some(&x1), Some(&y0), Some(&y1)) =
        (xs.first(), xs.last(), ys.first(), ys.last())
    {
        p.paths.extend(ys.iter().map(|&y| hline(x0, x1, y)));
        p.paths.extend(xs.iter().map(|&x| vline(x, y0, y1)));
    }
    p.words
        .extend(words.iter().map(|&(t, x, top)| input_word(t, x, top, 10.0)));
    p
}
