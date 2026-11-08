//! Table block assembly from a detected grid; source-agnostic (PDF pages and the scan path).

use plaindoc_core::{BBox, Block, Cell, Level, ParseError, Result, Warning, WarningKind};

use super::{DetectedGrid, PageTables};

/// Below this model probability an accepted table gets `TableUncertain` (results_final.txt §2:
/// raising the threshold to 0.30 would drop 2 of 45 ODL tables, so 0.10–0.30 is the doubtful band). Also the
/// bound of `TableConfig::uncertain_page_columns`: retuning it moves that guard too.
pub(crate) const UNCERTAIN_P: f64 = 0.30;
/// Below this share of filled cells an accepted table gets `TableUncertain` (spec §6 "too many empty cells").
const UNCERTAIN_FILL: f64 = 0.5;

/// Top-left `[x0, top, x1, bottom]` → model `BBox` (PDF points, bottom-left origin).
pub(crate) fn to_bbox(b: [f64; 4], page_height: f64) -> BBox {
    BBox::new(
        b[0] as f32,
        (page_height - b[3]) as f32,
        (b[2] - b[0]) as f32,
        (b[3] - b[1]) as f32,
    )
}

/// Table block of a grid; nested grids become `Block::Table` after the cell text (D5 order).
fn grid_block(t: &DetectedGrid, page_height: f64) -> Block {
    let rows = t
        .rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| {
                    let mut cell = Cell::new(c.text.clone()).with_span(c.colspan, c.rowspan);
                    cell.blocks
                        .extend(c.nested.iter().map(|n| grid_block(n, page_height)));
                    cell
                })
                .collect()
        })
        .collect();
    Block::Table {
        rows,
        header_rows: t.header_rows,
        bbox: Some(to_bbox(t.bbox, page_height)),
    }
}

/// Region (PDF points, bottom-left origin), block and `TableUncertain` warnings of a detected grid.
/// `page_height` is the page top edge in PDF y (grid coordinates are top-left, y down); x passes through
/// unchanged.
pub fn to_block(t: &DetectedGrid, page_height: f64) -> (BBox, Block, Vec<Warning>) {
    let mut warnings = Vec::new();
    if t.features.fill < UNCERTAIN_FILL {
        warnings.push(Warning::new(
            WarningKind::TableUncertain,
            Level::Warn,
            format!(
                "table structure uncertain: {:.0}% of cells are empty",
                100.0 * (1.0 - t.features.fill)
            ),
        ));
    } else if t.probability < UNCERTAIN_P {
        warnings.push(Warning::new(
            WarningKind::TableUncertain,
            Level::Warn,
            format!(
                "table detection uncertain (confidence {:.2})",
                t.probability
            ),
        ));
    }
    (
        to_bbox(t.bbox, page_height),
        grid_block(t, page_height),
        warnings,
    )
}

/// Text of a caught panic payload.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Runs the pure detector, turning its panic into an error (the page gets "table detection failed"
/// instead of the whole file failing). Wrap only the detector, not calls into foreign libraries: a
/// panic inside pdf_oxide may leave its thread-local state broken, so it must propagate to `run_caught`
/// (PARKED SP2).
pub fn guarded(detect: impl FnOnce() -> PageTables) -> Result<PageTables> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(detect)).map_err(|p| {
        ParseError::Internal(format!(
            "table detector panicked: {}",
            panic_message(p.as_ref())
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::{CandidateKind, Features, GridCell, Structure};

    #[test]
    fn to_block_bbox_spans_and_warning() {
        let cell = GridCell::new("a", 2, 1);
        let g = DetectedGrid {
            kind: CandidateKind::Lined,
            bbox: [50.0, 100.0, 250.0, 140.0],
            cells: [50.0, 100.0, 250.0, 140.0],
            rows: vec![vec![cell]],
            header_rows: 1,
            structure: Structure::Planar,
            probability: 0.2,
            features: Features {
                fill: 1.0,
                ..Features::default()
            },
        };
        let (bbox, block, w) = to_block(&g, 800.0);
        assert_eq!(bbox, BBox::new(50.0, 660.0, 200.0, 40.0));
        match block {
            Block::Table {
                rows, header_rows, ..
            } => {
                assert_eq!(header_rows, 1);
                assert_eq!(rows[0][0].colspan, 2);
            }
            b => panic!("{b:?}"),
        }
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].kind, WarningKind::TableUncertain);
    }

    #[test]
    fn detector_panic_becomes_an_error() {
        let e = guarded(|| panic!("boom")).unwrap_err();
        assert!(
            e.to_string().contains("table detector panicked: boom"),
            "{e}"
        );
    }
}
