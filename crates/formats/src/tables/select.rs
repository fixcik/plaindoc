//! False-table filters, the hybrid structure choice and header rows (D11).

use super::TableConfig;
use super::detect::{Candidate, CandidateKind, within};
use super::features::{Features, is_num};
use super::lattice::Planar;

/// D11: logistic model threshold (results_final.txt §2, "компромисс по holdout ODL").
pub(crate) const LOGIT_THRESHOLD: f64 = 0.10;
/// D11 hybrid: planar grid only when this share of elementary edges is drawn (hybrid_sweep_guard.txt,
/// ecov=0.7 best on test; filters.py default 1.0 is NOT the working point).
const HYBRID_MIN_EDGE_COV: f64 = 0.7;
const HYBRID_MIN_CELLS: usize = 4;
/// D11 two-sided row guard: planar rows within 0.6–1.4× of prototype rows (filters_on_synth.md §3).
const HYBRID_ROWS_MIN: f64 = 0.6;
const HYBRID_ROWS_MAX: f64 = 1.4;

/// Logistic model from `docs/design/spikes/tables-corpus/results/filters_logit.json`
/// (trained by filters.py on pt1m-even + icdar-even; D11: weights unchanged).
/// Order: `[name, log1p?, weight, mean, sd]`; the 31st input is `kind_T`.
const LOGIT_B: f64 = 0.924_926_308_868_837_8;
#[rustfmt::skip]
const LOGIT: [(&str, bool, f64, f64, f64); 30] = [
    ("nrows", true, 0.23280723174143753, 2.1607308959503815, 0.6854841376816201),
    ("ncols", true, 0.20800316897051907, 1.7442989226987269, 0.38929621850662727),
    ("fill", false, 0.6289671972321976, 0.83638893129771, 0.17322008292744875),
    ("mean_words", false, -0.2840066509632427, 2.69010286259542, 5.270731348909495),
    ("num_frac", false, -0.04377335936639671, 0.29931641221374045, 0.2883869232103475),
    ("numcol_frac", false, -0.0717164025588454, 0.292562213740458, 0.3265266932959601),
    ("sparse_rows", false, -0.245241262464273, 0.11574770992366412, 0.2137413368953441),
    ("line_w_mean", false, -0.17187109303680445, 0.7110049618320611, 0.225463071852739),
    ("nlines", true, 0.4832401689255929, 2.3877364696760046, 0.6835999394828319),
    ("n_h", true, -0.37349397040775356, 1.4336647203897, 0.9444907997068415),
    ("n_v", true, -0.7076348620145544, 0.8863633590327076, 1.0888963602757948),
    ("h_len", false, 0.7654746174259405, 0.495075, 0.5228788894016188),
    ("v_len", false, -0.24296884497994653, 0.5034179389312977, 2.0280911390244882),
    ("n_paths", true, 0.673574804589757, 2.433501651984586, 1.6131721610148626),
    ("n_segs", true, -0.45214479122174195, 0.18653875690031316, 0.8620522038075118),
    ("n_diag", true, -0.14093413161628898, 0.03713972669071773, 0.3923543688212176),
    ("fill_area", false, -0.3339861696582651, 0.22223187022900764, 0.37118645897174535),
    ("paths_per_word", true, 0.12799289775254105, 0.44287679561107024, 0.5700279491022523),
    ("row_h_cv", false, -0.8772100666056555, 0.3659837786259542, 0.23736952194161468),
    ("align_cv", false, -0.4813028072095078, 0.05137137404580153, 0.06819097607985007),
    ("w_frac", false, 0.46465690994559267, 0.68218893129771, 0.18394050541947626),
    ("h_frac", false, 0.33491603640158885, 0.20475973282442747, 0.15877880288302587),
    ("fs_ratio", false, -0.3173253841287676, 0.8918557251908397, 0.1385253112486493),
    ("first_col_words", false, -0.14495021137945988, 2.468995992366412, 2.3307298928856617),
    ("last_col_num", false, 0.24989471591679469, 0.29674980916030536, 0.3963462728906565),
    ("header_like", false, 0.21349031478103894, 0.3797709923664122, 0.4853297700774626),
    ("word_share", false, 0.7542099265868402, 0.18509809160305343, 0.19393938022605128),
    ("nw", true, 0.27097886118796966, 4.057827694111525, 0.9784554769999154),
    ("short_frac", false, 0.7139818635228314, 0.8439137404580153, 0.20752520550060913),
    ("row_h_fs", false, -0.8429903177198219, 2.483270419847328, 2.198280499275569),
];
const LOGIT_KIND_T: (f64, f64, f64) = (
    -0.039453723998668117,
    0.5687022900763359,
    0.49525750407062685,
);

/// Probability that a candidate is a real table.
pub(crate) fn probability(f: &Features, kind: CandidateKind) -> f64 {
    let named = f.named();
    let get = |k: &str| named.iter().find(|(n, _)| *n == k).map_or(0.0, |x| x.1);
    let mut z = LOGIT_B;
    for (name, log, w, mu, sd) in LOGIT {
        let x = get(name);
        let x = if log { x.max(0.0).ln_1p() } else { x };
        z += w * (x - mu) / sd;
    }
    // A model region's grid is text-built (SP4-R1): `kind_T` is 1 for it as for `T`.
    let kt = f64::from(u8::from(kind != CandidateKind::Lined));
    z += LOGIT_KIND_T.0 * (kt - LOGIT_KIND_T.1) / LOGIT_KIND_T.2;
    1.0 / (1.0 + (-z.clamp(-30.0, 30.0)).exp())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// Chart, axis labels or legend (rule `chart`).
    Chart,
    /// At most 4 cells or fewer than 8 words (rule `tiny`).
    Tiny,
    /// Half of the rows have at most one filled cell (rule `sparse`).
    Sparse,
    /// Two-column prose (rule `twocol`, D11).
    TwoColumnProse,
    /// Separate boxes in flow, such as tag chips (rule `box_flow`, SP8b S3).
    BoxFlow,
    /// Logistic model below the threshold.
    LowProbability,
    /// Most words of the candidate are not upright: a landscape table on a portrait page, read in the
    /// unrotated frame, gives garbage cells (not in the prototype; SP4 review I2).
    Rotated,
    /// Table of contents or equation numbers on a scan ([`super::TableConfig::scan_text_guards`]).
    ScanDecoy,
}

/// filters.py `rule_chart`.
fn rule_chart(f: &Features) -> bool {
    (f.num_frac >= 0.5 && f.fill <= 0.55)
        || f.n_diag >= 8.0
        || (f.fill_area >= 0.5 && f.line_w_mean <= 0.3 && f.n_v <= 2.0)
}
/// filters.py `rule_tiny`.
fn rule_tiny(f: &Features) -> bool {
    f.nrows * f.ncols <= 4.0 || f.nw < 8.0
}
/// filters.py `rule_sparse`.
fn rule_sparse(f: &Features) -> bool {
    f.sparse_rows >= 0.5
}
/// D11 `twocol` (fsyn.py `rule_twocol`; thresholds fitted on synthetic decoys, small margin).
fn rule_twocol(f: &Features, k: CandidateKind) -> bool {
    k == CandidateKind::Text
        && f.ncols == 2.0
        && f.numcols == 0.0
        && f.col_w_cv <= 0.05
        && f.line_w_mean >= 0.85
        && f.mean_words >= 3.5
}

/// First failed rule, in filters order.
pub(crate) fn rule_reject(f: &Features, k: CandidateKind) -> Option<RejectReason> {
    if rule_chart(f) {
        Some(RejectReason::Chart)
    } else if rule_tiny(f) {
        Some(RejectReason::Tiny)
    } else if rule_sparse(f) {
        Some(RejectReason::Sparse)
    } else if rule_twocol(f, k) {
        Some(RejectReason::TwoColumnProse)
    } else {
        None
    }
}

/// run.py axis guard (T14): a `T` candidate wider than this many columns ...
const AXIS_MAX_COLS: f64 = 15.0;
/// ... with no token of at least this many letters is chart axis labels (ODL `01030000000076`, dates).
const AXIS_MIN_LETTERS: usize = 3;

/// run.py `reject`: `k == 'T' and ncols > 15 and alpha3 == 0` (`alpha3`: tokens of the filled cells with
/// at least three `isalpha` chars).
pub(crate) fn rule_axis(c: &Candidate) -> bool {
    c.kind == CandidateKind::Text
        && c.features.ncols > AXIS_MAX_COLS
        && !c
            .rows
            .iter()
            .flatten()
            .flat_map(|t| t.split_whitespace())
            .any(|tok| tok.chars().filter(|ch| ch.is_alphabetic()).count() >= AXIS_MIN_LETTERS)
}

/// [`super::TableConfig::scan_text_guards`]: a text block of table-of-contents lines or with a last column
/// of equation numbers only.
pub(crate) fn scan_decoy(c: &Candidate) -> bool {
    if !c.text_block {
        return false;
    }
    let digits = |t: &str, max: usize| {
        !t.is_empty() && t.len() <= max && t.chars().all(|ch| ch.is_ascii_digit())
    };
    let letters = |t: &str| t.chars().any(char::is_alphabetic);
    // Title, page number and at most one more cell; the leader ends the (lettered) cell before the page
    // number, or OCR glued its last dot to the number (`.12`) after a title ending in a dot. A value
    // column such as `.45` / `.05` or `..` as a missing-data marker is not a leader.
    let toc_line = |r: &Vec<String>| {
        let cells: Vec<&str> = r
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        let n = cells.len();
        if !(2..=3).contains(&n) {
            return false;
        }
        let (first, prev, last) = (cells[0], cells[n - 2], cells[n - 1]);
        letters(first)
            && letters(prev)
            && digits(last.trim_start_matches('.'), 4)
            && (prev.ends_with("..")
                || prev.ends_with(". .")
                || (last.starts_with('.') && prev.ends_with('.')))
    };
    let toc = c.rows.iter().filter(|r| toc_line(r)).count();
    if toc >= 2 && 2 * toc >= c.rows.len() {
        return true;
    }
    let ends: Vec<&str> = c
        .rows
        .iter()
        .filter(|r| r.len() >= 2)
        .filter_map(|r| r.last().map(|t| t.trim()))
        .filter(|t| !t.is_empty())
        .collect();
    ends.len() >= 2
        && ends.iter().all(|t| {
            // `(1`, `1)`: OCR often loses one bracket of an equation number; a bare number is not one.
            let n = t.trim_start_matches('(').trim_end_matches(')');
            n.len() < t.len() && digits(n, 3)
        })
}

/// First reject reason after the rotation check, in run.py order: D11 rules, the T15 gutter guard, the
/// T14 axis guard, then the model probability `p` against `threshold`. Shared by [`select`] and the
/// parity diff so the oracle checks the shipped chain.
pub(crate) fn filter_reason(
    c: &Candidate,
    p: f64,
    cfg: &TableConfig,
    threshold: f64,
) -> Option<RejectReason> {
    rule_reject(&c.features, c.kind)
        .or_else(|| c.gutter_prose.then_some(RejectReason::TwoColumnProse))
        .or_else(|| c.box_flow.then_some(RejectReason::BoxFlow))
        .or_else(|| (cfg.axis_guard && rule_axis(c)).then_some(RejectReason::Chart))
        .or_else(|| (cfg.scan_text_guards && scan_decoy(c)).then_some(RejectReason::ScanDecoy))
        .or_else(|| (p < threshold).then_some(RejectReason::LowProbability))
}

/// D11 hybrid with the two-sided row guard (hybguard.py `qual(r, 1.4)`).
pub(crate) fn use_planar(c: &Candidate) -> bool {
    let Some(pl) = &c.planar else { return false };
    let n = c.rows.len() as f64;
    c.kind == CandidateKind::Lined
        && pl.edge_cov >= HYBRID_MIN_EDGE_COV
        && pl.ncells >= HYBRID_MIN_CELLS
        && pl.nonrect == 0
        && pl.nrows >= 2
        && pl.ncols >= 2
        && pl.nrows as f64 >= HYBRID_ROWS_MIN * n
        && pl.nrows as f64 <= HYBRID_ROWS_MAX * n
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridCell {
    pub text: String,
    pub colspan: u32,
    pub rowspan: u32,
    /// Tables nested in this cell (after `text`).
    pub nested: Vec<DetectedGrid>,
}

impl GridCell {
    pub fn new(text: impl Into<String>, colspan: u32, rowspan: u32) -> Self {
        Self {
            text: text.into(),
            colspan,
            rowspan,
            nested: Vec::new(),
        }
    }

    /// Cell text followed by the text of nested tables, row by row (how ground truth flattens them).
    pub fn flat_text(&self) -> String {
        let mut parts = vec![self.text.clone()];
        for t in &self.nested {
            parts.extend(t.rows.iter().flatten().map(GridCell::flat_text));
        }
        parts.retain(|p| !p.trim().is_empty());
        parts.join(" ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Structure {
    /// Column/row bounds of the prototype; no merged cells.
    Prototype,
    /// Planar partition of the rulings; may have merged cells.
    Planar,
}

/// An accepted table.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectedGrid {
    pub kind: CandidateKind,
    /// `[x0, top, x1, bottom]`, top-left origin.
    pub bbox: [f64; 4],
    /// The part of `bbox` whose words the cells took: `bbox` for the planar partition (it takes every word of
    /// the region), the column/row bounds within it otherwise. A word of `bbox` outside it is in no cell, so
    /// the caller keeps it in the page text (final review C1, D5).
    pub cells: [f64; 4],
    /// Cells listed in the row where they start (HTML order).
    pub rows: Vec<Vec<GridCell>>,
    pub header_rows: usize,
    pub structure: Structure,
    pub probability: f64,
    pub features: Features,
}

/// A rejected candidate (feeds `TableUncertain` diagnostics).
#[derive(Clone, Debug, PartialEq)]
pub struct Rejected {
    pub kind: CandidateKind,
    pub bbox: [f64; 4],
    pub reason: RejectReason,
    pub probability: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageTables {
    pub tables: Vec<DetectedGrid>,
    pub rejected: Vec<Rejected>,
    /// Set when the page was over a hostile-input limit and was not examined.
    pub skipped: Option<super::LimitExceeded>,
}

/// Header rows (not in the prototype): the deepest rowspan of the first row; 2 when the first row has a
/// colspan and the second row is all text (grouped headers); otherwise 1. Measured on 230 single-table
/// synthetic pages: 88% exact (a "first row is all text" test gave 68%: numeric headers such as years).
pub(crate) fn header_rows(rows: &[Vec<GridCell>]) -> usize {
    let Some(first) = rows.first() else { return 0 };
    let deepest = first.iter().map(|c| c.rowspan as usize).max().unwrap_or(1);
    let spans = first.iter().any(|c| c.colspan > 1);
    let h = if deepest > 1 {
        deepest
    } else if spans
        && rows.len() > 2
        && rows[1]
            .iter()
            .all(|c| !c.text.trim().is_empty() && !is_num(&c.text))
    {
        2
    } else {
        1
    };
    h.min(rows.len().saturating_sub(1))
}

fn proto_rows(rows: Vec<Vec<String>>) -> Vec<Vec<GridCell>> {
    rows.into_iter()
        .map(|r| r.into_iter().map(|t| GridCell::new(t, 1, 1)).collect())
        .collect()
}

fn planar_rows(pl: Planar) -> Vec<Vec<GridCell>> {
    pl.rows
        .into_iter()
        .map(|r| {
            r.into_iter()
                .map(|c| GridCell::new(c.text, c.cs, c.rs))
                .collect()
        })
        .collect()
}

/// Applies rules → model → hybrid to the page candidates, keeping candidate order.
pub(crate) fn select(cands: Vec<Candidate>, cfg: &TableConfig) -> PageTables {
    let mut out = PageTables::default();
    let mut models = Vec::new();
    // Accepted tables a model region may replace (text blocks), in `out.tables` order.
    let mut replaceable = Vec::new();
    for c in cands {
        if c.kind == CandidateKind::Model {
            models.push(c);
            continue;
        }
        let p = probability(&c.features, c.kind);
        if cfg.filters {
            let reason = (c.rotated.then_some(RejectReason::Rotated))
                .or_else(|| filter_reason(&c, p, cfg, cfg.logit_threshold));
            if let Some(reason) = reason {
                out.rejected.push(Rejected {
                    kind: c.kind,
                    bbox: c.bbox,
                    reason,
                    probability: p,
                });
                continue;
            }
        }
        let planar = cfg.hybrid && use_planar(&c);
        let cells = if planar && c.planar.is_some() {
            c.bbox
        } else {
            c.cells
        };
        let rows = match (c.planar, planar) {
            (Some(pl), true) => planar_rows(pl),
            _ => proto_rows(c.rows),
        };
        // T17 `header_split`: the rows of the split header, else the span/text rule.
        let header_rows = if c.head > 0 {
            c.head.min(rows.len().saturating_sub(1))
        } else {
            header_rows(&rows)
        };
        replaceable.push(c.text_block);
        out.tables.push(DetectedGrid {
            kind: c.kind,
            bbox: c.bbox,
            cells,
            rows,
            header_rows,
            structure: if planar {
                Structure::Planar
            } else {
                Structure::Prototype
            },
            probability: p,
            features: c.features,
        });
    }
    if !models.is_empty() {
        merge_models(&mut out, models, &replaceable, cfg);
    }
    out.tables = super::nest::nest(out.tables);
    out
}

/// SP4-R1 (region-spike `hybrid.py`): an accepted lined table, rule group or fill stack with at least this
/// IoU to a model region wins and the region is dropped; an accepted text block with at least
/// [`MODEL_REPLACE_IOU`] to the region, or inside it widened by [`MODEL_REPLACE_MARGIN`] pt, is replaced by
/// the region's grid. Rule groups and fill stacks (kind `T`, not in the spike's prototype) win like lined
/// tables: replacing them too lost 0.016 TEDS_all and 0.038 borderless TEDS on the oracle set
/// (docs/tables-eval.md, "SP4-R1 model regions (oracle)").
const MODEL_LINED_IOU: f64 = 0.5;
const MODEL_REPLACE_IOU: f64 = 0.3;
const MODEL_REPLACE_MARGIN: f64 = 5.0;

fn iou(a: [f64; 4], b: [f64; 4]) -> f64 {
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let i = ix * iy;
    let u = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - i;
    if u > 0.0 { i / u } else { 0.0 }
}

/// Merges the model-region candidates into the accepted tables of the page (SP4-R1); `replaceable[i]`:
/// `out.tables[i]` is a text block. Text blocks not replaced stay (keepT: on the oracle set dropping them
/// only lowered the excess, and a real model misses regions). A region is accepted without the logistic
/// model (its grid has at least 3 rows and 2 columns by construction); the rules chart and tiny and the
/// rotation check still reject it.
fn merge_models(
    out: &mut PageTables,
    models: Vec<Candidate>,
    replaceable: &[bool],
    cfg: &TableConfig,
) {
    let mut replaced = vec![false; out.tables.len()];
    let mut added = Vec::new();
    for c in models {
        if out
            .tables
            .iter()
            .zip(replaceable)
            .any(|(t, &r)| !r && iou(t.bbox, c.bbox) >= MODEL_LINED_IOU)
        {
            continue;
        }
        let p = probability(&c.features, c.kind);
        if cfg.filters {
            let reason = (c.rotated.then_some(RejectReason::Rotated))
                .or_else(|| rule_chart(&c.features).then_some(RejectReason::Chart))
                .or_else(|| rule_tiny(&c.features).then_some(RejectReason::Tiny));
            if let Some(reason) = reason {
                out.rejected.push(Rejected {
                    kind: c.kind,
                    bbox: c.bbox,
                    reason,
                    probability: p,
                });
                continue;
            }
        }
        for ((t, &r), gone) in out.tables.iter().zip(replaceable).zip(replaced.iter_mut()) {
            if r && (iou(t.bbox, c.bbox) >= MODEL_REPLACE_IOU
                || within(c.bbox, t.bbox, MODEL_REPLACE_MARGIN))
            {
                *gone = true;
            }
        }
        let rows = proto_rows(c.rows);
        added.push(DetectedGrid {
            kind: c.kind,
            bbox: c.bbox,
            cells: c.cells,
            header_rows: header_rows(&rows),
            rows,
            structure: Structure::Prototype,
            probability: p,
            features: c.features,
        });
    }
    let mut keep = replaced.into_iter().map(|r| !r);
    out.tables.retain(|_| keep.next().unwrap_or(true));
    out.tables.extend(added);
}

#[cfg(test)]
#[path = "select_test.rs"]
mod tests;
