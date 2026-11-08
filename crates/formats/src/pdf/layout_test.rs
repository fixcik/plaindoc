use super::*;
use pdf_oxide::StructuredRegion;

fn span(text: &str, x: f32, y: f32, w: f32, h: f32, size: f32) -> TextSpan {
    TextSpan {
        text: text.into(),
        bbox: Rect::new(x, y, w, h),
        font_size: size,
        ..Default::default()
    }
}

fn region(kind: RegionRole, spans: Vec<TextSpan>) -> StructuredRegion {
    let bbox = spans
        .iter()
        .skip(1)
        .fold(spans[0].bbox, |acc, s| acc.union(&s.bbox));
    let text = spans
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    StructuredRegion {
        kind,
        text,
        bbox,
        spans,
        column_index: None,
        section_id: None,
    }
}

fn page(regions: Vec<StructuredRegion>) -> StructuredPage {
    StructuredPage {
        page_index: 0,
        page_width: 612.0,
        page_height: 792.0,
        regions,
    }
}

#[test]
fn heading_lines_are_split_out_of_body_region() {
    let r = region(
        RegionRole::BodyBlock,
        vec![
            span("Big", 72.0, 720.0, 30.0, 24.0, 24.0),
            span("Title", 106.0, 720.0, 50.0, 24.0, 24.0),
            span(
                "Body text here, a normal paragraph.",
                72.0,
                690.0,
                300.0,
                12.0,
                12.0,
            ),
            span("Second line of it.", 72.0, 676.0, 150.0, 12.0, 12.0),
        ],
    );
    let pb = page_to_blocks(&page(vec![r]), 12.0, &LayoutConfig::default());
    assert_eq!(pb.blocks.len(), 2);
    assert!(matches!(&pb.blocks[0], Block::Heading { level: 1, text, .. } if text == "Big Title"));
    assert!(
        matches!(&pb.blocks[1], Block::Paragraph { text, .. } if text == "Body text here, a normal paragraph. Second line of it.")
    );
}

#[test]
fn heading_levels_by_ratio() {
    let cfg = LayoutConfig::default();
    // body 9.0: ratios 2.33 / 1.75 / 1.33 / 1.22. Since SP8 a heading needs a score >= 0.9:
    // 12 and 11 pt regular lines fall inside the ODL mode window [10, 13] and get no rank boost.
    for (size, level) in [
        (21.0, Some(1u8)),
        (15.75, Some(2)),
        (12.0, None),
        (11.0, None),
    ] {
        let r = region(
            RegionRole::BodyBlock,
            vec![
                span("Head", 72.0, 700.0, 40.0, size, size),
                span("body", 72.0, 600.0, 40.0, 9.0, 9.0),
            ],
        );
        let pb = page_to_blocks(&page(vec![r]), 9.0, &cfg);
        let got = match &pb.blocks[0] {
            Block::Heading { level, .. } => Some(*level),
            _ => None,
        };
        assert_eq!(got, level, "size {size}");
    }
}

#[test]
fn non_finite_sizes_are_never_headings() {
    let cfg = LayoutConfig::default();
    for (size, body) in [(f32::NAN, 9.0), (f32::INFINITY, 9.0), (f32::NAN, f32::NAN)] {
        let r = region(
            RegionRole::BodyBlock,
            vec![
                span("Head", 72.0, 700.0, 40.0, 12.0, size),
                span("body", 72.0, 600.0, 40.0, 9.0, 9.0),
            ],
        );
        let pb = page_to_blocks(&page(vec![r]), body, &cfg);
        assert!(
            pb.blocks
                .iter()
                .all(|b| matches!(b, Block::Paragraph { .. })),
            "size {size} body {body}"
        );
    }
}

#[test]
fn region_without_heading_keeps_engine_text() {
    let mut r = region(
        RegionRole::BodyBlock,
        vec![
            span("Г", 33.0, 702.0, 15.0, 21.0, 10.5),
            span("одовой", 46.0, 702.0, 60.0, 21.0, 10.5),
        ],
    );
    r.text = "Г одовой".into();
    let pb = page_to_blocks(&page(vec![r]), 10.5, &LayoutConfig::default());
    assert!(matches!(&pb.blocks[0], Block::Paragraph { text, .. } if text == "Г одовой"));
}

#[test]
fn adjacent_spans_on_one_line_join_without_space() {
    let r = region(
        RegionRole::BodyBlock,
        vec![
            span("Г", 33.0, 702.0, 14.0, 21.0, 21.0),
            span("одовой отчёт", 46.5, 702.0, 120.0, 21.0, 21.0),
            span("текст", 33.0, 650.0, 40.0, 10.5, 10.5),
        ],
    );
    let pb = page_to_blocks(&page(vec![r]), 10.5, &LayoutConfig::default());
    assert!(
        matches!(&pb.blocks[0], Block::Heading { level: 1, text, .. } if text == "Годовой отчёт")
    );
}

#[test]
fn roles_map_to_notes_and_only_artifacts_drop() {
    let regions = vec![
        region(
            RegionRole::Header,
            vec![span("Report", 24.0, 769.5, 100.0, 8.0, 8.0)],
        ),
        region(
            RegionRole::Footer,
            vec![span("file:///x", 24.0, 17.2, 100.0, 8.0, 8.0)],
        ),
        region(
            RegionRole::PageNumber,
            vec![span("7", 300.0, 20.0, 6.0, 8.0, 8.0)],
        ),
        region(
            RegionRole::MarginalLabel,
            vec![span("314", 62.0, 623.0, 12.0, 11.0, 11.0)],
        ),
        region(
            RegionRole::Artifact,
            vec![span("1/3", 577.0, 17.2, 10.0, 8.0, 8.0)],
        ),
        region(
            RegionRole::StructuralHeading { level: 9 },
            vec![span("Tagged", 72.0, 700.0, 50.0, 12.0, 12.0)],
        ),
    ];
    let pb = page_to_blocks(&page(regions), 12.0, &LayoutConfig::default());
    let kinds: Vec<_> = pb
        .blocks
        .iter()
        .map(|b| match b {
            Block::Note { kind, text, .. } => format!("{kind:?}:{text}"),
            Block::Heading { level, .. } => format!("H{level}"),
            _ => "P".into(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "Header:Report",
            "Footer:file:///x",
            "PageNumber:7",
            "Marginal:314",
            "H6"
        ]
    );
    assert_eq!((pb.dropped_regions, pb.dropped_chars), (1, 3));
}

#[test]
fn bbox_keeps_pdf_origin() {
    let r = region(
        RegionRole::Header,
        vec![span("H", 24.0, 769.5, 10.0, 8.0, 8.0)],
    );
    let pb = page_to_blocks(&page(vec![r]), 12.0, &LayoutConfig::default());
    let Block::Note { bbox: Some(b), .. } = &pb.blocks[0] else {
        panic!()
    };
    assert_eq!((b.x, b.y, b.w, b.h), (24.0, 769.5, 10.0, 8.0));
}

#[test]
fn body_size_is_char_weighted_mode_smaller_on_tie() {
    let a = page(vec![region(
        RegionRole::BodyBlock,
        vec![
            span("Title", 0.0, 700.0, 50.0, 24.0, 24.0),
            span("body text body", 0.0, 680.0, 90.0, 12.0, 12.0),
        ],
    )]);
    assert_eq!(body_font_size(&[&a]), 12.0);
    let tie = page(vec![region(
        RegionRole::BodyBlock,
        vec![
            span("abcd", 0.0, 700.0, 40.0, 11.0, 11.0),
            span("efgh", 0.0, 680.0, 40.0, 10.0, 10.0),
        ],
    )]);
    assert_eq!(body_font_size(&[&tie]), 10.0);
    assert_eq!(body_font_size(&[]), 0.0);
}

#[test]
fn table_between_needs_vertical_gap_and_x_overlap() {
    let para = |text: &str, x: f32, y: f32| {
        paragraphs(&styled_lines(&[span(text, x, y, 100.0, 11.0, 11.0)])).remove(0)
    };
    let table = |x: f32, y: f32, w: f32, h: f32| {
        let b = BBox::new(x, y, w, h);
        DetectedTable::new(b, Block::paragraph(String::new(), None))
    };
    let (above, below) = (para("Above", 72.0, 700.0), para("Below", 72.0, 500.0));
    // between the paragraphs, over them in x
    assert!(table_between(
        &[table(60.0, 550.0, 200.0, 100.0)],
        &above,
        &below
    ));
    // overlapping the paragraph below
    assert!(!table_between(
        &[table(60.0, 480.0, 200.0, 100.0)],
        &above,
        &below
    ));
    // no x overlap
    assert!(!table_between(
        &[table(400.0, 550.0, 100.0, 100.0)],
        &above,
        &below
    ));
    // no table
    assert!(!table_between(&[], &above, &below));
}

#[test]
fn region_is_cut_into_slices_of_engine_text() {
    let mut r = region(
        RegionRole::BodyBlock,
        vec![
            span("Г", 72.0, 700.0, 6.0, 10.0, 10.0),
            span("лава one runs to the edge", 78.0, 700.0, 200.0, 10.0, 10.0),
            span("and ends here.", 72.0, 688.0, 90.0, 10.0, 10.0),
            span(
                "Next one starts at the edge",
                72.0,
                668.0,
                206.0,
                10.0,
                10.0,
            ),
        ],
    );
    r.text = "Г лава one runs to the edge\nand ends here. Next one starts at the edge".into();
    let pb = page_to_blocks(&page(vec![r.clone()]), 10.0, &LayoutConfig::default());
    let texts: Vec<&str> = pb
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph { text, .. } => text.as_str(),
            _ => "",
        })
        .collect();
    assert_eq!(
        texts,
        [
            "Г лава one runs to the edge\nand ends here.",
            "Next one starts at the edge"
        ]
    );
    assert_eq!(chars_of(&blocks_text(&pb.blocks)), chars_of(&r.text));
}

#[test]
fn split_preserves_characters() {
    let r = region(
        RegionRole::BodyBlock,
        vec![
            span("Big", 72.0, 720.0, 30.0, 24.0, 24.0),
            span("Title", 106.0, 720.0, 50.0, 24.0, 24.0),
            span("Body, text.", 72.0, 690.0, 80.0, 12.0, 12.0),
        ],
    );
    let expected = chars_of(&r.text);
    let pb = page_to_blocks(&page(vec![r]), 12.0, &LayoutConfig::default());
    assert_eq!(chars_of(&blocks_text(&pb.blocks)), expected);
}
