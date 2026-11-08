//! Fill stacks (zebra rows, per-cell fills, A2 `detect_fill_groups`).

use super::bounds::{
    inside, region_lines, region_of, text_pitch, too_wordy, valley_bounds, within,
};
use super::model::drop_close_rows;
use super::rules::{
    BAND_COVER, BAND_MIN_W, FILL_COL_SHARE, FILL_COL_TOL, FILL_EDGE_TOL, FILL_MAX_H_FRAC,
    FILL_MIN_H, FILL_MIN_W, MAX_STACK_FILLS, STACK_ALIGN, STACK_GAP_H, cells_and_head,
    extend_candidate, header_top, split_at_captions,
};
use super::{CandidateKind, Raw};
use crate::tables::grid::Line;
use crate::tables::lattice::covered_len;
use crate::tables::prims::{PRect, Prims, Region, Word};
use crate::tables::stats::median;
use crate::tables::{LimitExceeded, TableConfig};

/// A band of filled boxes with common top and bottom.
struct Band<'p> {
    top: f64,
    bot: f64,
    x0: f64,
    x1: f64,
    rs: Vec<&'p PRect>,
}

/// One fill-stack candidate over rows `y0..y1` (`rg_extend` widens them) and its lines: rows from the band
/// edges, columns from fill x edges shared by the bands, else from valleys.
fn build_fill_stack<'a>(
    ws: &'a [Word],
    st: &[Band],
    r: &Region,
    colb: Option<Vec<f64>>,
    cfg: &TableConfig,
) -> Result<Option<(Raw<'a>, Vec<Line<'a>>)>, LimitExceeded> {
    let rw: Vec<&Word> = ws.iter().filter(|w| inside(w, r, 2.0)).collect();
    if rw.len() < 4 {
        return Ok(None);
    }
    let mut ys: Vec<f64> = std::iter::once(r.y0)
        .chain(st.iter().map(|b| b.top))
        .chain(st.iter().map(|b| b.bot))
        .chain(std::iter::once(r.y1 + 1.0))
        .collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut rowb = drop_close_rows(ys);
    let lines = region_lines(&rw, cfg);
    let colb = match colb {
        Some(c) => c,
        None => {
            let mut xs: Vec<f64> = st
                .iter()
                .flat_map(|b| b.rs.iter().map(|q| q.x0))
                .chain(st.iter().flat_map(|b| b.rs.iter().map(|q| q.x1)))
                .collect();
            xs.sort_by(f64::total_cmp);
            let mut cl: Vec<(f64, usize)> = Vec::new();
            for x in xs {
                match cl.last_mut() {
                    Some(c) if x - c.0 <= FILL_COL_TOL => c.1 += 1,
                    _ => cl.push((x, 1)),
                }
            }
            let need = 2f64.max(FILL_COL_SHARE * st.len() as f64);
            let cb: Vec<f64> = cl
                .into_iter()
                .filter(|&(x, n)| n as f64 >= need && r.x0 + 5.0 < x && x < r.x1 - 5.0)
                .map(|c| c.0)
                .collect();
            if cb.is_empty() {
                let (d, a, f) = (cfg.dense_valleys, cfg.gmin_adapt, cfg.fs_tolerances);
                valley_bounds(&rw, &lines, r, d, a, f)
            } else {
                std::iter::once(r.x0)
                    .chain(cb)
                    .chain(std::iter::once(r.x1))
                    .collect()
            }
        }
    };
    if colb.len() < 3 {
        return Ok(None);
    }
    let top = header_top(
        cfg,
        r.y0 < st[0].top - 2.0,
        &rw,
        &colb,
        &mut rowb,
        st[0].top,
    );
    let Some((rows, head)) = cells_and_head(&rw, &colb, &rowb, top)? else {
        return Ok(None);
    };
    if too_wordy(&rows) {
        return Ok(None);
    }
    Ok(Some((
        Raw {
            kind: CandidateKind::Text,
            bbox: [r.x0, r.y0, r.x1, r.y1],
            rows,
            words: rw,
            colb,
            rowb,
            sub: None,
            head,
            text_rows: false,
        },
        lines,
    )))
}

/// Stacks of filled bands (zebra rows, per-cell fills) as `T` candidates (A2, proto2.py
/// `detect_fill_groups`); a region overlapping a stack without lying inside it vetoes the stack.
pub(crate) fn detect_fill_groups<'a>(
    ws: &'a [Word],
    p: &Prims,
    regs: &[Region],
    page_w: f64,
    page_h: f64,
    cfg: &TableConfig,
) -> Result<Vec<Raw<'a>>, LimitExceeded> {
    if p.rects.iter().filter(|r| r.fill).count() > MAX_STACK_FILLS {
        return Ok(Vec::new());
    }
    let mut fr: Vec<&PRect> = p
        .rects
        .iter()
        .filter(|r| {
            let (w, h) = (r.x1 - r.x0, r.b - r.t);
            r.fill
                && h >= FILL_MIN_H
                && w >= FILL_MIN_W
                && h < FILL_MAX_H_FRAC * page_h
                && !(w > 0.8 * page_w && h > 0.5 * page_h)
        })
        .collect();
    if fr.len() < 2 {
        return Ok(Vec::new());
    }
    fr.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.x0.total_cmp(&b.x0)));
    let mut groups: Vec<(f64, f64, Vec<&PRect>)> = Vec::new();
    for r in fr {
        match groups
            .iter_mut()
            .find(|g| (g.0 - r.t).abs() <= FILL_EDGE_TOL && (g.1 - r.b).abs() <= FILL_EDGE_TOL)
        {
            Some(g) => g.2.push(r),
            None => groups.push((r.t, r.b, vec![r])),
        }
    }
    let mut bands: Vec<Band> = Vec::new();
    for (top, bot, rs) in groups {
        let cov = covered_len(rs.iter().map(|q| (q.x0, q.x1)).collect(), FILL_EDGE_TOL);
        let x0 = rs.iter().map(|q| q.x0).fold(f64::MAX, f64::min);
        let x1 = rs.iter().map(|q| q.x1).fold(f64::MIN, f64::max);
        if x1 - x0 >= BAND_MIN_W && cov >= BAND_COVER * (x1 - x0) {
            bands.push(Band {
                top,
                bot,
                x0,
                x1,
                rs,
            });
        }
    }
    bands.sort_by(|a, b| a.top.total_cmp(&b.top));
    let mut stacks: Vec<Vec<Band>> = Vec::new();
    for b in bands {
        let fits = |s: &Vec<Band>| {
            let l = &s[s.len() - 1];
            (l.x0 - b.x0).abs() <= STACK_ALIGN
                && (l.x1 - b.x1).abs() <= STACK_ALIGN
                && b.top >= l.bot - FILL_EDGE_TOL
                && b.top - l.bot <= STACK_GAP_H * (b.bot - b.top).max(l.bot - l.top)
        };
        match stacks.iter().position(fits) {
            Some(i) => stacks[i].push(b),
            None => stacks.push(vec![b]),
        }
    }
    if cfg.fill_split {
        stacks = split_at_captions(ws, stacks, |b| (b.x0, b.x1), |b| (b.top, b.bot), false);
    }
    let mut out = Vec::new();
    for st in stacks.iter().filter(|s| s.len() >= 2) {
        let r = region_of([
            st.iter().map(|b| b.x0).fold(f64::MAX, f64::min),
            st[0].top,
            st.iter().map(|b| b.x1).fold(f64::MIN, f64::max),
            st[st.len() - 1].bot,
        ]);
        if regs.iter().any(|q| {
            q.x0 < r.x1
                && r.x0 < q.x1
                && q.y0 < r.y1
                && r.y0 < q.y1
                && !within([r.x0, r.y0, r.x1, r.y1], [q.x0, q.y0, q.x1, q.y1], 3.0)
        }) {
            continue;
        }
        let Some((mut c0, lines)) = build_fill_stack(ws, st, &r, None, cfg)? else {
            continue;
        };
        if cfg.rg_extend {
            let hb = median(&st.iter().map(|b| b.bot - b.top).collect::<Vec<_>>());
            let pitch = text_pitch(&lines, hb);
            c0 = extend_candidate(ws, &r, pitch, cfg.header_lines, c0, |e, colb| {
                Ok(build_fill_stack(ws, st, e, Some(colb), cfg)?.map(|c| c.0))
            })?;
        }
        out.push(c0);
    }
    Ok(out)
}
