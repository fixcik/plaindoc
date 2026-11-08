//! Words and ruling primitives (`mk_words`, `mk_prims`, `regions` in proto.py).

use super::stats::Dsu;
use super::{InputWord, PageInput};

/// Thinner than this across, longer than [`LINE_MIN_LEN`] along: a ruling (proto.py `mk_prims`).
const LINE_MAX_THICK: f64 = 2.5;
const LINE_MIN_LEN: f64 = 5.0;
/// tan 1°: a raster segment is a ruling within this slope, not only within [`LINE_MAX_THICK`] (Task 12(c)
/// brief: a full-width line with a residual skew of 0.3° drifts 2.8 pt over 540 pt).
const RASTER_SKEW_TAN: f64 = 0.017_455_064_928_217_585; // = 1f64.to_radians().tan()
/// Raster vertical pieces closer than this in x and this in y form one stack (Task 12(c) brief: rounded
/// cell corners leave 4-8 pt gaps).
const STACK_COORD_TOL: f64 = 2.0;
const STACK_GAP_TOL: f64 = 8.0;
/// Paths at least this big are page frames/backgrounds, not table parts (proto.py `regions`, absolute pt).
const REGION_RECT_MAX_W: f64 = 560.0;
const REGION_RECT_MAX_H: f64 = 700.0;
/// Primitives closer than this are one region (proto.py `regions(tol=3.0)`).
const REGION_TOL: f64 = 3.0;
const REGION_MIN_PRIMS: usize = 3;
const REGION_MIN_W: f64 = 60.0;
const REGION_MIN_H: f64 = 20.0;

/// Detector-internal word (top-left origin).
#[derive(Clone, Debug)]
pub(crate) struct Word {
    pub x0: f64,
    pub x1: f64,
    pub top: f64,
    pub bot: f64,
    pub t: String,
    pub fs: f64,
    pub cx: f64,
    pub cy: f64,
    pub rotated: bool,
}

impl Word {
    pub(crate) fn from_input(w: &InputWord) -> Self {
        let r = w.rect;
        Self {
            x0: r.x,
            x1: r.x + r.w,
            top: r.top,
            bot: r.top + r.h,
            t: w.text.clone(),
            fs: if w.font_size != 0.0 { w.font_size } else { r.h },
            cx: r.x + r.w / 2.0,
            cy: r.top + r.h / 2.0,
            rotated: w.rotated,
        }
    }
}

pub(crate) fn words_of(page: &PageInput) -> Vec<Word> {
    page.words.iter().map(Word::from_input).collect()
}

/// Horizontal ruling `(x_from, x_to, y)`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HLine {
    pub a: f64,
    pub b: f64,
    pub y: f64,
    /// From a raster segment ([`super::InputSegment::raster`]) under the raster rules
    /// ([`super::TableConfig::short_vertical_stacks`]); false otherwise.
    pub raster: bool,
}

/// Vertical ruling `(y_from, y_to, x)`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VLine {
    pub a: f64,
    pub b: f64,
    pub x: f64,
    /// From a raster segment ([`super::InputSegment::raster`]) under the raster rules
    /// ([`super::TableConfig::short_vertical_stacks`]); false otherwise.
    pub raster: bool,
}

/// Non-line path box `(x0, top, x1, bot)`; `fill` feeds the fill stacks (A2, proto2.py `detect_fill_groups`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PRect {
    pub x0: f64,
    pub t: f64,
    pub x1: f64,
    pub b: f64,
    pub fill: bool,
    /// Drawn with a stroke (its edges are also rulings).
    pub stroke: bool,
    /// Background fill ([`mark_backgrounds`], [`super::TableConfig::lined_skip_background`]); false otherwise.
    pub background: bool,
}

pub(crate) struct Prims {
    pub hs: Vec<HLine>,
    pub vs: Vec<VLine>,
    pub rects: Vec<PRect>,
}

#[cfg(test)]
pub(crate) fn prims(page: &PageInput) -> Prims {
    prims_with(page, false)
}

/// Primitives of a page; `raster_rules` ([`super::TableConfig::short_vertical_stacks`]) classifies raster
/// segments by angle and marks their rulings raster (no ruling is raster without it).
pub(crate) fn prims_with(page: &PageInput, raster_rules: bool) -> Prims {
    let (mut hs, mut vs, mut rects) = (Vec::new(), Vec::new(), Vec::new());
    let h = |a: f64, b: f64, y: f64, raster: bool| HLine { a, b, y, raster };
    let v = |a: f64, b: f64, x: f64, raster: bool| VLine { a, b, x, raster };
    for p in &page.paths {
        let (x, top, w, ht) = (p.rect.x, p.rect.top, p.rect.w, p.rect.h);
        if ht <= LINE_MAX_THICK && w > LINE_MIN_LEN {
            hs.push(h(x, x + w, top + ht / 2.0, false));
        } else if w <= LINE_MAX_THICK && ht > LINE_MIN_LEN {
            vs.push(v(top, top + ht, x + w / 2.0, false));
        } else {
            rects.push(PRect {
                x0: x,
                t: top,
                x1: x + w,
                b: top + ht,
                fill: p.fill,
                stroke: p.stroke,
                background: false,
            });
            if p.stroke {
                hs.push(h(x, x + w, top, false));
                hs.push(h(x, x + w, top + ht, false));
                vs.push(v(top, top + ht, x, false));
                vs.push(v(top, top + ht, x + w, false));
            }
        }
    }
    for s in &page.segments {
        let (dx, dy) = ((s.x1 - s.x2).abs(), (s.y1 - s.y2).abs());
        // Raster segments under the raster rules: within 1° of an axis (Task 12(c)).
        let angle = raster_rules && s.raster;
        let (hmax, vmax) = if angle {
            (
                LINE_MAX_THICK.max(RASTER_SKEW_TAN * dx),
                LINE_MAX_THICK.max(RASTER_SKEW_TAN * dy),
            )
        } else {
            (LINE_MAX_THICK, LINE_MAX_THICK)
        };
        if dy <= hmax && dx > LINE_MIN_LEN {
            hs.push(h(
                s.x1.min(s.x2),
                s.x1.max(s.x2),
                (s.y1 + s.y2) / 2.0,
                angle,
            ));
        } else if dx <= vmax && dy > LINE_MIN_LEN {
            vs.push(v(
                s.y1.min(s.y2),
                s.y1.max(s.y2),
                (s.x1 + s.x2) / 2.0,
                angle,
            ));
        }
    }
    Prims { hs, vs, rects }
}

/// A background fill holds at least this many thin rulings.
const BG_MIN_RULINGS: usize = 3;
/// A background fill is at least this many times larger than the box of the rulings inside it (a fill that
/// coincides with the table frame is not a background).
const BG_MIN_AREA_RATIO: f64 = 2.0;
/// Slack in pt for a ruling to count as inside a fill.
const BG_TOL: f64 = 1.0;
/// Pages with more fills x rulings than this are not marked (the per-fill test is quadratic).
const BG_MAX_WORK: usize = 4_000_000;

/// Marks the background fills ([`super::TableConfig::lined_skip_background`]): a fill without stroke that holds
/// at least [`BG_MIN_RULINGS`] thin rulings and whose area is at least [`BG_MIN_AREA_RATIO`] times the area of
/// the box of those rulings, and the fills without stroke of a region passing the same test that are not
/// fully inside the box of its rulings. The colour is not known, so a background is told by geometry only.
/// Nothing is marked on pages over [`BG_MAX_WORK`].
pub(crate) fn mark_backgrounds(p: &mut Prims) {
    if p.rects.len() * (p.hs.len() + p.vs.len()) > BG_MAX_WORK {
        return;
    }
    for r in &mut p.rects {
        if !r.fill || r.stroke {
            continue;
        }
        let (mut n, mut bx) = (0usize, [f64::MAX, f64::MAX, f64::MIN, f64::MIN]);
        let mut add = |x0: f64, y0: f64, x1: f64, y1: f64| {
            n += 1;
            bx = [bx[0].min(x0), bx[1].min(y0), bx[2].max(x1), bx[3].max(y1)];
        };
        let inside = |x0: f64, y0: f64, x1: f64, y1: f64| {
            x0 >= r.x0 - BG_TOL && x1 <= r.x1 + BG_TOL && y0 >= r.t - BG_TOL && y1 <= r.b + BG_TOL
        };
        for h in &p.hs {
            if inside(h.a, h.y, h.b, h.y) {
                add(h.a, h.y, h.b, h.y);
            }
        }
        for v in &p.vs {
            if inside(v.x, v.a, v.x, v.b) {
                add(v.x, v.a, v.x, v.b);
            }
        }
        r.background = n >= BG_MIN_RULINGS
            && (r.x1 - r.x0) * (r.b - r.t) >= BG_MIN_AREA_RATIO * (bx[2] - bx[0]) * (bx[3] - bx[1]);
    }
    // The same test on a region: cell backgrounds of a layout table (each holding no ruling) chain into the
    // region of a ruled table. When the region is at least twice the box of its rulings, its fills without
    // stroke outside that box are backgrounds too.
    let (nh, nv) = (p.hs.len(), p.vs.len());
    let mut marks = Vec::new();
    for reg in regions_with(p, true) {
        let mut bx = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut n = 0usize;
        for &m in &reg.members {
            let b = if m < nh {
                [p.hs[m].a, p.hs[m].y, p.hs[m].b, p.hs[m].y]
            } else if m < nh + nv {
                let v = &p.vs[m - nh];
                [v.x, v.a, v.x, v.b]
            } else {
                continue;
            };
            n += 1;
            bx = [
                bx[0].min(b[0]),
                bx[1].min(b[1]),
                bx[2].max(b[2]),
                bx[3].max(b[3]),
            ];
        }
        if n < BG_MIN_RULINGS
            || (reg.x1 - reg.x0) * (reg.y1 - reg.y0)
                < BG_MIN_AREA_RATIO * (bx[2] - bx[0]) * (bx[3] - bx[1])
        {
            continue;
        }
        for &m in reg.members.iter().filter(|&&m| m >= nh + nv) {
            let r = &p.rects[m - nh - nv];
            let inside = r.x0 >= bx[0] - REGION_TOL
                && r.x1 <= bx[2] + REGION_TOL
                && r.t >= bx[1] - REGION_TOL
                && r.b <= bx[3] + REGION_TOL;
            if r.fill && !r.stroke && !inside {
                marks.push(m - nh - nv);
            }
        }
    }
    for k in marks {
        p.rects[k].background = true;
    }
}

/// Collinear rulings share a coordinate within this many pt (proto2.py `merge_collinear` ctol).
const MERGE_COORD_TOL: f64 = 1.0;
/// Rulings of one line are chained when the gap between their ends is at most this many pt (gtol).
const MERGE_GAP_TOL: f64 = 1.5;

/// Merges `(start, end, coord, raster)` pieces lying on one line (A1, proto2.py `merge_collinear`): stable
/// sort by `(coord, start)`, one pass; the merged coord is the running mean of the pieces, the end the max.
/// A merged ruling is raster when any of its pieces is (a page mixes them only in tests).
fn merge_pieces(
    mut ls: Vec<(f64, f64, f64, bool)>,
    ctol: f64,
    gtol: f64,
) -> Vec<(f64, f64, f64, bool)> {
    ls.sort_by(|p, q| p.2.total_cmp(&q.2).then(p.0.total_cmp(&q.0)));
    let mut out: Vec<(f64, f64, f64, bool, f64)> = Vec::with_capacity(ls.len());
    for (a, b, c, r) in ls {
        if let Some(last) = out.last_mut()
            && (last.2 - c).abs() <= ctol
            && a <= last.1 + gtol
        {
            let n = last.4 + 1.0;
            *last = (
                last.0,
                last.1.max(b),
                (last.2 * (n - 1.0) + c) / n,
                last.3 || r,
                n,
            );
        } else {
            out.push((a, b, c, r, 1.0));
        }
    }
    out.into_iter()
        .map(|(a, b, c, r, _)| (a, b, c, r))
        .collect()
}

/// Merges collinear horizontal and vertical rulings; boxes are untouched (A1, `TableConfig.merge_lines`).
pub(crate) fn merge_collinear(p: &mut Prims) {
    p.hs = merge_pieces(
        p.hs.iter().map(|l| (l.a, l.b, l.y, l.raster)).collect(),
        MERGE_COORD_TOL,
        MERGE_GAP_TOL,
    )
    .into_iter()
    .map(|(a, b, y, raster)| HLine { a, b, y, raster })
    .collect();
    p.vs = merge_pieces(
        p.vs.iter().map(|l| (l.a, l.b, l.x, l.raster)).collect(),
        MERGE_COORD_TOL,
        MERGE_GAP_TOL,
    )
    .into_iter()
    .map(|(a, b, x, raster)| VLine { a, b, x, raster })
    .collect();
}

/// Stacks of short raster vertical pieces become one ruling spanning their gaps (Task 12(c),
/// `TableConfig.short_vertical_stacks`): pieces within [`STACK_COORD_TOL`] in x and [`STACK_GAP_TOL`] in y
/// are chained as in [`merge_collinear`]. Non-raster rulings are returned first, unchanged.
pub(crate) fn stack_raster_verticals(vs: Vec<VLine>) -> Vec<VLine> {
    let (raster, mut out): (Vec<VLine>, Vec<VLine>) = vs.into_iter().partition(|l| l.raster);
    out.extend(
        merge_pieces(
            raster.iter().map(|l| (l.a, l.b, l.x, true)).collect(),
            STACK_COORD_TOL,
            STACK_GAP_TOL,
        )
        .into_iter()
        .map(|(a, b, x, raster)| VLine { a, b, x, raster }),
    );
    out
}

/// Connected group of primitives: `(x0, y0, x1, y1)`. `members` are the primitives of a lined region's own
/// component (A3): index `i < hs.len()` is `hs[i]`, then `vs[i - nh]`, then `rects[i - nh - nv]`, ascending.
/// Regions made from rules or fills have none.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Region {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub members: Vec<usize>,
}

impl Prims {
    /// The primitives of one component (A3, proto2.py `comp_lines`), in their original order.
    pub(crate) fn subset(&self, members: &[usize]) -> Prims {
        let (nh, nv) = (self.hs.len(), self.vs.len());
        let mut out = Prims {
            hs: Vec::new(),
            vs: Vec::new(),
            rects: Vec::new(),
        };
        for &i in members {
            if i < nh {
                out.hs.push(self.hs[i]);
            } else if i < nh + nv {
                out.vs.push(self.vs[i - nh]);
            } else {
                out.rects.push(self.rects[i - nh - nv]);
            }
        }
        out
    }
}

/// Unions every pair of boxes that touch (within [`REGION_TOL`]). The pairs are found by a sweep along x
/// (original) or, when that enumerates fewer candidate pairs, along y, so a stack of full-width rulings is
/// not quadratic. The union-find result is the same either way (components do not depend on the order of
/// unions). The y sweep is used only for finite, non-inverted boxes. It decides every pair with the original
/// x-sweep predicate, roles (`a` earlier, `b` later) taken from the stable x order, so floating-point
/// rounding at a gap of exactly `REGION_TOL` is the same as in the x sweep. The y break is only a prefilter:
/// the old predicate implies `later.y0 <= earlier.y3 + REGION_TOL` up to one rounding of the sum, and the
/// margin (absolute plus relative to the magnitude) is far above that rounding, so no pair the x sweep
/// unions is skipped; extra candidates are rejected by the exact predicate.
fn link_touching(boxes: &[[f64; 4]], d: &mut Dsu) {
    let n = boxes.len();
    // The x sweep's pair test on y, `a` before `b` in x order (its break on `b.x0` comes before it).
    let touch = |a: &[f64; 4], b: &[f64; 4]| b[1] <= a[3] + REGION_TOL && b[3] >= a[1] - REGION_TOL;
    let sweep_x = |d: &mut Dsu, order: &[usize]| {
        for (ii, &i) in order.iter().enumerate() {
            let a = &boxes[i];
            for &j in &order[ii + 1..] {
                let b = &boxes[j];
                if b[0] > a[2] + REGION_TOL {
                    break;
                }
                if touch(a, b) {
                    d.union(i, j);
                }
            }
        }
    };
    let sweep_y = |d: &mut Dsu, order: &[usize], ox: &[usize]| {
        let mut rank = vec![0usize; n];
        for (r, &i) in ox.iter().enumerate() {
            rank[i] = r;
        }
        for (ii, &i) in order.iter().enumerate() {
            let e = &boxes[i];
            let limit = e[3] + REGION_TOL + 1e-6 + 1e-9 * e[3].abs();
            for &j in &order[ii + 1..] {
                let l = &boxes[j];
                if l[1] > limit {
                    break;
                }
                let (a, b) = if rank[i] < rank[j] { (e, l) } else { (l, e) };
                // Finite boxes: `!(b.x0 > a.x2 + tol)` of the x sweep's break is `<=`.
                if b[0] <= a[2] + REGION_TOL && touch(a, b) {
                    d.union(i, j);
                }
            }
        }
    };
    let sorted = |axis: usize| {
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&i, &j| boxes[i][axis].total_cmp(&boxes[j][axis]));
        order
    };
    let ox = sorted(0);
    let normal = boxes
        .iter()
        .all(|b| b.iter().all(|v| v.is_finite()) && b[0] <= b[2] && b[1] <= b[3]);
    if normal && n > 64 {
        let oy = sorted(1);
        // Candidate pairs a sweep along `axis` looks at (the break position of every box, by binary search).
        let cost = |axis: usize, order: &[usize]| -> usize {
            let starts: Vec<f64> = order.iter().map(|&i| boxes[i][axis]).collect();
            order
                .iter()
                .enumerate()
                .map(|(ii, &i)| {
                    let end = starts.partition_point(|&v| v <= boxes[i][axis + 2] + REGION_TOL);
                    end.saturating_sub(ii + 1)
                })
                .sum()
        };
        if cost(1, &oy) < cost(0, &ox) {
            sweep_y(d, &oy, &ox);
            return;
        }
    }
    sweep_x(d, &ox);
}

/// Regions of touching rulings/boxes, in order of their first primitive (proto.py `regions`).
pub(crate) fn regions(p: &Prims) -> Vec<Region> {
    regions_with(p, false)
}

/// [`regions`]; `skip_background` leaves out the fills marked by [`mark_backgrounds`] (member indices stay).
pub(crate) fn regions_with(p: &Prims, skip_background: bool) -> Vec<Region> {
    let mut boxes: Vec<[f64; 4]> = Vec::new();
    boxes.extend(p.hs.iter().map(|h| [h.a, h.y - 0.5, h.b, h.y + 0.5]));
    boxes.extend(p.vs.iter().map(|v| [v.x, v.a, v.x + 0.5, v.b]));
    let (nh, nv) = (p.hs.len(), p.vs.len());
    // Box index -> member index (rects keep their position among all rects).
    let mut member: Vec<usize> = (0..nh + nv).collect();
    for (k, r) in p.rects.iter().enumerate() {
        if (r.x1 - r.x0) < REGION_RECT_MAX_W
            && (r.b - r.t) < REGION_RECT_MAX_H
            && !(skip_background && r.background)
        {
            boxes.push([r.x0, r.t, r.x1, r.b]);
            member.push(nh + nv + k);
        }
    }
    let n = boxes.len();
    let mut d = Dsu::new(n);
    link_touching(&boxes, &mut d);
    // Components in order of their first primitive (Python dict insertion order).
    let mut slot: Vec<Option<usize>> = vec![None; n];
    let mut comps: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        let r = d.find(i);
        match slot[r] {
            Some(k) => comps[k].push(i),
            None => {
                slot[r] = Some(comps.len());
                comps.push(vec![i]);
            }
        }
    }
    comps
        .iter()
        .filter_map(|c| {
            let f = |k: usize, max: bool| {
                let it = c.iter().map(|&i| boxes[i][k]);
                if max {
                    it.fold(f64::MIN, f64::max)
                } else {
                    it.fold(f64::MAX, f64::min)
                }
            };
            let r = Region {
                x0: f(0, false),
                y0: f(1, false),
                x1: f(2, true),
                y1: f(3, true),
                members: c.iter().map(|&i| member[i]).collect(),
            };
            (c.len() >= REGION_MIN_PRIMS
                && r.x1 - r.x0 > REGION_MIN_W
                && r.y1 - r.y0 > REGION_MIN_H)
                .then_some(r)
        })
        .collect()
}

#[cfg(test)]
#[path = "prims_test.rs"]
mod tests;
