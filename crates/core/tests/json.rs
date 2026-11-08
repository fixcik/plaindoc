use plaindoc_core::*;
use serde_json::{Value, json};

#[test]
fn json_shape_is_tagged_and_snake_case() {
    let mut s = Section::new(SectionKind::Sheet, 0);
    let mut c = Cell::new("1");
    let mut value = CellValue::new(Some(ScalarValue::Number(1.0)));
    value.formula = Some("A1".into());
    c.value = Some(value);
    s.blocks.push(Block::Table {
        rows: vec![vec![c]],
        header_rows: 0,
        bbox: None,
    });
    s.blocks
        .push(Block::heading(1, "H", Some(BBox::new(0.0, 1.0, 2.0, 3.0))));
    let mut d = Document::new(vec![s]);
    d.metadata.format = Some(Format::Csv);
    d.warnings
        .push(Warning::new(WarningKind::TableUncertain, Level::Warn, "w"));
    let v: Value = serde_json::from_str(&d.to_json()).unwrap();
    assert_eq!(v["metadata"]["format"], json!("csv"));
    assert_eq!(
        v["warnings"][0],
        json!({"kind": "TableUncertain", "level": "warn", "message": "w"})
    );
    let sec = &v["sections"][0];
    assert_eq!(sec["kind"], json!("sheet"));
    assert_eq!(sec["source"], json!("text_layer"));
    assert_eq!(sec["blocks"][0]["type"], json!("table"));
    assert_eq!(
        sec["blocks"][0]["rows"][0][0],
        json!({
            "blocks": [{"type": "paragraph", "text": "1", "bbox": null}],
            "colspan": 1,
            "rowspan": 1,
            "value": {"value": {"type": "number", "value": 1.0}, "formula": "A1", "number_format": null}
        })
    );
    assert_eq!(
        sec["blocks"][1],
        json!({"type": "heading", "level": 1, "text": "H", "bbox": {"x": 0.0, "y": 1.0, "w": 2.0, "h": 3.0}})
    );
}

#[test]
fn warning_code_is_serialized_only_when_set() {
    let mut w = Warning::new(WarningKind::HiddenContent, Level::Info, "m");
    let v: Value = serde_json::to_value(&w).unwrap();
    assert!(v.get("code").is_none(), "{v}");
    w.code = Some("tiny");
    assert_eq!(serde_json::to_value(&w).unwrap()["code"], json!("tiny"));
}
