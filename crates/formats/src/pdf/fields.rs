//! Form fields drawn as `_` runs (Word templates, policies, questionnaires). A value is printed over
//! its run with the baseline 0.5–2 pt above the label's, so the engine reads it before the label
//! and glues it to the label and to the run. On a field row the spans are put in x order within
//! each chunk, the run under a value is dropped, an empty run becomes `___`, and the row becomes a
//! paragraph of its own. Rows without a field keep the engine text.

use pdf_oxide::layout::TextSpan;
use pdf_oxide::{RegionRole, StructuredPage, StructuredRegion};

use super::layout::{group_lines, non_ws_chars};

/// A run this long or longer in a span with no value over it is an empty field: it becomes `___`.
const EMPTY_RUN: usize = 4;
/// A value lies over a run when their x-overlap is more than this share of the narrower of the two.
const OVER_SHARE: f32 = 0.5;
/// Spans of a row farther apart than this many font sizes are different chunks (another column):
/// x order is restored inside a chunk only (as `columns::CHUNK_GAP_FS`).
const CHUNK_GAP_FS: f32 = 2.0;
/// Rows with more spans are left as they are (service guard: values are matched to runs pairwise).
const MAX_ROW_SPANS: usize = 200;
/// A space goes between two spans of a field row when the gap is wider than this many font sizes
/// (as `headings::styled_lines`).
const SPACE_FS: f32 = 0.15;

fn longest_run(t: &str) -> usize {
    t.split(|c| c != '_').map(str::len).max().unwrap_or(0)
}

fn finite(s: &TextSpan) -> bool {
    [s.bbox.x, s.bbox.y, s.bbox.width, s.bbox.height, s.font_size]
        .iter()
        .all(|v| v.is_finite())
}

/// Whether the box of `v` lies over the box of `u` (the text tests are the caller's).
fn box_over(v: &TextSpan, u: &TextSpan) -> bool {
    let ov = (v.bbox.x + v.bbox.width).min(u.bbox.x + u.bbox.width) - v.bbox.x.max(u.bbox.x);
    ov > OVER_SHARE * v.bbox.width.min(u.bbox.width)
}

/// Rows of a region's spans in region order, grouped as `layout::group_lines`: indices into the
/// spans.
fn rows(spans: &[TextSpan]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let (mut anchor_yc, mut anchor_h) = (0.0f32, 0.0f32);
    for (i, s) in spans.iter().enumerate() {
        if s.text.trim().is_empty() {
            if let Some(r) = out.last_mut() {
                r.push(i);
            }
            continue;
        }
        let yc = s.bbox.y + s.bbox.height / 2.0;
        match out.last_mut() {
            Some(r) if (yc - anchor_yc).abs() < 0.5 * anchor_h.min(s.bbox.height) => r.push(i),
            _ => {
                (anchor_yc, anchor_h) = (yc, s.bbox.height);
                out.push(vec![i]);
            }
        }
    }
    out
}

/// The spans of a field row in reading order with their field text, each with whether a value lay
/// over its run, or `None` when the row holds no field (no run under a value and no run of
/// [`EMPTY_RUN`]). Spans without a finite box keep their place after the span before them.
fn field_row(spans: &[TextSpan], row: &[usize]) -> Option<Vec<(TextSpan, bool)>> {
    let all: Vec<&TextSpan> = row
        .iter()
        .map(|&i| &spans[i])
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    let used: Vec<&TextSpan> = all.iter().copied().filter(|s| finite(s)).collect();
    if used.len() > MAX_ROW_SPANS {
        return None;
    }
    // a value has text other than `_`; a run under it is at least 2 long
    let has_text: Vec<bool> = used
        .iter()
        .map(|v| v.text.chars().any(|c| c != '_' && !c.is_whitespace()))
        .collect();
    let run: Vec<usize> = used.iter().map(|s| longest_run(&s.text)).collect();
    let over = |v: usize, u: usize| has_text[v] && run[u] >= 2 && box_over(used[v], used[u]);
    let n = used.len();
    let covered: Vec<bool> = (0..n)
        .map(|u| (0..n).any(|v| v != u && over(v, u)))
        .collect();
    let value: Vec<bool> = (0..n).map(|v| (0..n).any(|u| over(v, u))).collect();
    if !covered.iter().any(|c| *c) && !run.iter().any(|&r| r >= EMPTY_RUN) {
        return None;
    }
    let rtl = used
        .iter()
        .any(|s| s.text.chars().any(|c| matches!(c, '\u{0590}'..='\u{08FF}')));
    // chunks in the engine order, x order inside a chunk
    let mut order: Vec<usize> = (0..used.len()).collect();
    if !rtl {
        let mut by_x = order.clone();
        by_x.sort_by(|&a, &b| used[a].bbox.x.total_cmp(&used[b].bbox.x));
        let mut chunks: Vec<Vec<usize>> = Vec::new();
        let mut right = f32::NEG_INFINITY;
        for i in by_x {
            let s = used[i];
            match chunks.last_mut() {
                Some(c) if s.bbox.x - right <= CHUNK_GAP_FS * s.font_size.max(1.0) => c.push(i),
                _ => chunks.push(vec![i]),
            }
            right = right.max(s.bbox.x + s.bbox.width);
        }
        // a value sits higher than its label and comes first in the engine order: a chunk is
        // placed by its first span that is not a value over a run
        chunks.sort_by_key(|c| {
            c.iter()
                .copied()
                .filter(|&i| !value[i])
                .min()
                .or_else(|| c.iter().min().copied())
        });
        order = chunks.concat();
    }
    // spans without a finite box after the span before them (`None`: before the first)
    let mut unplaced: Vec<(Option<usize>, &TextSpan)> = Vec::new();
    let mut prev = None;
    for s in &all {
        if finite(s) {
            prev = Some(prev.map_or(0, |p| p + 1));
        } else {
            unplaced.push((prev, s));
        }
    }
    let after = |p: Option<usize>| {
        unplaced
            .iter()
            .filter(move |(q, _)| *q == p)
            .map(|(_, s)| ((*s).clone(), false))
    };
    let mut seq: Vec<(TextSpan, bool)> = after(None).collect();
    for i in order {
        seq.push((used[i].clone(), covered[i]));
        seq.extend(after(Some(i)));
    }
    let out = seq
        .into_iter()
        .filter_map(|(mut s, covered)| {
            s.text = if covered {
                strip_runs(&s.text, 2, "")
            } else {
                strip_runs(&s.text, EMPTY_RUN, "___")
            };
            (!s.text.trim().is_empty()).then_some((s, covered))
        })
        .collect();
    Some(out)
}

/// `t` with each run of at least `min` underscores replaced by `with`.
fn strip_runs(t: &str, min: usize, with: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut run = 0;
    for c in t.chars().chain(std::iter::once('\0')) {
        if c == '_' {
            run += 1;
            continue;
        }
        if run > 0 {
            if run >= min {
                out.push(' ');
                out.push_str(with);
                out.push(' ');
            } else {
                out.push_str(&"_".repeat(run));
            }
            run = 0;
        }
        if c != '\0' {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text of a field row: spans joined with a space where the gap is wide enough or a value follows
/// the span whose run it covered (the run's box still reaches under the value).
fn row_text(spans: &[(TextSpan, bool)]) -> String {
    let mut text = String::new();
    let (mut right, mut after_run) = (f32::NEG_INFINITY, false);
    for (s, covered) in spans {
        let ok = finite(s);
        if !text.is_empty()
            && (after_run || !ok || s.bbox.x - right > SPACE_FS * s.font_size.max(1.0))
        {
            text.push(' ');
        }
        text.push_str(s.text.trim());
        if ok {
            right = right.max(s.bbox.x + s.bbox.width);
        }
        // a span without a finite box is set apart on both sides
        after_run = *covered || !ok;
    }
    text
}

/// A piece of the region `r` (its box when no span has a finite one); `None` without spans.
fn piece(r: &StructuredRegion, spans: Vec<TextSpan>, text: String) -> Option<StructuredRegion> {
    spans.first()?;
    let first = spans.iter().find(|s| finite(s)).map_or(r.bbox, |s| s.bbox);
    // not `r.clone()`: that would copy all the region's spans for each piece
    Some(StructuredRegion {
        kind: r.kind.clone(),
        text,
        bbox: spans
            .iter()
            .filter(|s| finite(s))
            .fold(first, |b, s| b.union(&s.bbox)),
        spans,
        column_index: r.column_index,
        section_id: r.section_id,
    })
}

/// The engine text of the rows `from..to`, as a function of `(from, to)`, when the region text has
/// the rows' characters in row order (as `layout::cut_region`), else the rows rebuilt from their
/// spans. Counts and char positions are computed once per region.
fn engine_text<'a>(
    r: &'a StructuredRegion,
    rows: &'a [Vec<usize>],
) -> impl Fn(usize, usize) -> String + 'a {
    // non-whitespace chars before each row
    let mut prefix = vec![0usize];
    for row in rows {
        let n: usize = row.iter().map(|&i| non_ws_chars(&r.spans[i].text)).sum();
        prefix.push(prefix[prefix.len() - 1] + n);
    }
    let whole = prefix[rows.len()] == non_ws_chars(&r.text);
    let pos: Vec<usize> = if whole {
        r.text
            .char_indices()
            .filter(|(_, c)| !c.is_whitespace())
            .map(|(i, _)| i)
            .collect()
    } else {
        Vec::new()
    };
    move |from, to| {
        if !whole {
            let spans: Vec<TextSpan> = rows[from..to]
                .iter()
                .flatten()
                .map(|&i| r.spans[i].clone())
                .collect();
            return group_lines(&spans)
                .into_iter()
                .map(|l| l.text)
                .collect::<Vec<_>>()
                .join("\n");
        }
        let a = pos.get(prefix[from]).copied().unwrap_or(r.text.len());
        let b = pos.get(prefix[to]).copied().unwrap_or(r.text.len());
        r.text[a..b].trim().to_string()
    }
}

/// The page with its form fields read as values after labels: each body region with a field row
/// is split into its field rows (one region each, text rebuilt) and the runs of other rows
/// between them (engine text). `None` when the page has no field.
pub(crate) fn rewrite_fields(page: &StructuredPage) -> Option<StructuredPage> {
    let mut out: Option<Vec<StructuredRegion>> = None;
    for (ri, r) in page.regions.iter().enumerate() {
        let fields = (r.kind == RegionRole::BodyBlock
            && r.spans.iter().any(|s| s.text.contains("__")))
        .then(|| {
            let rows = rows(&r.spans);
            let f: Vec<Option<Vec<(TextSpan, bool)>>> =
                rows.iter().map(|row| field_row(&r.spans, row)).collect();
            (rows, f)
        })
        .filter(|(_, f)| f.iter().any(Option::is_some));
        let Some((rows, f)) = fields else {
            if let Some(o) = &mut out {
                o.push(r.clone());
            }
            continue;
        };
        let o = out.get_or_insert_with(|| page.regions[..ri].to_vec());
        let engine_text = engine_text(r, &rows);
        let mut start = 0;
        for (k, field) in f.into_iter().enumerate() {
            let Some(row) = field else { continue };
            if start < k {
                let rest = rows[start..k].iter().flatten().map(|&i| r.spans[i].clone());
                o.extend(piece(r, rest.collect(), engine_text(start, k)));
            }
            let text = row_text(&row);
            o.extend(piece(r, row.into_iter().map(|(s, _)| s).collect(), text));
            start = k + 1;
        }
        if start < rows.len() {
            let rest = rows[start..].iter().flatten().map(|&i| r.spans[i].clone());
            o.extend(piece(r, rest.collect(), engine_text(start, rows.len())));
        }
    }
    out.map(|regions| StructuredPage {
        page_index: page.page_index,
        page_width: page.page_width,
        page_height: page.page_height,
        regions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_oxide::geometry::Rect;

    fn span(text: &str, x: f32, y: f32, w: f32) -> TextSpan {
        TextSpan {
            text: text.into(),
            bbox: Rect::new(x, y, w, 9.0),
            font_size: 9.0,
            ..Default::default()
        }
    }

    fn page(spans: Vec<TextSpan>, text: &str) -> StructuredPage {
        let bbox = spans
            .iter()
            .skip(1)
            .fold(spans[0].bbox, |b, s| b.union(&s.bbox));
        StructuredPage {
            page_index: 0,
            page_width: 595.0,
            page_height: 842.0,
            regions: vec![StructuredRegion {
                kind: RegionRole::BodyBlock,
                text: text.into(),
                bbox,
                spans,
                column_index: None,
                section_id: None,
            }],
        }
    }

    fn texts(p: &StructuredPage) -> Vec<&str> {
        p.regions.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn value_over_its_run_follows_the_label() {
        // the value is 1.7 pt higher than its label, so the engine reads it first
        let p = page(
            vec![
                span("Intro line of the form.", 42.0, 700.0, 200.0),
                span("Иван Тестов", 147.0, 655.7, 71.0),
                span("Страхователь:", 42.0, 654.0, 100.0),
                span("________________", 145.0, 654.0, 116.0),
                span("Адрес:", 42.0, 634.0, 40.0),
                span("________________", 85.0, 634.0, 116.0),
                span("Closing text.", 42.0, 610.0, 80.0),
            ],
            "Intro line of the form.\nИван Тестов Страхователь: ________________ Адрес: \
             ________________\nClosing text.",
        );
        let s = rewrite_fields(&p).expect("fields");
        assert_eq!(
            texts(&s),
            [
                "Intro line of the form.",
                "Страхователь: Иван Тестов",
                "Адрес: ___",
                "Closing text."
            ]
        );
    }

    #[test]
    fn page_without_runs_is_untouched() {
        let p = page(
            vec![
                span("a_b snake_case", 42.0, 700.0, 100.0),
                span("1__8 cells", 150.0, 700.0, 60.0),
            ],
            "a_b snake_case 1__8 cells",
        );
        assert!(rewrite_fields(&p).is_none());
    }

    #[test]
    fn short_runs_stay_and_long_runs_shrink() {
        let p = page(
            vec![span(
                "Подпись ____________ Дата «___» ________ 20__ г.",
                42.0,
                700.0,
                400.0,
            )],
            "Подпись ____________ Дата «___» ________ 20__ г.",
        );
        let s = rewrite_fields(&p).expect("fields");
        assert_eq!(texts(&s), ["Подпись ___ Дата «___» ___ 20__ г."]);
    }

    #[test]
    fn columns_of_a_row_keep_their_engine_order() {
        // two fields far apart on one row: x order is restored inside each chunk; the chunks keep
        // the engine order of their labels
        let p = page(
            vec![
                span("Ann", 92.0, 701.5, 20.0),
                span("Dana", 352.0, 701.5, 30.0),
                span("Code:", 42.0, 700.0, 40.0),
                span("__________", 85.0, 700.0, 80.0),
                span("Name:", 300.0, 700.0, 40.0),
                span("__________", 345.0, 700.0, 80.0),
            ],
            "Ann Dana Code: __________ Name: __________",
        );
        let s = rewrite_fields(&p).expect("fields");
        assert_eq!(texts(&s), ["Code: Ann Name: Dana"]);
    }

    #[test]
    fn crowded_rows_are_left_alone() {
        let spans: Vec<TextSpan> = (0..MAX_ROW_SPANS + 1)
            .map(|i| span("a__b", 2.0 * i as f32, 700.0, 1.5))
            .collect();
        assert!(rewrite_fields(&page(spans, "")).is_none());
    }

    #[test]
    fn non_finite_spans_are_kept_in_engine_order() {
        // inside a field row: kept after the span before it (here: first)
        let mut p = page(
            vec![
                span("Name:", 42.0, 700.0, 40.0),
                span("__________", 85.0, 700.0, 80.0),
            ],
            "Name: __________",
        );
        p.regions[0].spans[0].bbox = Rect::new(f32::NAN, 700.0, f32::INFINITY, 9.0);
        assert_eq!(texts(&rewrite_fields(&p).expect("fields")), ["Name: ___"]);
        // a row of its own between two field rows
        let mut p = page(
            vec![
                span("A:", 42.0, 700.0, 20.0),
                span("________", 65.0, 700.0, 60.0),
                span("Lost line", 42.0, 690.0, 50.0),
                span("B:", 42.0, 680.0, 20.0),
                span("________", 65.0, 680.0, 60.0),
            ],
            "A: ________ Lost line B: ________",
        );
        p.regions[0].spans[2].bbox = Rect::new(42.0, f32::NAN, 50.0, 9.0);
        assert_eq!(
            texts(&rewrite_fields(&p).expect("fields")),
            ["A: ___", "Lost line", "B: ___"]
        );
    }

    #[test]
    fn long_runs_in_crowded_rows_are_cheap() {
        // 50 rows of 200 spans (the row limit), each a value letter and 1600 underscores
        let text = format!("x{}", "_".repeat(1600));
        let spans: Vec<TextSpan> = (0..50)
            .flat_map(|r| {
                let t = &text;
                (0..MAX_ROW_SPANS)
                    .map(move |i| span(t, 2.0 * i as f32, 700.0 - 14.0 * r as f32, 1.5))
            })
            .collect();
        let t = std::time::Instant::now();
        let s = rewrite_fields(&page(spans, ""));
        eprintln!(
            "rewrite_fields, 50 x 200 spans of 1600 '_': {:?}",
            t.elapsed()
        );
        assert!(s.is_some());
    }
}
