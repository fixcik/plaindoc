use super::*;
use plaindoc_core::Cell;

#[test]
fn only_web_and_mail_urls_are_kept() {
    let ok = |u: &str| clean_url(u);
    assert_eq!(ok("https://e.org/a").as_deref(), Some("https://e.org/a"));
    assert_eq!(ok(" HTTP://E.org ").as_deref(), Some("http://E.org"));
    assert_eq!(ok("mailto:a@e.org").as_deref(), Some("mailto:a@e.org"));
    // control and format chars are removed, not kept as a hidden part of the URL
    assert_eq!(
        ok("https://e.org/\u{202E}a\u{200B}b\u{7}").as_deref(),
        Some("https://e.org/ab")
    );
    for bad in [
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "java\u{0}script:x",
        "file:///etc/passwd",
        "data:text/html,x",
        "/relative",
        "e.org/x",
        "https://",
        "https:///path",
        "http:e.org",
        "mailto:",
        "mailto:?subject=x",
        "",
    ] {
        assert_eq!(ok(bad), None, "{bad:?}");
    }
    let long = format!("https://e.org/{}", "a".repeat(MAX_URL));
    assert_eq!(ok(&long), None);
    let edge = format!("https://e.org/{}", "a".repeat(MAX_URL - 14));
    assert_eq!(ok(&edge).map(|u| u.len()), Some(MAX_URL));
}

#[test]
fn a_glyph_is_linked_by_the_area_inside() {
    let g = Rect::new(10.0, 10.0, 10.0, 10.0);
    // half inside
    assert!(covered(&g, &[Rect::new(15.0, 0.0, 20.0, 30.0)]));
    // the centre is inside, but less than half of the area (a thin rectangle)
    assert!(!covered(&g, &[Rect::new(0.0, 14.0, 30.0, 2.0)]));
    // two rectangles add up
    assert!(covered(
        &g,
        &[
            Rect::new(10.0, 10.0, 3.0, 10.0),
            Rect::new(17.0, 10.0, 3.0, 10.0)
        ]
    ));
    // a box without area: by its centre
    assert!(covered(
        &Rect::new(5.0, 5.0, 0.0, 0.0),
        &[Rect::new(0.0, 0.0, 10.0, 10.0)]
    ));
}

fn glyphs(text: &str, x: f32, y: f32, link: impl Fn(usize) -> Option<u32>) -> Vec<Glyph> {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .enumerate()
        .map(|(i, c)| Glyph {
            c,
            x: x + 5.0 * i as f32,
            y,
            link: link(i),
        })
        .collect()
}

fn para(t: &str, x: f32, y: f32) -> Block {
    Block::paragraph(t, Some(BBox::new(x, y - 5.0, 200.0, 10.0)))
}

fn links_of(b: &Block) -> Vec<(usize, usize, &str)> {
    match b {
        Block::Paragraph { links, .. } | Block::Heading { links, .. } => links
            .iter()
            .map(|l| (l.start, l.end, l.url.as_str()))
            .collect(),
        _ => Vec::new(),
    }
}

#[test]
fn a_run_is_found_in_its_block_only() {
    // two blocks with the same words; the link is on the second block's glyphs
    let mut g = glyphs("go here", 10.0, 100.0, |_| None);
    g.extend(glyphs("go here", 10.0, 50.0, |i| (i >= 2).then_some(0)));
    let pl = PageLinks {
        urls: vec!["https://e.org".into()],
        glyphs: g,
        warnings: Vec::new(),
    };
    let mut blocks = vec![para("go here", 10.0, 100.0), para("go here", 10.0, 50.0)];
    apply(&mut blocks, &pl);
    assert_eq!(links_of(&blocks[0]), vec![]);
    assert_eq!(links_of(&blocks[1]), vec![(3, 7, "https://e.org")]);
}

#[test]
fn text_that_differs_from_the_glyphs_gets_no_link() {
    let pl = PageLinks {
        urls: vec!["https://e.org".into()],
        glyphs: glyphs("abc", 10.0, 100.0, |_| Some(0)),
        warnings: Vec::new(),
    };
    let mut blocks = vec![para("xyz", 10.0, 100.0)];
    apply(&mut blocks, &pl);
    assert_eq!(links_of(&blocks[0]), vec![]);
}

#[test]
fn a_table_links_the_cell_with_the_text() {
    let pl = PageLinks {
        urls: vec!["https://e.org/b".into()],
        glyphs: glyphs("a b c", 10.0, 100.0, |i| (i == 1).then_some(0)),
        warnings: Vec::new(),
    };
    let mut blocks = vec![Block::Table {
        rows: vec![vec![Cell::new("a"), Cell::new("b"), Cell::new("c")]],
        header_rows: 0,
        bbox: Some(BBox::new(0.0, 90.0, 100.0, 20.0)),
    }];
    apply(&mut blocks, &pl);
    let Block::Table { rows, .. } = &blocks[0] else {
        unreachable!()
    };
    let got: Vec<_> = rows[0].iter().map(|c| links_of(&c.blocks[0])).collect();
    assert_eq!(got, vec![vec![], vec![(0, 1, "https://e.org/b")], vec![]]);
}

#[test]
fn neighbours_with_one_url_join_and_overlaps_go() {
    let mut l = vec![
        Link::new(8, 12, "u"),
        Link::new(0, 3, "u"),
        Link::new(4, 7, "u"),
        Link::new(9, 11, "v"),
        Link::new(13, 14, "w"),
    ];
    tidy("our web page x y", &mut l);
    assert_eq!(l, vec![Link::new(0, 12, "u"), Link::new(13, 14, "w")]);
}

#[test]
fn nearest_occurrence_wins() {
    let hay: Vec<(char, usize, usize)> =
        "abab".chars().enumerate().map(|(i, c)| (c, 0, i)).collect();
    assert_eq!(nearest(&hay, &['a', 'b'], 3), Some(2));
    assert_eq!(nearest(&hay, &['a', 'b'], 0), Some(0));
    // an occurrence across two texts does not count
    let split = vec![('a', 0, 0), ('b', 1, 0)];
    assert_eq!(nearest(&split, &['a', 'b'], 0), None);
    assert_eq!(nearest(&hay, &[], 0), None);
}
