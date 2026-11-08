//! Candidate paragraphs of running headers and footers: per-page edge bands, row merging and
//! outer folios (a child of `running`; the constants and the layer matching stay there).

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edge {
    Top,
    Bottom,
}

/// A paragraph (or, in the edge bands, a whole row of paragraphs) with its `(region, span)` indices.
#[derive(Clone, Debug)]
pub(super) struct Cand {
    pub(super) spans: Vec<(usize, usize)>,
    pub(super) text: String,
    pub(super) size: f32,
    pub(super) bbox: Rect,
    /// Most chunks on one line of the paragraph (spans split by gaps > [`CHUNK_GAP_FS`]).
    pub(super) chunks: usize,
}

pub(super) fn norm(t: &str) -> String {
    t.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(super) fn digit_mask(t: &str) -> (String, Vec<u64>) {
    let mut mask = String::new();
    let mut nums = Vec::new();
    let mut cur: Option<u64> = None;
    for c in t.chars() {
        if let Some(d) = c.to_digit(10) {
            cur = Some(
                cur.unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(u64::from(d)),
            );
        } else {
            if let Some(n) = cur.take() {
                nums.push(n);
                mask.push('#');
            }
            mask.push(c);
        }
    }
    if let Some(n) = cur {
        nums.push(n);
        mask.push('#');
    }
    (mask, nums)
}

/// Same size, overlapping boxes, and equal text or numbering that differs by `step` in one number.
pub(super) fn similar(a: &Cand, b: &Cand, step: u64) -> bool {
    let ov = |a0: f32, a1: f32, b0: f32, b1: f32| a1.min(b1) - a0.max(b0) > 0.0;
    if (a.size - b.size).abs() > SIZE_TOL
        || !ov(
            a.bbox.x,
            a.bbox.x + a.bbox.width,
            b.bbox.x,
            b.bbox.x + b.bbox.width,
        )
        || !ov(
            a.bbox.y,
            a.bbox.y + a.bbox.height,
            b.bbox.y,
            b.bbox.y + b.bbox.height,
        )
    {
        return false;
    }
    let (ta, tb) = (norm(&a.text), norm(&b.text));
    if ta == tb {
        return true;
    }
    let ((ma, na), (mb, nb)) = (digit_mask(&ta), digit_mask(&tb));
    if ma != mb || na.is_empty() || na.len() != nb.len() {
        return false;
    }
    let diffs: Vec<u64> = na
        .iter()
        .zip(&nb)
        .filter(|(x, y)| x != y)
        .map(|(x, y)| x.abs_diff(*y))
        .collect();
    diffs == [step]
}

/// Whether the centre of `r` lies inside one of the page's table boxes.
pub(super) fn in_tables(tables: &[DetectedTable], r: &Rect) -> bool {
    let (x, y) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
    let holds = |b: &BBox| b.x <= x && x <= b.x + b.w && b.y <= y && y <= b.y + b.h;
    tables.iter().any(|t| holds(&t.bbox))
}

/// A span of digits only (a folio set apart from the running title, often larger than it).
pub(super) fn is_number(s: &TextSpan) -> bool {
    let t = s.text.trim();
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
}

/// Paragraph candidates of a page inside the edge bands (body and marginal regions), with span
/// indices per paragraph, plus one placeholder per edge for the nearest body text outside the
/// bands (empty text, NaN size: never matches, but orders layers and bounds the gate's gap). Only
/// edge-band spans are cloned: memory O(band spans), not O(page). Spans whose centre lies inside a
/// table of the page are never candidates.
pub(super) fn candidates(page: &StructuredPage, tables: &[DetectedTable], body: f32) -> Vec<Cand> {
    let mut out = Vec::new();
    let h = page.page_height;
    let in_edge = |s: &TextSpan| {
        s.bbox.y >= (1.0 - EDGE_BAND) * h || s.bbox.y + s.bbox.height <= EDGE_BAND * h
    };
    let (mut top_mid, mut bottom_mid): (Option<Rect>, Option<Rect>) = (None, None);
    for (ri, r) in page.regions.iter().enumerate() {
        if !matches!(r.kind, RegionRole::BodyBlock | RegionRole::MarginalLabel) {
            continue;
        }
        for s in r
            .spans
            .iter()
            .filter(|s| !s.text.trim().is_empty() && !in_edge(s))
        {
            if top_mid.is_none_or(|t| s.bbox.y + s.bbox.height > t.y + t.height) {
                top_mid = Some(s.bbox);
            }
            if bottom_mid.is_none_or(|b| s.bbox.y < b.y) {
                bottom_mid = Some(s.bbox);
            }
        }
        // span index of every edge-band line (same grouping as `styled_lines`)
        let live: Vec<usize> = (0..r.spans.len())
            .filter(|&i| {
                let s = &r.spans[i];
                !s.text.trim().is_empty() && in_edge(s) && !in_tables(tables, &s.bbox)
            })
            .collect();
        if live.is_empty() {
            continue;
        }
        let spans: Vec<TextSpan> = live.iter().map(|&i| r.spans[i].clone()).collect();
        let lines = styled_lines(&spans);
        let line_of = line_index_of_spans(&spans);
        let band = |l: &super::super::headings::StyledLine| {
            let h = page.page_height;
            if l.bbox.y >= (1.0 - EDGE_BAND) * h {
                0
            } else if l.bbox.y + l.bbox.height <= EDGE_BAND * h {
                2
            } else {
                1
            }
        };
        for p in paragraphs(&lines) {
            // a paragraph straddling the edge band is split there (column regions glue a running
            // head to the first body line below it)
            let mut a = p.lines.start;
            while a < p.lines.end {
                let mut b = a + 1;
                while b < p.lines.end && band(&lines[b]) == band(&lines[a]) {
                    b += 1;
                }
                let run = &lines[a..b];
                let bbox = run
                    .iter()
                    .skip(1)
                    .fold(run[0].bbox, |r, l| r.union(&l.bbox));
                // spans of lines a..b: `line_of` is non-decreasing
                let (s0, s1) = (
                    line_of.partition_point(|&l| l < a),
                    line_of.partition_point(|&l| l < b),
                );
                let idx: Vec<usize> = live[s0..s1].to_vec();
                // the size of the text, not of a larger folio beside it
                let text_size = idx
                    .iter()
                    .map(|&i| &r.spans[i])
                    .filter(|s| !is_number(s))
                    .map(|s| s.font_size)
                    .fold(0.0f32, f32::max);
                let size = if text_size > 0.0 {
                    text_size
                } else {
                    run.iter().map(|l| l.size).fold(0.0f32, f32::max)
                };
                let text = run
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                if bbox.y.is_finite()
                    && bbox.height.is_finite()
                    && size.is_finite()
                    && (body <= 0.0
                        || size <= MAX_SIZE_OF_BODY * body
                        || is_folio(&digit_mask(&norm(&text)).0))
                {
                    out.push(Cand {
                        chunks: max_chunks(&r.spans, &idx, &line_of[s0..s1], &live[s0..s1]),
                        spans: idx.iter().map(|&i| (ri, i)).collect(),
                        text,
                        size,
                        bbox,
                    });
                }
                a = b;
            }
        }
    }
    let mut out = merge_edge_rows(out, page.page_height);
    for bbox in [top_mid, bottom_mid].into_iter().flatten() {
        out.push(Cand {
            spans: Vec::new(),
            text: String::new(),
            size: f32::NAN,
            bbox,
            chunks: 1,
        });
    }
    out
}

/// Edge-band candidates sharing a row (vertical centres within half the smaller height) become one
/// candidate, cells in x order: column splits and cell regions cut a running head into pieces.
pub(super) fn merge_edge_rows(cands: Vec<Cand>, h: f32) -> Vec<Cand> {
    let yc = |c: &Cand| c.bbox.y + c.bbox.height / 2.0;
    let (mut edge, mut out): (Vec<Cand>, Vec<Cand>) = cands
        .into_iter()
        .partition(|c| in_band(c, Edge::Top, h) || in_band(c, Edge::Bottom, h));
    edge.sort_by(|a, b| yc(b).total_cmp(&yc(a)).then(a.bbox.x.total_cmp(&b.bbox.x)));
    let mut rows: Vec<Vec<Cand>> = Vec::new();
    for c in edge {
        match rows.last_mut() {
            Some(row)
                if (yc(&row[0]) - yc(&c)).abs() < 0.5 * row[0].bbox.height.min(c.bbox.height) =>
            {
                row.push(c)
            }
            _ => rows.push(vec![c]),
        }
    }
    for mut row in rows {
        row.sort_by(|a, b| a.bbox.x.total_cmp(&b.bbox.x));
        let mut it = row.into_iter();
        let Some(mut m) = it.next() else { continue };
        for c in it {
            m.text.push(' ');
            m.text.push_str(&c.text);
            m.size = m.size.max(c.size);
            m.bbox = m.bbox.union(&c.bbox);
            m.chunks += c.chunks;
            m.spans.extend(c.spans);
        }
        out.push(m);
    }
    out
}

/// A page number at the left or right end of the outermost row of an edge, beside a running title
/// ("Title  9", "18  Title", "Zhang et al.  Page 5 of 9"): no page text lies beyond the row, the
/// row is one line apart from the rest of the page by at least [`MIN_BODY_GAP_LINES`], its end
/// chunk (split by gaps > [`FOLIO_GAP_FS`]) is a folio ([`is_folio`]) not set smaller than the rest
/// of the row (a superscript footnote mark is), and the rest has letters (not an axis of ticks).
/// A bare number beside a row ending a sentence ("22  Likelihood of the model.") is a footnote.
/// Its `(region, span)` indices, or `None`. Used on single-page documents only: multipage
/// documents rely on the cross-page layers (footnotes numbered on from page to page look like
/// page numbers). Known residual false splits on isolated edge rows: "Chapter 3", a year, a total,
/// the last TOC entry; their number becomes a note, the text is kept.
pub(super) fn outer_folio(
    page: &StructuredPage,
    c: &Cand,
    edge: Edge,
) -> Option<Vec<(usize, usize)>> {
    let (top, bottom) = (c.bbox.y + c.bbox.height, c.bbox.y);
    let mut gap = f32::INFINITY;
    for (ri, r) in page.regions.iter().enumerate() {
        for (si, s) in r.spans.iter().enumerate() {
            if s.text.trim().is_empty() || c.spans.contains(&(ri, si)) {
                continue;
            }
            let (st, sb) = (s.bbox.y + s.bbox.height, s.bbox.y);
            let (beyond, inside) = match edge {
                Edge::Top => (st > top + SIZE_TOL, bottom - st),
                Edge::Bottom => (sb < bottom - SIZE_TOL, sb - top),
            };
            if beyond {
                return None;
            }
            if inside >= 0.0 {
                gap = gap.min(inside);
            }
        }
    }
    let apart = gap >= MIN_BODY_GAP_LINES * c.bbox.height;
    if !apart {
        return None;
    }
    let mut v: Vec<(&TextSpan, (usize, usize))> = c
        .spans
        .iter()
        .filter_map(|&(r, s)| Some((page.regions.get(r)?.spans.get(s)?, (r, s))))
        .filter(|(s, _)| !s.text.trim().is_empty())
        .collect();
    let yc = |s: &TextSpan| s.bbox.y + s.bbox.height / 2.0;
    let (lo, hi, hmin) = v.iter().fold(
        (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY),
        |(lo, hi, hm), (s, _)| (lo.min(yc(s)), hi.max(yc(s)), hm.min(s.bbox.height)),
    );
    // one line only (NaN geometry fails this test)
    let one_line = hi - lo < 0.5 * hmin;
    if v.len() < 2 || !one_line {
        return None;
    }
    v.sort_by(|a, b| a.0.bbox.x.total_cmp(&b.0.bbox.x));
    let mut chunks: Vec<std::ops::Range<usize>> = Vec::new();
    let mut right = f32::NEG_INFINITY;
    for (i, (s, _)) in v.iter().enumerate() {
        match chunks.last_mut() {
            Some(c) if s.bbox.x - right <= FOLIO_GAP_FS * s.font_size.max(1.0) => c.end = i + 1,
            _ => chunks.push(i..i + 1),
        }
        right = right.max(s.bbox.x + s.bbox.width);
    }
    let last = chunks.len().checked_sub(1).filter(|&l| l > 0)?;
    for (k, rest) in [(0, chunks[1].start..v.len()), (last, 0..chunks[last].start)] {
        let (f, others) = (&v[chunks[k].clone()], &v[rest]);
        let text = f
            .iter()
            .map(|(s, _)| s.text.trim())
            .collect::<Vec<_>>()
            .join(" ");
        let small = f
            .iter()
            .map(|(s, _)| s.font_size)
            .fold(f32::INFINITY, f32::min);
        let large = others
            .iter()
            .map(|(s, _)| s.font_size)
            .fold(f32::NEG_INFINITY, f32::max);
        let mask = digit_mask(&norm(&text)).0;
        let rest = others
            .iter()
            .map(|(s, _)| s.text.trim())
            .collect::<Vec<_>>()
            .join(" ");
        let footnote = mask == "#" && rest.trim_end().ends_with(['.', '!', '?']);
        if is_folio(&mask)
            && !footnote
            && small >= large - SIZE_TOL
            && rest.chars().any(char::is_alphabetic)
        {
            return Some(f.iter().map(|&(_, i)| i).collect());
        }
    }
    None
}

/// Most chunks on one line among `idx` spans (x-sorted per line, split by wide gaps).
pub(super) fn max_chunks(
    spans: &[TextSpan],
    idx: &[usize],
    line_of: &[usize],
    live: &[usize],
) -> usize {
    let mut by_line: std::collections::BTreeMap<usize, Vec<&TextSpan>> = Default::default();
    for (&si, &l) in live.iter().zip(line_of) {
        if idx.binary_search(&si).is_ok() {
            by_line.entry(l).or_default().push(&spans[si]);
        }
    }
    by_line
        .into_values()
        .map(|mut v| {
            v.sort_by(|a, b| a.bbox.x.total_cmp(&b.bbox.x));
            let mut right = f32::NEG_INFINITY;
            let mut n = 0;
            for s in v {
                if n == 0 || s.bbox.x - right > CHUNK_GAP_FS * s.font_size.max(1.0) {
                    n += 1;
                }
                right = right.max(s.bbox.x + s.bbox.width);
            }
            n
        })
        .max()
        .unwrap_or(0)
}

/// Line number of each span, the same grouping rule as `headings::styled_lines`.
pub(super) fn line_index_of_spans(spans: &[TextSpan]) -> Vec<usize> {
    let mut out = Vec::with_capacity(spans.len());
    let (mut line, mut ayc, mut ah) = (0usize, f32::NAN, 0.0f32);
    for (i, s) in spans.iter().enumerate() {
        let yc = s.bbox.y + s.bbox.height / 2.0;
        if i > 0 && (yc - ayc).abs() < 0.5 * ah.min(s.bbox.height) {
            out.push(line);
            continue;
        }
        if i > 0 {
            line += 1;
        }
        (ayc, ah) = (yc, s.bbox.height);
        out.push(line);
    }
    out
}
