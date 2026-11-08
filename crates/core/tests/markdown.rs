use plaindoc_core::*;

fn body(blocks: Vec<Block>) -> Document {
    let mut s = Section::new(SectionKind::Body, 0);
    s.blocks = blocks;
    Document::new(vec![s])
}

fn row(cells: &[&str]) -> Vec<Cell> {
    cells.iter().map(|t| Cell::new(*t)).collect()
}

#[test]
fn headings_paragraphs_code() {
    let d = body(vec![
        Block::heading(2, "Title", None),
        Block::paragraph("Hello", None),
        Block::heading(9, "Deep", None),
        Block::Code {
            lang: Some("rust".into()),
            text: "fn x() {}\n".into(),
            bbox: None,
        },
    ]);
    assert_eq!(
        d.to_markdown(),
        "## Title\n\nHello\n\n###### Deep\n\n```rust\nfn x() {}\n```\n"
    );
}

#[test]
fn code_fence_longer_than_content_fence() {
    let d = body(vec![Block::Code {
        lang: None,
        text: "```\ninner\n```".into(),
        bbox: None,
    }]);
    assert_eq!(d.to_markdown(), "````\n```\ninner\n```\n````\n");
}

#[test]
fn nested_lists() {
    let mut a = ListItem::new("a");
    a.blocks.push(Block::List {
        ordered: true,
        items: vec![ListItem::new("a1")],
        bbox: None,
    });
    let d = body(vec![
        Block::List {
            ordered: false,
            items: vec![a, ListItem::new("b")],
            bbox: None,
        },
        Block::List {
            ordered: true,
            items: vec![ListItem::new("x"), ListItem::new("y")],
            bbox: None,
        },
    ]);
    assert_eq!(d.to_markdown(), "- a\n  1. a1\n- b\n\n1. x\n2. y\n");
}

#[test]
fn simple_table_is_gfm_with_padding_and_escaping() {
    let d = body(vec![Block::Table {
        rows: vec![
            row(&["h1", "h2"]),
            row(&["a|b", "line1\nline2"]),
            row(&["only"]),
        ],
        header_rows: 1,
        bbox: None,
    }]);
    assert_eq!(
        d.to_markdown(),
        "| h1 | h2 |\n| --- | --- |\n| a\\|b | line1<br>line2 |\n| only |  |\n"
    );
}

#[test]
fn table_without_header_gets_empty_gfm_header() {
    let d = body(vec![Block::Table {
        rows: vec![row(&["a", "b"])],
        header_rows: 0,
        bbox: None,
    }]);
    assert_eq!(d.to_markdown(), "|  |  |\n| --- | --- |\n| a | b |\n");
}

#[test]
fn warnings_as_comments_and_switch_off() {
    let mut s = Section::new(SectionKind::Page, 0);
    s.warnings.push(Warning::new(
        WarningKind::NoTextLayer,
        Level::Loss,
        "no text --> here",
    ));
    s.warnings.push(Warning::new(
        WarningKind::OcrUsed,
        Level::Info,
        "info is hidden",
    ));
    s.blocks.push(Block::paragraph("p", None));
    let mut d = Document::new(vec![s]);
    d.warnings
        .push(Warning::new(WarningKind::Truncated, Level::Warn, "cut"));
    assert_eq!(
        d.to_markdown(),
        "<!-- warning(warn) Truncated: cut -->\n\n<!-- warning(loss) NoTextLayer: no text - -> here -->\n\np\n"
    );
    let mut o = MarkdownOptions::default();
    o.page_separator = PageSeparator::Comment;
    assert_eq!(
        d.to_markdown_with(&o),
        "<!-- warning(warn) Truncated: cut -->\n\n<!-- page 1 -->\n\n<!-- warning(loss) NoTextLayer: no text - -> here -->\n\np\n"
    );
    o.warnings = false;
    assert_eq!(d.to_markdown_with(&o), "<!-- page 1 -->\n\np\n");
}

#[test]
fn sheet_marker_with_name_and_notes_images() {
    let mut s = Section::new(SectionKind::Sheet, 1);
    s.name = Some("Data".into());
    s.blocks.push(Block::Note {
        kind: NoteKind::Footnote,
        text: "f".into(),
        bbox: None,
    });
    s.blocks.push(Block::Note {
        kind: NoteKind::Header,
        text: "h".into(),
        bbox: None,
    });
    s.blocks.push(Block::Image {
        alt: Some("pic".into()),
        ocr_text: None,
        bbox: None,
        image: None,
    });
    assert_eq!(
        Document::new(vec![s]).to_markdown(),
        "<!-- sheet 2: Data -->\n\n> Footnote: f\n\n<!-- image: pic -->\n"
    );
}

#[test]
fn empty_document_is_empty_string() {
    assert_eq!(Document::new(vec![]).to_markdown(), "");
    assert_eq!(body(vec![]).to_markdown(), "");
}

#[test]
fn list_item_with_table_is_indented() {
    let mut item = ListItem::new("see");
    item.blocks.push(Block::Table {
        rows: vec![row(&["h"]), row(&["v"])],
        header_rows: 1,
        bbox: None,
    });
    let d = body(vec![Block::List {
        ordered: false,
        items: vec![item],
        bbox: None,
    }]);
    assert_eq!(d.to_markdown(), "- see\n\n  | h |\n  | --- |\n  | v |\n");
}

#[test]
fn new_note_kinds() {
    let note = |kind, t: &str| Block::Note {
        kind,
        text: t.into(),
        bbox: None,
    };
    let d = body(vec![
        note(NoteKind::PageNumber, "3"),
        note(NoteKind::Marginal, "m"),
        note(NoteKind::Endnote, "e"),
        note(NoteKind::Comment, "c"),
    ]);
    assert_eq!(
        d.to_markdown(),
        "<!-- marginal: m -->\n\n> Endnote: e\n\n> Comment: c\n"
    );
}

#[test]
fn footer_note_is_comment() {
    let d = body(vec![Block::Note {
        kind: NoteKind::Footer,
        text: "f".into(),
        bbox: None,
    }]);
    let mut o = MarkdownOptions::default();
    o.headers_footers = true;
    assert_eq!(d.to_markdown_with(&o), "<!-- footer: f -->\n");
}

#[test]
fn notes_are_hidden_by_default_and_shown_by_option() {
    let note = |kind, t: &str| Block::Note {
        kind,
        text: t.into(),
        bbox: None,
    };
    let d = body(vec![
        note(NoteKind::Header, "h"),
        Block::paragraph("p", None),
        note(NoteKind::Marginal, "m"),
        note(NoteKind::Footnote, "f"),
        note(NoteKind::PageNumber, "3"),
        note(NoteKind::Footer, "ft"),
    ]);
    assert_eq!(
        d.to_markdown(),
        "p\n\n<!-- marginal: m -->\n\n> Footnote: f\n"
    );
    let mut o = MarkdownOptions::default();
    o.headers_footers = true;
    assert_eq!(
        d.to_markdown_with(&o),
        "<!-- header: h -->\n\np\n\n<!-- marginal: m -->\n\n> Footnote: f\n\n<!-- page-number: 3 -->\n\n<!-- footer: ft -->\n"
    );
}

#[test]
fn escaping_in_cells_comments_and_fences() {
    let d = body(vec![
        Block::Table {
            rows: vec![row(&["<b>x</b>"])],
            header_rows: 0,
            bbox: None,
        },
        Block::Image {
            alt: Some("a --> b".into()),
            ocr_text: Some("txt".into()),
            bbox: None,
            image: None,
        },
        Block::Note {
            kind: NoteKind::SpeakerNotes,
            text: "l1\nl2".into(),
            bbox: None,
        },
    ]);
    assert_eq!(
        d.to_markdown(),
        "|  |\n| --- |\n| \\<b>x\\</b> |\n\n<!-- image: a - -> b -->\n\ntxt\n\n> Notes: l1\n> l2\n"
    );
}

#[test]
fn merged_cells_render_as_html_table() {
    let d = body(vec![Block::Table {
        rows: vec![vec![Cell::new("H").with_span(2, 1)], row(&["a<b", "c"])],
        header_rows: 1,
        bbox: None,
    }]);
    assert_eq!(
        d.to_markdown(),
        "<table>\n<thead>\n<tr><th colspan=\"2\">H</th></tr>\n</thead>\n<tbody>\n<tr><td>a&lt;b</td><td>c</td></tr>\n</tbody>\n</table>\n"
    );
}

#[test]
fn nested_blocks_in_cell_force_html() {
    let mut c = Cell::new("x");
    c.blocks.push(Block::List {
        ordered: false,
        items: vec![ListItem::new("i")],
        bbox: None,
    });
    let d = body(vec![Block::Table {
        rows: vec![vec![c]],
        header_rows: 0,
        bbox: None,
    }]);
    assert_eq!(
        d.to_markdown(),
        "<table>\n<tbody>\n<tr><td>x<ul><li>i</li></ul></td></tr>\n</tbody>\n</table>\n"
    );
}

#[test]
fn multiple_header_rows_use_html_and_header_count_is_clamped() {
    let d = body(vec![Block::Table {
        rows: vec![row(&["a"]), row(&["b"]), row(&["c"])],
        header_rows: 2,
        bbox: None,
    }]);
    assert_eq!(
        d.to_markdown(),
        "<table>\n<thead>\n<tr><th>a</th></tr>\n<tr><th>b</th></tr>\n</thead>\n<tbody>\n<tr><td>c</td></tr>\n</tbody>\n</table>\n"
    );
    let d = body(vec![Block::Table {
        rows: vec![row(&["a"])],
        header_rows: 5,
        bbox: None,
    }]);
    assert_eq!(
        d.to_markdown(),
        "<table>\n<thead>\n<tr><th>a</th></tr>\n</thead>\n</table>\n"
    );
}

#[test]
fn cell_content_keeps_order_text_table_text() {
    let mut c = Cell::new("E");
    c.blocks.push(Block::Table {
        rows: vec![row(&["t"])],
        header_rows: 0,
        bbox: None,
    });
    c.blocks.push(Block::paragraph("F", None));
    let d = body(vec![Block::Table {
        rows: vec![vec![c]],
        header_rows: 0,
        bbox: None,
    }]);
    assert_eq!(
        d.to_markdown(),
        "<table>\n<tbody>\n<tr><td>E<table>\n<tbody>\n<tr><td>t</td></tr>\n</tbody>\n</table>F</td></tr>\n</tbody>\n</table>\n"
    );
}

#[test]
fn cell_lone_cr_becomes_br() {
    let d = body(vec![Block::Table {
        rows: vec![row(&["a\rb\r\nc"])],
        header_rows: 0,
        bbox: None,
    }]);
    assert_eq!(d.to_markdown(), "|  |\n| --- |\n| a<br>b<br>c |\n");
}

#[test]
fn cell_backslash_is_escaped_first() {
    let d = body(vec![Block::Table {
        rows: vec![row(&["a\\|b", "\\<x>"])],
        header_rows: 0,
        bbox: None,
    }]);
    assert_eq!(
        d.to_markdown(),
        "|  |  |\n| --- | --- |\n| a\\\\\\|b | \\\\\\<x> |\n"
    );
}

#[test]
fn list_item_starting_with_code_has_no_blank_after_marker() {
    let mut item = ListItem::new("");
    item.blocks.push(Block::Code {
        lang: None,
        text: "x".into(),
        bbox: None,
    });
    item.blocks.push(Block::paragraph("after", None));
    let d = body(vec![Block::List {
        ordered: false,
        items: vec![item],
        bbox: None,
    }]);
    assert_eq!(d.to_markdown(), "-\n  ```\n  x\n  ```\n\n  after\n");
}

#[test]
fn html_cells_have_no_raw_line_breaks() {
    let mut c = Cell::new("x").with_span(2, 1);
    c.blocks.push(Block::Code {
        lang: None,
        text: "a\n\n# b".into(),
        bbox: None,
    });
    c.blocks.push(Block::paragraph("p\r\rq", None));
    let d = body(vec![Block::Table {
        rows: vec![vec![c]],
        header_rows: 0,
        bbox: None,
    }]);
    assert_eq!(
        d.to_markdown(),
        "<table>\n<tbody>\n<tr><td colspan=\"2\">x<p>a<br><br># b</p>p<br><br>q</td></tr>\n</tbody>\n</table>\n"
    );
}

#[test]
fn paragraph_start_that_reads_as_markup_is_escaped() {
    let para = |t: &str| Block::paragraph(t, None);
    let d = body(
        [
            "# not a heading",
            "> not a quote",
            "| not | a table",
            "12. not a list",
            "3) neither",
            "3.14 is a number",
            "2024 was a year",
            "plain # text",
            "___",
            "- - -",
            " ** _",
            "__init__ stays",
        ]
        .into_iter()
        .map(para)
        .collect(),
    );
    assert_eq!(
        d.to_markdown(),
        "\\# not a heading\n\n\\> not a quote\n\n\\| not | a table\n\n12\\. not a list\n\n\
         3\\) neither\n\n3.14 is a number\n\n2024 was a year\n\nplain # text\n\n\\___\n\n\\- - -\n\n \\** _\n\n\
         __init__ stays\n"
    );
}

#[test]
fn header_footer_notes_in_list_items_are_hidden_by_default() {
    let note = |kind, t: &str| Block::Note {
        kind,
        text: t.into(),
        bbox: None,
    };
    let mut only_note = ListItem::new("x");
    only_note.blocks = vec![note(NoteKind::Footer, "f")];
    let mut a = ListItem::new("a");
    a.blocks.push(note(NoteKind::PageNumber, "7"));
    let list = |ordered| {
        body(vec![Block::List {
            ordered,
            items: vec![only_note.clone(), a.clone()],
            bbox: None,
        }])
    };
    assert_eq!(list(false).to_markdown(), "- a\n");
    assert_eq!(list(true).to_markdown(), "1. a\n");
    let mut o = MarkdownOptions::default();
    o.headers_footers = true;
    assert_eq!(
        list(false).to_markdown_with(&o),
        "-\n  <!-- footer: f -->\n- a\n\n  <!-- page-number: 7 -->\n"
    );
}
