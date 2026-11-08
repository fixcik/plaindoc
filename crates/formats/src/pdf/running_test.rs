use super::*;

fn span(text: &str, x: f32, y: f32, w: f32, size: f32) -> TextSpan {
    TextSpan {
        text: text.into(),
        bbox: Rect::new(x, y, w, size),
        font_size: size,
        ..Default::default()
    }
}

fn body(spans: Vec<TextSpan>) -> StructuredRegion {
    region_of(
        RegionRole::BodyBlock,
        &StructuredRegion {
            kind: RegionRole::BodyBlock,
            text: String::new(),
            bbox: spans[0].bbox,
            spans: Vec::new(),
            column_index: None,
            section_id: None,
        },
        spans,
    )
}

/// Page `i`: running head "Annual report 2025" at the top, folio "Page i+1" at the bottom.
fn page(i: usize, head: &str) -> StructuredPage {
    StructuredPage {
        page_index: i,
        page_width: 612.0,
        page_height: 792.0,
        regions: vec![body(vec![
            span(head, 72.0, 760.0, 150.0, 9.0),
            span(
                &format!("Body text of page {i} that is long"),
                72.0,
                700.0,
                400.0,
                11.0,
            ),
            span("and continues here.", 72.0, 686.0, 200.0, 11.0),
            span(&format!("Page {}", i + 1), 290.0, 30.0, 40.0, 9.0),
        ])],
    }
}

/// [`mark_running`] applied: `None` for an unchanged page.
fn marked_with(
    pages: &[StructuredPage],
    tables: &[&[DetectedTable]],
) -> Vec<Option<StructuredPage>> {
    let tables = (0..pages.len()).map(|i| tables.get(i).copied().unwrap_or(&[]));
    mark_running(pages.iter().zip(tables), 11.0)
        .iter()
        .zip(pages)
        .map(|(m, p)| (!m.is_empty()).then(|| move_out(p, m)))
        .collect()
}

/// [`mark_running`] of pages without tables.
fn run(pages: &[&StructuredPage]) -> Vec<Moves> {
    mark_running(pages.iter().map(|&p| (p, &[][..])), 11.0)
}

fn marked(pages: &[StructuredPage]) -> Vec<Option<StructuredPage>> {
    marked_with(pages, &[])
}

fn roles(p: &StructuredPage) -> Vec<String> {
    p.regions
        .iter()
        .map(|r| format!("{:?}:{}", r.kind, r.text))
        .collect()
}

#[test]
fn running_head_seen_on_two_pages_keeps_its_first_copy_and_folios_always_go() {
    let pages: Vec<StructuredPage> = (0..2).map(|i| page(i, "Annual report 2025")).collect();
    let out = marked(&pages);
    // SP8 ruling 1 (B3): the head's first occurrence stays body text, the page number is marked
    // from page 1
    assert_eq!(
        roles(out[0].as_ref().expect("page 1 changed")),
        [
            "BodyBlock:Annual report 2025 Body text of page 0 that is long and continues here.",
            "Footer:Page 1",
        ]
    );
    assert_eq!(
        roles(out[1].as_ref().expect("page 2 changed")),
        [
            "Header:Annual report 2025",
            "BodyBlock:Body text of page 1 that is long and continues here.",
            "Footer:Page 2",
        ]
    );
    // a head on MIN_SUPPORT pages or more is a running head on its first page too
    let pages: Vec<StructuredPage> = (0..4).map(|i| page(i, "Annual report 2025")).collect();
    for (i, p) in marked(&pages).iter().enumerate() {
        let r = roles(p.as_ref().expect("page changed"));
        assert_eq!(r[0], "Header:Annual report 2025", "page {i}: {r:?}");
        assert_eq!(r.last().unwrap(), &format!("Footer:Page {}", i + 1));
    }
}

#[test]
fn lone_page_number_on_a_single_page_stays_and_unique_heads_stay_body() {
    assert!(run(&[&page(0, "Head")])[0].is_empty());
    let pages: Vec<StructuredPage> = (0..3)
        .map(|i| page(i, &format!("Unique {i}x{i}")))
        .collect();
    let out = marked(&pages);
    // folios match; heads differ in two numbers at once and stay body
    for (i, p) in out.iter().enumerate() {
        let r = roles(p.as_ref().unwrap());
        assert!(
            r[0].starts_with(&format!("BodyBlock:Unique {i}x{i}")),
            "{r:?}"
        );
        assert_eq!(r.last().unwrap(), &format!("Footer:Page {}", i + 1));
    }
}

/// Page `i`: a top row `head` with `num` beside it (as in a book's running title), body text,
/// and a bottom row `foot` (none when empty).
fn titled(i: usize, head: &str, num: &str, foot: &str) -> StructuredPage {
    let mut spans = vec![
        span(head, 104.0, 760.0, 266.0, 10.0),
        span(num, 384.0, 759.0, 12.0, 11.0),
        span(
            &format!("Body text {} of the page", ["a", "b", "c"][i % 3]),
            72.0,
            700.0,
            400.0,
            11.0,
        ),
        span("and continues here.", 72.0, 686.0, 200.0, 11.0),
    ];
    if !foot.is_empty() {
        let (n, rest) = foot.split_once(' ').unwrap();
        spans.push(span(n, 72.0, 40.0, 8.0, 9.0));
        spans.push(span(rest, 90.0, 40.0, 300.0, 9.0));
    }
    StructuredPage {
        page_index: i,
        page_width: 612.0,
        page_height: 792.0,
        regions: vec![body(spans)],
    }
}

#[test]
fn numbered_running_title_is_marked_whole_from_the_first_page() {
    let pages: Vec<StructuredPage> = (0..3)
        .map(|i| titled(i, "Overcoming Barriers", &(9 + i).to_string(), ""))
        .collect();
    let out = marked(&pages);
    // the copies differ in the page number: a running line, not a title (single pages only split
    // the number off)
    for (i, p) in out.iter().enumerate() {
        let r = roles(p.as_ref().expect("page changed"));
        assert_eq!(
            r[0],
            format!("Header:Overcoming Barriers {}", 9 + i),
            "{r:?}"
        );
    }
}

#[test]
fn footnote_numbers_at_the_page_foot_are_not_page_numbers() {
    // a bare number beside a sentence, on a single page
    let p = titled(
        0,
        "Plain heading",
        "",
        "1 The source of this figure is the survey.",
    );
    assert!(run(&[&p])[0].is_empty());
    // footnotes 3, 7, 12 without a full stop: the numbers do not follow the pages
    let pages: Vec<StructuredPage> = [3, 7, 12]
        .iter()
        .enumerate()
        .map(|(i, n)| {
            titled(
                i,
                &format!("Unique {i}x{i}"),
                "",
                &format!("{n} Source of the data"),
            )
        })
        .collect();
    assert!(marked(&pages).iter().all(Option::is_none));
    // footnotes 1, 2, 3 ending a sentence on consecutive pages (numbered like pages)
    let notes = [
        "The data come from the survey.",
        "See the appendix for the method.",
        "Totals may not add up.",
    ];
    let pages: Vec<StructuredPage> = (0..3)
        .map(|i| {
            titled(
                i,
                &format!("Unique {i}x{i}"),
                "",
                &format!("{} {}", i + 1, notes[i]),
            )
        })
        .collect();
    assert!(marked(&pages).iter().all(Option::is_none));
}

#[test]
fn page_number_beside_the_running_title_is_marked_on_a_single_page() {
    // "Title   9" at the top (folio set larger), "18   Title" at the bottom, footnote mark tight
    let p = StructuredPage {
        page_index: 0,
        page_width: 612.0,
        page_height: 792.0,
        regions: vec![body(vec![
            span(
                "Overcoming Barriers to Representation",
                104.0,
                760.0,
                266.0,
                10.0,
            ),
            span("9", 384.0, 759.0, 6.0, 11.0),
            span(
                "Body text of the page that is long enough",
                72.0,
                700.0,
                400.0,
                11.0,
            ),
            span("and continues here.", 72.0, 686.0, 200.0, 11.0),
            span("18", 19.0, 32.0, 14.0, 14.0),
            span("Study on Plastics Use", 52.0, 33.0, 294.0, 10.0),
        ])],
    };
    let out = marked(std::slice::from_ref(&p));
    assert_eq!(
        roles(out[0].as_ref().expect("changed")),
        [
            "Header:9",
            "BodyBlock:Overcoming Barriers to Representation Body text of the page that is \
             long enough and continues here. Study on Plastics Use",
            "Footer:18",
        ]
    );
    // a smaller number (superscript mark) and a number tight against the text are not folios
    let mut q = p.clone();
    q.regions[0].spans[1].font_size = 6.0;
    q.regions[0].spans[4].bbox.x = 40.0;
    assert!(run(&[&q])[0].is_empty());
    // a list row tight under the text above, a row under a larger title, a row of ticks
    let mut q = p.clone();
    q.regions[0].spans[3].bbox.y = 48.0; // "and continues here." 2 pt above the bottom row
    q.regions[0]
        .spans
        .push(span("Large report title", 72.0, 775.0, 200.0, 16.0));
    assert!(run(&[&q])[0].is_empty());
    let mut q = p.clone();
    q.regions[0].spans[0].text = "10   20".into();
    q.regions[0].spans.truncate(4);
    assert!(run(&[&q])[0].is_empty());
}

#[test]
fn page_number_masks() {
    for t in ["12", "Page 3 of 9", "стр. 4", "3/9", "- 7 -"] {
        let m = digit_mask(&norm(t)).0;
        assert_eq!(is_folio(&m), t != "- 7 -", "{t}");
    }
    assert!(!is_folio(&digit_mask(&norm("Annual report 2025")).0));
    assert!(!is_folio(&digit_mask(&norm("Page")).0));
}

#[test]
fn repeated_table_header_row_is_not_a_running_head() {
    // three cells on one row, 4 pt above the first body row on every page
    let mk = |i: usize| StructuredPage {
        page_index: i,
        page_width: 612.0,
        page_height: 792.0,
        regions: vec![
            body(vec![
                span("Наименование", 72.0, 760.0, 80.0, 9.0),
                span("Количество", 250.0, 760.0, 70.0, 9.0),
                span("Сумма", 420.0, 760.0, 40.0, 9.0),
            ]),
            body(vec![span(&format!("Строка {i}"), 72.0, 747.0, 80.0, 9.0)]),
        ],
    };
    let pages: Vec<StructuredPage> = (0..4).map(mk).collect();
    let out = marked(&pages);
    assert!(
        out.iter().all(Option::is_none),
        "{:?}",
        out.iter().flatten().map(roles).collect::<Vec<_>>()
    );
}

#[test]
fn cells_inside_a_table_are_never_running_heads() {
    let words = ["one", "two", "three", "four"];
    let pages: Vec<StructuredPage> = (0..4)
        .map(|i| {
            let mut p = page(i, "Annual report 2025");
            p.regions[0].spans[1].text = format!("Body text {} that is long", words[i]);
            p
        })
        .collect();
    // a table at the top edge holds the repeated line on every page
    let t = DetectedTable::new(
        BBox::new(60.0, 740.0, 500.0, 40.0),
        plaindoc_core::Block::paragraph(String::new(), None),
    );
    let tables: Vec<&[DetectedTable]> = vec![std::slice::from_ref(&t); 4];
    let out = marked_with(&pages, &tables);
    for p in &out {
        let r = roles(p.as_ref().expect("folio still marked"));
        assert!(r[0].starts_with("BodyBlock:Annual report 2025"), "{r:?}");
        assert!(r.last().unwrap().starts_with("Footer:Page"), "{r:?}");
    }
}

#[test]
fn numbering_needs_the_page_step() {
    let c = |t: &str| Cand {
        spans: vec![],
        text: t.into(),
        size: 9.0,
        bbox: Rect::new(0.0, 0.0, 50.0, 9.0),
        chunks: 1,
        key: Key::of(t),
    };
    assert!(similar(&c("Page 3 of 9"), &c("Page 4 of 9"), 1));
    assert!(similar(&c("12"), &c("14"), 2));
    assert!(!similar(&c("12"), &c("14"), 1));
    assert!(!similar(&c("Page 3 of 9"), &c("Page 4 of 10"), 1));
    assert!(!similar(&c("Глава 1"), &c("Раздел 2"), 1));
}

#[test]
fn non_finite_and_empty_pages_do_not_panic() {
    let mut p = page(0, "Head");
    p.regions[0].spans[0].bbox = Rect::new(f32::NAN, f32::NAN, f32::INFINITY, f32::NAN);
    let empty = StructuredPage {
        page_index: 1,
        page_width: 0.0,
        page_height: 0.0,
        regions: vec![],
    };
    let _ = run(&[&p, &empty, &p]);
    assert!(run(&[]).is_empty());
}

/// Page `i` of a 12-page deck: header `head` at the top, two body lines, and the footer
/// "Prepared 01.02.2026, rev. 03.02.2026 N / 12" (N = i + 1).
fn deck(i: usize, head: &str) -> StructuredPage {
    let mut p = page(i, head);
    p.regions[0].spans[1].text = format!("Item {} body text that is long", ["a", "b", "c"][i % 3]);
    p.regions[0].spans[3] = span(
        &format!("Prepared 01.02.2026, rev. 03.02.2026 {} / 12", i + 1),
        72.0,
        30.0,
        400.0,
        9.0,
    );
    p
}

/// Variant A of the head on pages 1 and 10–12, variant B on pages 2–9.
fn deck_head(i: usize) -> &'static str {
    if i == 0 || i >= 9 {
        "Acme / Quarterly review: Part one"
    } else {
        "Acme / Quarterly review: Notes"
    }
}

#[test]
fn numbered_footer_and_head_variants_are_marked_on_every_page() {
    let pages: Vec<StructuredPage> = (0..12).map(|i| deck(i, deck_head(i))).collect();
    let out = marked(&pages);
    for (i, p) in out.iter().enumerate() {
        let r = roles(p.as_ref().expect("page changed"));
        assert_eq!(r[0], format!("Header:{}", deck_head(i)), "page {i}: {r:?}");
        assert!(r[1].starts_with("BodyBlock:Item"), "page {i}: {r:?}");
        assert_eq!(
            r.last().unwrap(),
            &format!("Footer:Prepared 01.02.2026, rev. 03.02.2026 {} / 12", i + 1),
            "page {i}"
        );
    }
}

#[test]
fn a_line_on_two_distant_pages_and_a_different_first_title_stay_body() {
    // a body line in the top band on pages 4 and 8 only; page 1 starts with its own title
    let pages: Vec<StructuredPage> = (0..12)
        .map(|i| {
            let head = match i {
                0 => "Welcome to the review",
                3 | 7 => "Remarks on the method",
                _ => "Unique head of page",
            };
            let mut p = deck(i, head);
            if !matches!(i, 0 | 3 | 7) {
                p.regions[0].spans[0].text = format!("Unique head {i}x{i}");
            }
            p
        })
        .collect();
    let out = marked(&pages);
    for (i, p) in out.iter().enumerate() {
        let r = roles(p.as_ref().expect("footer marked"));
        assert!(r[0].starts_with("BodyBlock:"), "page {i}: {r:?}");
        assert!(r.last().unwrap().starts_with("Footer:Prepared"), "page {i}");
    }
    // the title of page 1 differs from the running head of the other pages
    let pages: Vec<StructuredPage> = (0..12)
        .map(|i| {
            deck(
                i,
                if i == 0 {
                    "Welcome to the review"
                } else {
                    deck_head(1)
                },
            )
        })
        .collect();
    let out = marked(&pages);
    let r = roles(out[0].as_ref().unwrap());
    assert!(r[0].starts_with("BodyBlock:Welcome to the review"), "{r:?}");
    assert_eq!(
        roles(out[1].as_ref().unwrap())[0],
        format!("Header:{}", deck_head(1))
    );
}

#[test]
fn moved_regions_keep_tight_punctuation() {
    // the bracket and the comma are spans of their own (another font) touching their words
    let spans = vec![
        span("Bank", 100.0, 800.0, 30.0, 10.0),
        span("(", 132.5, 800.0, 3.0, 10.0),
        span("licence", 135.5, 800.0, 30.0, 10.0),
        span(")", 165.5, 800.0, 3.0, 10.0),
        span(",", 168.5, 800.0, 2.0, 10.0),
        span("body", 100.0, 700.0, 30.0, 10.0),
        span("(", 132.5, 700.0, 3.0, 10.0),
        span("text", 135.5, 700.0, 20.0, 10.0),
        span(")", 155.5, 700.0, 3.0, 10.0),
    ];
    let p = StructuredPage {
        page_index: 0,
        page_width: 595.0,
        page_height: 842.0,
        regions: vec![body(spans)],
    };
    let head: Vec<(usize, usize)> = (0..5).map(|s| (0, s)).collect();
    let out = move_out(&p, &[(head, RegionRole::Header)]);
    let texts: Vec<&str> = out.regions.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, ["Bank (licence),", "body (text)"]);
}
