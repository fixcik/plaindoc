//! Candidate features (proto.py `features`), rounded to 4 decimals like the prototype output on which
//! the logistic model was fitted. Only features used by the D11 rules and the model are computed.

use super::PageInput;
use super::detect::Raw;
use super::grid::text_lines;
use super::prims::{Prims, Word};
use super::stats::{cluster, mean, median, pstdev, round4};

/// Share of numeric cells that makes a column numeric (proto.py `numcols`).
const NUMCOL_SHARE: f64 = 0.6;
/// Paths/segments within this slack of the bbox count as inside.
const BOX_SLACK: f64 = 3.0;

/// Features of one candidate (names as in `filters_logit.json`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Features {
    pub nrows: f64,
    pub ncols: f64,
    pub fill: f64,
    pub mean_words: f64,
    pub num_frac: f64,
    pub numcols: f64,
    pub numcol_frac: f64,
    pub sparse_rows: f64,
    pub line_w_mean: f64,
    pub nlines: f64,
    pub n_h: f64,
    pub n_v: f64,
    pub h_len: f64,
    pub v_len: f64,
    pub n_paths: f64,
    pub n_segs: f64,
    pub n_diag: f64,
    pub fill_area: f64,
    pub paths_per_word: f64,
    pub row_h_cv: f64,
    pub col_w_cv: f64,
    pub align_cv: f64,
    pub w_frac: f64,
    pub h_frac: f64,
    pub fs_ratio: f64,
    pub first_col_words: f64,
    pub last_col_num: f64,
    pub header_like: f64,
    pub word_share: f64,
    pub nw: f64,
    pub short_frac: f64,
    pub row_h_fs: f64,
}

impl Features {
    /// `(name, value)` pairs, for golden comparisons and debugging.
    pub fn named(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("nrows", self.nrows),
            ("ncols", self.ncols),
            ("fill", self.fill),
            ("mean_words", self.mean_words),
            ("num_frac", self.num_frac),
            ("numcols", self.numcols),
            ("numcol_frac", self.numcol_frac),
            ("sparse_rows", self.sparse_rows),
            ("line_w_mean", self.line_w_mean),
            ("nlines", self.nlines),
            ("n_h", self.n_h),
            ("n_v", self.n_v),
            ("h_len", self.h_len),
            ("v_len", self.v_len),
            ("n_paths", self.n_paths),
            ("n_segs", self.n_segs),
            ("n_diag", self.n_diag),
            ("fill_area", self.fill_area),
            ("paths_per_word", self.paths_per_word),
            ("row_h_cv", self.row_h_cv),
            ("col_w_cv", self.col_w_cv),
            ("align_cv", self.align_cv),
            ("w_frac", self.w_frac),
            ("h_frac", self.h_frac),
            ("fs_ratio", self.fs_ratio),
            ("first_col_words", self.first_col_words),
            ("last_col_num", self.last_col_num),
            ("header_like", self.header_like),
            ("word_share", self.word_share),
            ("nw", self.nw),
            ("short_frac", self.short_frac),
            ("row_h_fs", self.row_h_fs),
        ]
    }
}

fn is_digit(c: char) -> bool {
    // Python `\d` on str: Unicode decimal digits; ASCII, Arabic-Indic and fullwidth cover the corpora.
    c.is_ascii_digit()
        || ('\u{660}'..='\u{669}').contains(&c)
        || ('\u{ff10}'..='\u{ff19}').contains(&c)
}

/// proto.py `NUM`: `^[\(\[]?[-+−–]?[$€£¥]?\d[\d,.\s]*%?[\)\]]?[*†‡a-z]?$` on the stripped text.
pub(crate) fn is_num(s: &str) -> bool {
    let c: Vec<char> = s.trim().chars().collect();
    if c.is_empty() {
        return false;
    }
    let mut i = 0;
    let opt = |set: &dyn Fn(char) -> bool, i: &mut usize, c: &[char]| {
        if *i < c.len() && set(c[*i]) {
            *i += 1;
        }
    };
    opt(&|x| x == '(' || x == '[', &mut i, &c);
    opt(&|x| "-+−–".contains(x), &mut i, &c);
    opt(&|x| "$€£¥".contains(x), &mut i, &c);
    if i >= c.len() || !is_digit(c[i]) {
        return false;
    }
    i += 1;
    while i < c.len() && (is_digit(c[i]) || c[i] == ',' || c[i] == '.' || c[i].is_whitespace()) {
        i += 1;
    }
    opt(&|x| x == '%', &mut i, &c);
    opt(&|x| x == ')' || x == ']', &mut i, &c);
    opt(&|x| "*†‡".contains(x) || x.is_ascii_lowercase(), &mut i, &c);
    i == c.len()
}

fn words_of(s: &str) -> f64 {
    s.split_whitespace().count() as f64
}

fn cv(v: &[f64]) -> f64 {
    if v.len() > 1 && mean(v) != 0.0 {
        pstdev(v) / mean(v)
    } else {
        0.0
    }
}

/// `page_fs` is the median font size of `ws_all` (`None` when the page has no words), computed once per
/// page by the caller.
pub(crate) fn features(
    t: &Raw,
    page: &PageInput,
    ws_all: &[Word],
    page_fs: Option<f64>,
    p: &Prims,
) -> Features {
    let [x0, y0, x1, y1] = t.bbox;
    let (w, h) = ((x1 - x0).max(1e-6), (y1 - y0).max(1e-6));
    let rows = &t.rows;
    let nr = rows.len();
    let nc = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let cell = |r: usize, j: usize| rows[r].get(j).map_or("", String::as_str);
    let ne: Vec<&str> = rows
        .iter()
        .flatten()
        .map(String::as_str)
        .filter(|c| !c.trim().is_empty())
        .collect();
    let wc: Vec<f64> = ne.iter().map(|c| words_of(c)).collect();
    let rw = &t.words;
    let mut f = Features {
        nrows: nr as f64,
        ncols: nc as f64,
        fill: ne.len() as f64 / (nr * nc).max(1) as f64,
        mean_words: if wc.is_empty() { 0.0 } else { mean(&wc) },
        num_frac: ne.iter().filter(|c| is_num(c)).count() as f64 / ne.len().max(1) as f64,
        short_frac: wc.iter().filter(|&&x| x <= 3.0).count() as f64 / wc.len().max(1) as f64,
        nw: rw.len() as f64,
        w_frac: w / page.width,
        h_frac: h / page.height,
        ..Default::default()
    };
    let numcols = (0..nc)
        .filter(|&j| {
            let col: Vec<&str> = (0..nr)
                .map(|r| cell(r, j))
                .filter(|c| !c.trim().is_empty())
                .collect();
            !col.is_empty()
                && col.iter().filter(|c| is_num(c)).count() as f64 / col.len() as f64
                    >= NUMCOL_SHARE
        })
        .count();
    f.numcols = numcols as f64;
    f.numcol_frac = numcols as f64 / nc as f64;
    f.sparse_rows = (0..nr)
        .filter(|&r| (0..nc).filter(|&j| !cell(r, j).trim().is_empty()).count() <= 1)
        .count() as f64
        / nr as f64;
    let lines = text_lines(rw);
    f.nlines = lines.len() as f64;
    let lw: Vec<f64> = lines
        .iter()
        .map(|l| {
            let a = l.w.iter().map(|w| w.x1).fold(f64::MIN, f64::max);
            let b = l.w.iter().map(|w| w.x0).fold(f64::MAX, f64::min);
            (a - b) / w
        })
        .collect();
    f.line_w_mean = if lw.is_empty() { 0.0 } else { mean(&lw) };
    let fs: Vec<f64> = rw.iter().map(|w| w.fs).collect();
    let fs_med = if fs.is_empty() { 0.0 } else { median(&fs) };
    let pfs = page_fs.unwrap_or(fs_med);
    f.fs_ratio = if pfs != 0.0 { fs_med / pfs } else { 1.0 };

    let s = BOX_SLACK;
    let inh: Vec<_> =
        p.hs.iter()
            .filter(|l| y0 - s <= l.y && l.y <= y1 + s && l.b > x0 - s && l.a < x1 + s)
            .collect();
    let inv: Vec<_> =
        p.vs.iter()
            .filter(|l| x0 - s <= l.x && l.x <= x1 + s && l.b > y0 - s && l.a < y1 + s)
            .collect();
    f.n_h = cluster(&inh.iter().map(|l| l.y).collect::<Vec<_>>(), 2.0).len() as f64;
    f.n_v = cluster(&inv.iter().map(|l| l.x).collect::<Vec<_>>(), 2.0).len() as f64;
    f.h_len = inh.iter().map(|l| l.b.min(x1) - l.a.max(x0)).sum::<f64>() / (w * (nr + 1) as f64);
    f.v_len = inv.iter().map(|l| l.b.min(y1) - l.a.max(y0)).sum::<f64>() / (h * (nc + 1) as f64);

    let (mut np, mut fill_area) = (0usize, 0.0f64);
    for q in &page.paths {
        let (x, top, qw, qh) = (q.rect.x, q.rect.top, q.rect.w, q.rect.h);
        if x + qw < x0 - s || x > x1 + s || top + qh < y0 - s || top > y1 + s {
            continue;
        }
        np += 1;
        if q.fill && qw > 2.5 && qh > 2.5 {
            fill_area += ((x + qw).min(x1) - x.max(x0)) * ((top + qh).min(y1) - top.max(y0));
        }
    }
    let in_box = |g: &&super::InputSegment| {
        x0 - s <= g.x1.min(g.x2)
            && g.x1.max(g.x2) <= x1 + s
            && y0 - s <= g.y1.min(g.y2)
            && g.y1.max(g.y2) <= y1 + s
    };
    let segs: Vec<_> = page.segments.iter().filter(in_box).collect();
    f.n_paths = np as f64;
    f.fill_area = (fill_area / (w * h)).min(1.0);
    f.n_segs = segs.len() as f64;
    f.paths_per_word = (np + segs.len()) as f64 / rw.len().max(1) as f64;
    f.n_diag = segs
        .iter()
        .filter(|g| (g.x1 - g.x2).abs() > 1.0 && (g.y1 - g.y2).abs() > 1.0)
        .count() as f64;

    let (rb, cb) = (&t.rowb, &t.colb);
    let rh: Vec<f64> = rb.windows(2).map(|x| x[1] - x[0]).collect();
    let cw: Vec<f64> = cb.windows(2).map(|x| x[1] - x[0]).collect();
    f.row_h_cv = cv(&rh);
    f.col_w_cv = cv(&cw);
    let row_h_med = if rh.is_empty() { 0.0 } else { median(&rh) };
    f.row_h_fs = if fs_med != 0.0 {
        row_h_med / fs_med
    } else {
        0.0
    };
    // Quirk kept for parity: `nc`/`nr` count kept columns/rows, `cb`/`rb` are the bounds before
    // `finalize` dropped empty ones.
    let mut al = Vec::new();
    for j in 0..nc.min(cb.len().saturating_sub(1)) {
        let mut xs = Vec::new();
        let col: Vec<&&Word> = rw
            .iter()
            .filter(|w| cb[j] <= w.cx && w.cx < cb[j + 1])
            .collect();
        for r in 0..nr.min(rb.len().saturating_sub(1)) {
            let m = col
                .iter()
                .filter(|w| rb[r] <= w.cy && w.cy < rb[r + 1])
                .map(|w| w.x0)
                .fold(f64::INFINITY, f64::min);
            if m.is_finite() {
                xs.push(m);
            }
        }
        if xs.len() > 1 && cw[j] > 0.0 {
            al.push(pstdev(&xs) / cw[j]);
        }
    }
    f.align_cv = if al.is_empty() { 0.0 } else { mean(&al) };
    f.word_share = rw.len() as f64 / ws_all.len().max(1) as f64;
    let first: Vec<f64> = (0..nr)
        .map(|r| cell(r, 0))
        .filter(|c| !c.trim().is_empty())
        .map(words_of)
        .collect();
    f.first_col_words = if first.is_empty() { 0.0 } else { mean(&first) };
    let last: Vec<&str> = rows
        .iter()
        .map(|r| r.last().map_or("", String::as_str))
        .filter(|c| !c.trim().is_empty())
        .collect();
    f.last_col_num = last.iter().filter(|c| is_num(c)).count() as f64 / last.len().max(1) as f64;
    f.header_like = f64::from(u8::from((0..nc).all(|j| {
        let c = cell(0, j);
        !c.trim().is_empty() && !is_num(c)
    })));
    round_all(&mut f);
    f
}

fn round_all(f: &mut Features) {
    for v in [
        &mut f.fill,
        &mut f.mean_words,
        &mut f.num_frac,
        &mut f.numcol_frac,
        &mut f.sparse_rows,
        &mut f.line_w_mean,
        &mut f.h_len,
        &mut f.v_len,
        &mut f.fill_area,
        &mut f.paths_per_word,
        &mut f.row_h_cv,
        &mut f.col_w_cv,
        &mut f.align_cv,
        &mut f.w_frac,
        &mut f.h_frac,
        &mut f.fs_ratio,
        &mut f.first_col_words,
        &mut f.last_col_num,
        &mut f.word_share,
        &mut f.short_frac,
        &mut f.row_h_fs,
    ] {
        *v = round4(*v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_like_the_prototype_regex() {
        for s in [
            "12",
            " 1,234.5 ",
            "(3.4)",
            "−12%",
            "$5",
            "12a",
            "1 000",
            "7*",
            "[2]",
            "12\u{202f}345",
        ] {
            assert!(is_num(s), "{s:?}");
        }
        for s in ["", "abc", "a1", "12-13", "1.2.3x4", "%5", "12AB"] {
            assert!(!is_num(s), "{s:?}");
        }
    }
}
