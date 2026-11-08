//! Heading detection for untagged PDF body text: paragraphs from lines, a score relative to the
//! neighbouring paragraphs (bolder / larger / CAPS / colour / gap, penalty for many lines) plus the
//! rank of the size above the document mode, and filters against table cells, forms and captions.
//!
//! Idea and thresholds follow OpenDataLoader's heading heuristic (ideas only; no code taken).

use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;
use pdf_oxide::{RegionRole, StructuredPage};

use super::headings_number::is_caption;
use super::headings_style::{case_break, is_caps, spaced_words};

/// Score at or above this makes a heading (0.75 in ODL gave precision 0.42 on ODL-200).
pub(crate) const HEADING_THRESHOLD: f32 = 0.9;
const MIN_CHARS: usize = 3;
const MAX_CHARS: usize = 150;
const MAX_LINES: usize = 2;
/// A line with another line on its baseline closer than this many font sizes is a cell/form field.
const BASELINE_NEIGHBOUR_FS: f32 = 2.5;
/// Paragraphs smaller than this share of the mode are never headings.
const MIN_SIZE_OF_MODE: f32 = 0.9;
/// Mode window and size cap of the rank boost (ODL: [10, 13] pt, sizes up to 32 pt).
const MODE_WINDOW: (f32, f32) = (10.0, 13.0);
const RANK_MAX_SIZE: f32 = 32.0;
pub(crate) const CAPTION_PREFIXES: [&str; 8] = [
    "figure",
    "fig.",
    "table",
    "рис.",
    "рисунок",
    "таблица",
    "схема",
    "диаграмма",
];
/// Line count above which a page is scored without the neighbour filter (service guard: the
/// baseline-neighbour scan is O(lines²) per row band).
const MAX_PAGE_LINES: usize = 5_000;

/// Document font statistics for the rank boost.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FontStats {
    /// Mode of span sizes (0.5 pt buckets, by span count) inside [`MODE_WINDOW`], else overall.
    pub mode: f32,
    /// Distinct sizes above the mode, ascending, up to [`RANK_MAX_SIZE`].
    pub above: Vec<f32>,
}

fn bucket(size: f32) -> i32 {
    (size * 2.0).round() as i32
}

pub(crate) fn font_stats(pages: &[&StructuredPage]) -> FontStats {
    let mut hist: std::collections::BTreeMap<i32, usize> = std::collections::BTreeMap::new();
    for p in pages {
        for r in p.regions.iter().filter(|r| r.kind == RegionRole::BodyBlock) {
            for s in r
                .spans
                .iter()
                .filter(|s| !s.text.trim().is_empty() && s.font_size.is_finite())
            {
                *hist.entry(bucket(s.font_size)).or_default() += 1;
            }
        }
    }
    let (lo, hi) = (bucket(MODE_WINDOW.0), bucket(MODE_WINDOW.1));
    let pick = |it: &mut dyn Iterator<Item = (&i32, &usize)>| {
        it.max_by_key(|(k, n)| (**n, -**k)).map(|(k, _)| *k)
    };
    let mode = pick(&mut hist.iter().filter(|(k, _)| (lo..=hi).contains(*k)))
        .or_else(|| pick(&mut hist.iter()))
        .unwrap_or(0);
    let above = hist
        .keys()
        .filter(|k| **k > mode && **k <= bucket(RANK_MAX_SIZE))
        .map(|k| *k as f32 / 2.0)
        .collect();
    FontStats {
        mode: mode as f32 / 2.0,
        above,
    }
}

/// One text line with the style facts the score needs.
#[derive(Clone, Debug)]
pub(crate) struct StyledLine {
    pub text: String,
    pub size: f32,
    pub bold: bool,
    pub font: String,
    pub color: (u8, u8, u8),
    pub bbox: Rect,
    /// Widest x gap between two joined spans (pt).
    pub gap: f32,
    /// A letter-spaced line as words, when the glyph gaps tell the words apart
    /// ([`super::headings_style::spaced_words`]).
    pub words: Option<String>,
}

/// Spans (reading order) to lines, the same grouping as `layout::group_lines`; a line is bold when
/// at least half of its non-whitespace chars are bold (weight >= 600).
pub(crate) fn styled_lines(spans: &[TextSpan]) -> Vec<StyledLine> {
    let mut out: Vec<(StyledLine, f32, f32, usize, usize)> = Vec::new(); // line, anchor yc, h, bold, all
    let mut members: Vec<Vec<&TextSpan>> = Vec::new();
    let mut right = 0.0f32;
    for s in spans.iter().filter(|s| !s.text.trim().is_empty()) {
        let yc = s.bbox.y + s.bbox.height / 2.0;
        let n = s.text.chars().filter(|c| !c.is_whitespace()).count();
        let b = if s.font_weight.is_bold() { n } else { 0 };
        if let Some((l, ayc, ah, bold, all)) = out.last_mut()
            && (yc - *ayc).abs() < 0.5 * ah.min(s.bbox.height)
        {
            if s.bbox.x - right > 0.15 * s.font_size
                && !l.text.ends_with(' ')
                && !s.text.starts_with(' ')
            {
                l.text.push(' ');
            }
            l.text.push_str(&s.text);
            l.size = l.size.max(s.font_size);
            l.bbox = l.bbox.union(&s.bbox);
            l.gap = l.gap.max(s.bbox.x - right);
            right = s.bbox.x + s.bbox.width;
            *bold += b;
            *all += n;
            if let Some(m) = members.last_mut() {
                m.push(s);
            }
            continue;
        }
        right = s.bbox.x + s.bbox.width;
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        out.push((
            StyledLine {
                text: s.text.clone(),
                size: s.font_size,
                bold: false,
                font: s.font_name.rsplit('+').next().unwrap_or("").to_string(),
                color: (c(s.color.r), c(s.color.g), c(s.color.b)),
                bbox: s.bbox,
                gap: 0.0,
                words: None,
            },
            yc,
            s.bbox.height,
            b,
            n,
        ));
        members.push(vec![s]);
    }
    out.into_iter()
        .zip(members)
        .map(|((mut l, _, _, b, n), m)| {
            l.text = l.text.trim().to_string();
            l.bold = n > 0 && 2 * b >= n;
            l.words = spaced_words(&l.text, &m);
            l
        })
        .collect()
}

/// A paragraph: consecutive lines of one region with the same size (±10 %) and weight, vertically
/// adjacent (gap <= 1.2 line heights) and overlapping in x.
#[derive(Clone, Debug)]
pub(crate) struct Para {
    /// Index range into the region's lines.
    pub lines: std::ops::Range<usize>,
    pub text: String,
    pub size: f32,
    pub bold: bool,
    pub font: String,
    pub color: (u8, u8, u8),
    pub bbox: Rect,
}

pub(crate) fn paragraphs(lines: &[StyledLine]) -> Vec<Para> {
    let mut out: Vec<Para> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if let Some(p) = out.last_mut() {
            let prev = &lines[i - 1];
            let h = prev.bbox.height.max(l.bbox.height);
            let gap = prev.bbox.y - (l.bbox.y + l.bbox.height);
            let ov = (prev.bbox.x + prev.bbox.width).min(l.bbox.x + l.bbox.width)
                - prev.bbox.x.max(l.bbox.x);
            let joins = gap >= -0.5 * l.bbox.height
                && gap <= 1.2 * h
                && (l.size - p.size).abs() <= 0.1 * l.size.max(p.size)
                && l.bold == p.bold
                && !case_break(prev, l)
                && (ov > 0.3 * prev.bbox.width.min(l.bbox.width)
                    || (l.bbox.x - prev.bbox.x).abs() < 1.5 * l.size);
            if joins {
                p.lines.end = i + 1;
                p.text.push(' ');
                p.text.push_str(&l.text);
                p.size = p.size.max(l.size);
                p.bbox = p.bbox.union(&l.bbox);
                continue;
            }
        }
        out.push(Para {
            lines: i..i + 1,
            text: l.text.clone(),
            size: l.size,
            bold: l.bold,
            font: l.font.clone(),
            color: l.color,
            bbox: l.bbox,
        });
    }
    out
}

/// Score of `b` against one neighbour (`None` = page edge, neutral 1.0).
fn against(b: &Para, n: Option<&Para>, n_is_next: bool) -> f32 {
    let Some(n) = n else { return 1.0 };
    let same = b.font == n.font;
    // (bolder, lighter, larger, smaller) weights; same font family weighs more
    let (bolder, lighter, larger, smaller) = if same {
        (0.55, 0.15, 0.55, 0.4)
    } else {
        (0.46, 0.1, 0.4, 0.23)
    };
    let mut p = 0.0;
    if b.bold && !n.bold {
        p += bolder;
    } else if n.bold && !b.bold {
        p -= lighter;
    }
    let eps = if same { 0.05 } else { 0.08 };
    if b.size > n.size + eps {
        p += larger;
    } else if n.size > b.size + eps {
        p -= smaller;
    }
    if b.size > 1.5 * n.size + eps {
        p += 0.1;
    }
    if b.color != n.color {
        p += 0.1;
    }
    match (is_caps(&b.text), is_caps(&n.text)) {
        (true, false) => p += 0.25,
        (false, true) => p -= 0.2,
        _ => {}
    }
    // vertical gap to the neighbour (PDF coordinates, y up)
    let gap = if n_is_next {
        b.bbox.y - (n.bbox.y + n.bbox.height)
    } else {
        n.bbox.y - (b.bbox.y + b.bbox.height)
    };
    if gap >= b.bbox.height / 2.0 {
        p += 0.2;
    }
    p
}

fn rank_boost(size: f32, stats: &FontStats) -> f32 {
    if stats.above.is_empty() || size <= stats.mode + 0.01 {
        return 0.0;
    }
    let pos = stats
        .above
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - size).abs().total_cmp(&(b.1 - size).abs()))
        .map_or(0, |(i, _)| i);
    (pos + 1) as f32 / stats.above.len() as f32
}

/// Whether `n` is heading-shaped ([`passes_filters`]) with a stronger style than `b` (larger and
/// not lighter, or bolder and not smaller; the size epsilon of [`against`]). Such a neighbour is a
/// heading above `b`, not context.
fn outranks(n: &Para, b: &Para, stats: &FontStats) -> bool {
    let eps = if b.font == n.font { 0.05 } else { 0.08 };
    let (larger, smaller) = (n.size > b.size + eps, b.size > n.size + eps);
    ((larger && (n.bold || !b.bold)) || (n.bold && !b.bold && !smaller)) && passes_filters(n, stats)
}

/// Outranking paragraphs skipped at most above a paragraph (title, subtitle, section heading).
const MAX_OUTRANKING_SKIP: usize = 3;

/// Heading scores of the page's paragraphs in reading order. `breaks[i]` says a table (or another
/// context boundary) lies between paragraphs `i - 1` and `i`: the neighbour across it counts as a
/// page edge (neutral).
pub(crate) fn scores(paras: &[&Para], breaks: &[bool], stats: &FontStats) -> Vec<f32> {
    let bold = paras.iter().filter(|p| p.bold).count();
    let rare_bold = bold > 0 && 2 * bold < paras.len();
    (0..paras.len())
        .map(|i| {
            let b = paras[i];
            let raw_prev = i.checked_sub(1).map(|j| paras[j]);
            let raw_next = paras.get(i + 1).copied();
            let cut = |k: usize| breaks.get(k).copied().unwrap_or(false);
            // context above `b`: the nearest paragraph, not across a break, past headings that
            // outrank `b` with a gap below them (a title over a section heading is not context;
            // a line tight under a larger one continues it and stays context). With only such
            // headings above, the document body style is the context (own font, colour and bbox:
            // no colour or gap bonus).
            let heading_above = |k: usize, j: usize| {
                paras[k].bbox.y - (paras[j].bbox.y + paras[j].bbox.height) >= b.bbox.height / 2.0
                    && outranks(paras[k], b, stats)
            };
            let mut j = i;
            let mut skipped = 0;
            let cut_prev = loop {
                match j.checked_sub(1).filter(|_| !cut(j)) {
                    // past the skip bound the heading above is kept as context
                    Some(k) if !heading_above(k, j) || skipped == MAX_OUTRANKING_SKIP => {
                        break Some(paras[k]);
                    }
                    Some(k) => {
                        skipped += 1;
                        j = k;
                    }
                    None => break None,
                }
            };
            let body = (skipped > 0 && cut_prev.is_none() && stats.mode > 0.0).then(|| Para {
                lines: 0..1,
                text: String::new(),
                size: stats.mode,
                bold: false,
                font: b.font.clone(),
                color: b.color,
                bbox: b.bbox,
            });
            let cut_prev = cut_prev.or(body.as_ref());
            let cut_next = raw_next.filter(|_| !cut(i + 1));
            // a table is a boundary like a page edge, unless that leaves no neighbour at all
            // (a line between tables would score 1.0 on nothing)
            let (prev, next) = if cut_prev.is_none()
                && cut_next.is_none()
                && (raw_prev.is_some() || raw_next.is_some())
            {
                (raw_prev, raw_next)
            } else {
                (cut_prev, cut_next)
            };
            let n = b.lines.len() as f32;
            // a paragraph alone on its page has no context: only the boosts count
            let base = if prev.is_none() && next.is_none() {
                0.0
            } else {
                against(b, prev, false).min(against(b, next, true))
            };
            let p = base * (1.0 - 0.0291 * (n - 1.0).powi(2)).max(0.0);
            p.clamp(0.0, 1.0)
                + 0.5 * rank_boost(b.size, stats)
                + if b.bold && rare_bold { 0.3 } else { 0.0 }
        })
        .collect()
}

/// Trailing `.`, `,`, `;` or `:`, also before a number glued to a word of two or more letters
/// ("focus.73", "Vol.4 No.6"; not "Appendix A.1", "Part II.1" or "2.1": a letter or a Roman
/// numeral numbers a section).
fn ends_with_punctuation(t: &str) -> bool {
    const P: [char; 4] = ['.', ',', ';', ':'];
    let t = t.trim_end();
    let bare = t.trim_end_matches(|c: char| c.is_ascii_digit());
    let word: Vec<char> = bare
        .chars()
        .rev()
        .skip(1)
        .take_while(|c| c.is_alphabetic())
        .collect();
    t.ends_with(P)
        || (bare.len() < t.len()
            && bare.ends_with(P)
            && word.len() >= 2
            && !word.iter().all(|c| "IVXLCDM".contains(*c)))
}

/// Filters: length, line count, captions, trailing punctuation, size against the mode.
pub(crate) fn passes_filters(p: &Para, stats: &FontStats) -> bool {
    let n = p.text.chars().filter(|c| !c.is_whitespace()).count();
    let lower = p.text.to_lowercase();
    (MIN_CHARS..=MAX_CHARS).contains(&n)
        && p.lines.len() <= MAX_LINES
        && !is_caption(&lower)
        && !ends_with_punctuation(&p.text)
        && p.size.is_finite()
        && (stats.mode <= 0.0 || p.size >= MIN_SIZE_OF_MODE * stats.mode)
}

/// For each line of the page (all body lines, any region), whether another line sits on its
/// baseline closer than [`BASELINE_NEIGHBOUR_FS`] font sizes. O(n log n + n·k), k = lines per row.
pub(crate) fn baseline_neighbours(lines: &[&StyledLine]) -> Vec<bool> {
    let mut out = vec![false; lines.len()];
    if lines.len() > MAX_PAGE_LINES {
        return out;
    }
    let yc = |l: &StyledLine| l.bbox.y + l.bbox.height / 2.0;
    let mut idx: Vec<usize> = (0..lines.len())
        .filter(|&i| yc(lines[i]).is_finite() && lines[i].bbox.x.is_finite())
        .collect();
    idx.sort_by(|&a, &b| yc(lines[a]).total_cmp(&yc(lines[b])));
    for (k, &i) in idx.iter().enumerate() {
        let a = lines[i];
        for &j in idx[k + 1..].iter() {
            let b = lines[j];
            if yc(b) - yc(a) >= 0.5 * a.bbox.height.max(b.bbox.height) {
                break;
            }
            if (yc(b) - yc(a)).abs() >= 0.5 * a.bbox.height.min(b.bbox.height) {
                continue;
            }
            let gap =
                (b.bbox.x - (a.bbox.x + a.bbox.width)).max(a.bbox.x - (b.bbox.x + b.bbox.width));
            if gap < BASELINE_NEIGHBOUR_FS * a.size.max(b.size) {
                out[i] = true;
                out[j] = true;
            }
        }
    }
    out
}

/// Numbering depth for heading levels: "3" / "3." / "IV." / "Глава 2" → 1, "3.1" → 2, "3.1.2" → 3.
pub(crate) fn numbering_depth(text: &str) -> Option<usize> {
    let first = text.split_whitespace().next()?;
    let lower = first.to_lowercase();
    if [
        "глава",
        "раздел",
        "часть",
        "статья",
        "chapter",
        "appendix",
        "приложение",
        "section",
        "part",
        "article",
    ]
    .contains(&lower.as_str())
    {
        return text.split_whitespace().nth(1).map(|_| 1);
    }
    let t = first.trim_end_matches(['.', ')']);
    if !t.is_empty()
        && t.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C'))
        && first.ends_with('.')
    {
        return Some(1);
    }
    let parts: Vec<&str> = t.split('.').collect();
    (parts.len() <= 4
        && parts
            .iter()
            .all(|p| (1..=2).contains(&p.len()) && p.chars().all(|c| c.is_ascii_digit()))
        && text.split_whitespace().nth(1).is_some())
    .then_some(parts.len())
}

/// Paragraph breaks: a baseline step above `K_GAP` document pitches splits; first-line indent,
/// ragged-edge pad and the "short line before a numbered marker" margin are in font sizes.
const K_GAP: f32 = 1.2;
const INDENT_FS: f32 = 0.8;
const SHORT_PAD_FS: f32 = 0.5;
const NUM_SHORT_FS: f32 = 3.0;
/// Document line pitch when no pair of lines gives one.
const DEFAULT_PITCH: f32 = 1.2;

/// Document mode of "baseline step / font size" over adjacent body lines of one size (±10 %),
/// 0.05 buckets in [0.8, 3.0]; ties go to the smaller ratio.
pub(crate) fn line_pitch_ratio(pages: &[&StructuredPage]) -> f32 {
    let mut hist: std::collections::BTreeMap<i32, usize> = std::collections::BTreeMap::new();
    for p in pages {
        for r in p.regions.iter().filter(|r| r.kind == RegionRole::BodyBlock) {
            for w in styled_lines(&r.spans).windows(2) {
                let (a, b) = (&w[0], &w[1]);
                if (a.size - b.size).abs() > 0.1 * a.size.max(b.size) {
                    continue;
                }
                let q = (a.bbox.y - b.bbox.y) / a.size;
                if (0.8..=3.0).contains(&q) {
                    *hist.entry((q * 20.0).round() as i32).or_default() += 1;
                }
            }
        }
    }
    hist.into_iter()
        .max_by_key(|(k, n)| (*n, -*k))
        .map_or(DEFAULT_PITCH, |(k, _)| k as f32 / 20.0)
}

fn ends_sentence(t: &str) -> bool {
    t.trim_end()
        .ends_with(['.', '!', '?', ':', ';', '»', '"', '”', ')', ']'])
}

/// List marker at the start of a line: `Some(false)` for a bullet (`•`, `-`, ...), `Some(true)` for
/// a numbered one (`1.`, `1)`, `a)`, `(i)`), each followed by whitespace or the line end.
fn list_marker(t: &str) -> Option<bool> {
    let t = t.trim_start();
    let free = |rest: &str| rest.chars().next().is_none_or(char::is_whitespace);
    let first = t.chars().next()?;
    if "•●▪◦○■□►▸‣∙·–—*-".contains(first) {
        return free(&t[first.len_utf8()..]).then_some(false);
    }
    let body = t.strip_prefix('(').unwrap_or(t);
    let n = body.find(['.', ')'])?;
    let tok = &body[..n];
    let digits = (1..=3).contains(&tok.len()) && tok.chars().all(|c| c.is_ascii_digit());
    let letter = tok.chars().count() == 1 && tok.chars().all(char::is_lowercase);
    let roman = (1..=4).contains(&tok.len()) && tok.chars().all(|c| "ivxIVX".contains(c));
    ((digits || letter || roman) && free(&body[n + 1..])).then_some(true)
}

/// Indices of the region's lines that start a new paragraph (the rule of the SP8 paragraph
/// research, checked per pair `a` → `b`; `ratio` is [`line_pitch_ratio`]):
/// jump up or no x overlap (columns) unless the phrase continues; size or weight change; baseline
/// gap; list marker; first-line indent (not under a list item); short line ending a sentence.
pub(crate) fn split_points(lines: &[StyledLine], ratio: f32) -> Vec<usize> {
    let mut out = Vec::new();
    if lines.len() < 2 {
        return out;
    }
    let x1 = |l: &StyledLine| l.bbox.x + l.bbox.width;
    let mut xs: std::collections::BTreeMap<i64, usize> = std::collections::BTreeMap::new();
    for l in lines.iter().filter(|l| l.bbox.x.is_finite()) {
        *xs.entry(l.bbox.x.round() as i64).or_default() += 1;
    }
    let left = xs
        .into_iter()
        .max_by_key(|(k, n)| (*n, -*k))
        .map_or(0.0, |(k, _)| k as f32);
    let mut rights: Vec<f32> = lines.iter().map(x1).filter(|v| v.is_finite()).collect();
    rights.sort_by(f32::total_cmp);
    let right = rights
        .get((rights.len().saturating_sub(1) as f32 * 0.9) as usize)
        .copied()
        .unwrap_or(0.0);
    let first_word = |l: &StyledLine| {
        let w = l
            .text
            .split_whitespace()
            .next()
            .map_or(0, |w| w.chars().count());
        l.bbox.width * w as f32 / l.text.chars().count().max(1) as f32
    };
    let mut start = 0;
    for i in 1..lines.len() {
        let (a, b) = (&lines[i - 1], &lines[i]);
        let s = a.size.max(b.size);
        let dy = a.bbox.y - b.bbox.y;
        let overlap = x1(a).min(x1(b)) - a.bbox.x.max(b.bbox.x);
        let split = if dy < -0.5 * s || overlap < 0.0 {
            ends_sentence(&a.text) || !b.text.chars().next().is_some_and(char::is_lowercase)
        } else {
            (a.size - b.size).abs() > 0.1 * s
                || a.bold != b.bold
                || dy > K_GAP * ratio * s
                || list_marker(&b.text).is_some_and(|numbered| {
                    !numbered || ends_sentence(&a.text) || x1(a) < right - NUM_SHORT_FS * s
                })
                || (b.bbox.x > left + INDENT_FS * s
                    && b.bbox.x < left + 8.0 * s
                    && a.bbox.x <= left + 0.3 * s
                    && list_marker(&lines[start].text).is_none())
                || (x1(a) + first_word(b) + SHORT_PAD_FS * s < right
                    && ends_sentence(&a.text)
                    && b.bbox.x <= left + 8.0 * s)
        };
        if split {
            out.push(i);
            start = i;
        }
    }
    out
}

#[cfg(test)]
#[path = "headings_test.rs"]
mod tests;
