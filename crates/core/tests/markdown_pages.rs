use plaindoc_core::*;

fn page(index: usize, blocks: Vec<Block>) -> Section {
    let mut s = Section::new(SectionKind::Page, index);
    s.blocks = blocks;
    s
}

fn p(t: &str) -> Block {
    Block::paragraph(t, None)
}

fn pages() -> Document {
    Document::new(vec![
        page(0, vec![p("one")]),
        page(1, vec![p("two")]),
        page(2, vec![p("three")]),
    ])
}

fn with(d: &Document, sep: PageSeparator) -> String {
    let mut o = MarkdownOptions::default();
    o.page_separator = sep;
    d.to_markdown_with(&o)
}

#[test]
fn rule_between_pages_only() {
    let md = pages().to_markdown();
    assert_eq!(md, "one\n\n---\n\ntwo\n\n---\n\nthree\n");
    assert_eq!(md.lines().filter(|l| *l == "---").count(), 2);
}

#[test]
fn comment_mode_is_the_old_output() {
    assert_eq!(
        with(&pages(), PageSeparator::Comment),
        "<!-- page 1 -->\n\none\n\n<!-- page 2 -->\n\ntwo\n\n<!-- page 3 -->\n\nthree\n"
    );
}

#[test]
fn none_mode_emits_nothing() {
    assert_eq!(with(&pages(), PageSeparator::None), "one\n\ntwo\n\nthree\n");
}

#[test]
fn single_page_has_no_rule() {
    let d = Document::new(vec![page(0, vec![p("only")])]);
    assert_eq!(d.to_markdown(), "only\n");
}

#[test]
fn page_starting_with_table_or_list_is_separated() {
    let table = Block::Table {
        rows: vec![
            vec![Cell::new("a"), Cell::new("b")],
            vec![Cell::new("1"), Cell::new("2")],
        ],
        header_rows: 1,
        bbox: None,
    };
    let d = Document::new(vec![
        page(0, vec![p("text")]),
        page(1, vec![table]),
        page(2, vec![p("tail")]),
    ]);
    assert_eq!(
        d.to_markdown(),
        "text\n\n---\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n---\n\ntail\n"
    );
}

#[test]
fn warnings_follow_the_rule() {
    let mut second = page(1, vec![p("two")]);
    second
        .warnings
        .push(Warning::new(WarningKind::NoTextLayer, Level::Loss, "none"));
    let d = Document::new(vec![page(0, vec![p("one")]), second]);
    assert_eq!(
        d.to_markdown(),
        "one\n\n---\n\n<!-- warning(loss) NoTextLayer: none -->\n\ntwo\n"
    );
}

#[test]
fn sheets_and_slides_keep_their_comments() {
    let mut s = Section::new(SectionKind::Sheet, 0);
    s.name = Some("Data".into());
    s.blocks.push(p("x"));
    let mut t = Section::new(SectionKind::Slide, 1);
    t.blocks.push(p("y"));
    let d = Document::new(vec![s, t]);
    assert_eq!(
        d.to_markdown(),
        "<!-- sheet 1: Data -->\n\nx\n\n<!-- slide 2 -->\n\ny\n"
    );
}
