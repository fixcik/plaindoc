use plaindoc_core::*;
use serde_json::{Value, json};

fn body(blocks: Vec<Block>) -> Document {
    let mut s = Section::new(SectionKind::Body, 0);
    s.blocks = blocks;
    Document::new(vec![s])
}

fn linked(text: &str, links: Vec<Link>) -> Block {
    let mut b = Block::paragraph(text, None);
    if let Block::Paragraph { links: l, .. } = &mut b {
        *l = links;
    }
    b
}

#[test]
fn paragraph_and_heading_links_render_inline() {
    let mut h = Block::heading(2, "Docs at site", None);
    if let Block::Heading { links, .. } = &mut h {
        links.push(Link::new(8, 12, "https://e.org/"));
    }
    let d = body(vec![
        h,
        linked(
            "Write to me or see [x] here",
            vec![
                Link::new(9, 14, "mailto:a@e.org"),
                Link::new(19, 22, "https://e.org/a b(1)"),
            ],
        ),
    ]);
    assert_eq!(
        d.to_markdown(),
        "## Docs at [site](https://e.org/)\n\n\
         Write to [me or](mailto:a@e.org) see [\\[x\\]](https://e.org/a%20b%281%29) here\n"
    );
}

#[test]
fn paragraph_start_is_escaped_after_the_links() {
    let d = body(vec![linked(
        "1. item",
        vec![Link::new(3, 7, "https://e.org")],
    )]);
    assert_eq!(d.to_markdown(), "1\\. [item](https://e.org)\n");
}

#[test]
fn gfm_and_html_cells_carry_links() {
    let cell = |t: &str, l: Vec<Link>| {
        let mut c = Cell::new("");
        c.blocks = vec![linked(t, l)];
        c
    };
    let rows = vec![
        vec![Cell::new("name"), Cell::new("site")],
        vec![
            Cell::new("a|b"),
            cell("go | [here]", vec![Link::new(5, 11, "https://e.org/x|y")]),
        ],
    ];
    let d = body(vec![Block::Table {
        rows: rows.clone(),
        header_rows: 1,
        bbox: None,
    }]);
    assert_eq!(
        d.to_markdown(),
        "| name | site |\n| --- | --- |\n| a\\|b | go \\| [\\[here\\]](https://e.org/x%7Cy) |\n"
    );
    let mut merged = rows;
    merged[0][0] = Cell::new("name").with_span(1, 2);
    let d = body(vec![Block::Table {
        rows: merged,
        header_rows: 1,
        bbox: None,
    }]);
    let md = d.to_markdown();
    assert!(
        md.contains("<td>go | <a href=\"https://e.org/x%7Cy\">[here]</a></td>"),
        "{md}"
    );
}

#[test]
fn links_are_in_json_only_when_present() {
    let d = body(vec![
        Block::paragraph("plain", None),
        linked("see site", vec![Link::new(4, 8, "https://e.org")]),
    ]);
    let v: Value = serde_json::from_str(&d.to_json()).unwrap();
    let blocks = &v["sections"][0]["blocks"];
    assert_eq!(
        blocks[0],
        json!({"type": "paragraph", "text": "plain", "bbox": null})
    );
    assert_eq!(
        blocks[1]["links"],
        json!([{"start": 4, "end": 8, "url": "https://e.org"}])
    );
}

#[test]
fn links_are_on_by_default() {
    assert!(ParseOptions::default().links);
}
