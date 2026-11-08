use super::*;
use crate::common::MAX_CELLS;

#[test]
fn cells_share_the_document_budget() {
    let mut budget = CellBudget::new();
    budget.take(MAX_CELLS - 3).unwrap();
    // 2 + 1 cells fit; the nested table's cell is charged too
    let ok = "<table><tr><td>a<table><tr><td>n</table><td>b</table>";
    assert!(convert_with(ok, budget, None).is_ok());
    let mut budget = CellBudget::new();
    budget.take(MAX_CELLS - 3).unwrap();
    let r = convert_with(
        "<table><tr><td>a<td>b</table><table><tr><td>c<td>d</table>",
        budget,
        None,
    );
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, limit: 2_000_000 }) if kind == "table cells"),
        "{:?}",
        r.err()
    );
}
