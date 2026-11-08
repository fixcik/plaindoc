use super::*;

fn bx(x0: f32, y0: f32, x1: f32, y1: f32) -> OrderBox {
    OrderBox {
        x0,
        y0,
        x1,
        y1,
        font_size: 10.0,
    }
}

#[test]
fn title_then_left_column_then_right_then_footer() {
    let blocks = [
        bx(72.0, 700.0, 540.0, 730.0),  // 0 title, full width
        bx(320.0, 500.0, 540.0, 690.0), // 1 right column, top paragraph
        bx(72.0, 500.0, 290.0, 690.0),  // 2 left column, top paragraph
        bx(72.0, 300.0, 290.0, 488.0),  // 3 left column, second paragraph (12 pt gap <= 2 fs)
        bx(320.0, 300.0, 540.0, 488.0), // 4 right column, second paragraph
        bx(72.0, 100.0, 540.0, 200.0),  // 5 full-width block below
    ];
    assert_eq!(xy_cut(&blocks).unwrap(), [0, 2, 3, 1, 4, 5]);
}

#[test]
fn single_column_keeps_top_down_order() {
    let blocks: Vec<OrderBox> = (0..5)
        .map(|i| {
            bx(
                72.0,
                700.0 - 40.0 * i as f32,
                540.0,
                730.0 - 40.0 * i as f32,
            )
        })
        .collect();
    assert_eq!(xy_cut(&blocks).unwrap(), [0, 1, 2, 3, 4]);
}

#[test]
fn narrow_page_number_in_gutter_does_not_hide_columns() {
    let blocks = [
        bx(72.0, 400.0, 290.0, 700.0),
        bx(320.0, 400.0, 540.0, 700.0),
        bx(300.0, 380.0, 310.0, 720.0), // narrow marginal item crossing the gutter
    ];
    let o = xy_cut(&blocks).unwrap();
    assert!(
        o.iter().position(|&i| i == 0) < o.iter().position(|&i| i == 1),
        "{o:?}"
    );
}

#[test]
fn limits_and_bad_boxes_fall_back() {
    assert_eq!(xy_cut(&vec![bx(0.0, 0.0, 1.0, 1.0); MAX_BLOCKS + 1]), None);
    assert_eq!(xy_cut(&[bx(f32::NAN, 0.0, 1.0, 1.0)]), None);
    assert_eq!(xy_cut(&[]), Some(vec![]));
}

#[test]
fn adversarial_row_gaps_are_complete() {
    let complete = |b: &[OrderBox]| {
        let mut o = xy_cut(b).unwrap();
        o.sort_unstable();
        assert_eq!(o, (0..b.len()).collect::<Vec<_>>());
    };
    // two and four columns of 6 pt row gaps (<= 2 fs): every row gap is checked and rejected
    for cols in [2usize, 4] {
        let w = 500.0 / cols as f32;
        let b: Vec<OrderBox> = (0..MAX_BLOCKS)
            .map(|i| {
                let (r, c) = ((i / cols) as f32, (i % cols) as f32);
                let y = 20_000.0 - 16.0 * r;
                bx(72.0 + w * c, y, 72.0 + w * c + w - 20.0, y + 10.0)
            })
            .collect();
        complete(&b);
    }
    // one bottom edge, 2000 narrow blocks with small x gaps
    let b: Vec<OrderBox> = (0..MAX_BLOCKS)
        .map(|i| {
            let f = i as f32;
            bx(6.0 * f, 0.0, 6.0 * f + 0.5, 10.0 + (i % 7) as f32)
        })
        .collect();
    complete(&b);
    // crossing stack: 20 narrow blocks over the gutter at the top and at the bottom, 5 pt
    // apart (no gutter cut), 980 rows of two column paragraphs with 30 pt row gaps (> 2 fs,
    // columns above and below: every row gap is checked and rejected)
    let (mut b, mut y) = (Vec::new(), 1e6f32);
    let narrow = |b: &mut Vec<OrderBox>, y: &mut f32| {
        for _ in 0..20 {
            b.push(bx(250.0, *y - 10.0, 350.0, *y));
            *y -= 15.0;
        }
    };
    narrow(&mut b, &mut y);
    for _ in 0..980 {
        b.push(bx(72.0, y - 70.0, 290.0, y));
        b.push(bx(320.0, y - 70.0, 540.0, y));
        y -= 100.0;
    }
    y += 25.0;
    narrow(&mut b, &mut y);
    assert_eq!(b.len(), MAX_BLOCKS);
    let t = std::time::Instant::now();
    complete(&b);
    eprintln!("xy_cut crossing stack, 2000 blocks: {:?}", t.elapsed());
}

#[test]
fn gutter_is_cut_before_a_wider_row_gap() {
    // a figure caption at the top of the right column, 56 pt above the right text and above
    // the left column's caption (ODL rule "the larger gap wins" reads it first)
    let blocks = [
        bx(311.0, 419.0, 535.0, 477.0), // 0 right caption
        bx(305.0, 185.0, 539.0, 363.0), // 1 right text
        bx(63.0, 299.0, 284.0, 317.0),  // 2 left caption
        bx(57.0, 71.0, 291.0, 189.0),   // 3 left text
    ];
    assert_eq!(xy_cut(&blocks).unwrap(), [2, 3, 0, 1]);
}

#[test]
fn row_gap_through_both_columns_is_not_a_cut() {
    // paragraph breaks of both columns line up (40 pt > 2 fs); a table at the foot crosses
    // the gutter, so the page has no gutter cut: the gap must not read the columns by rows
    let blocks = [
        bx(72.0, 520.0, 290.0, 700.0),  // 0 left, top
        bx(72.0, 300.0, 290.0, 480.0),  // 1 left, bottom
        bx(320.0, 520.0, 540.0, 700.0), // 2 right, top
        bx(320.0, 300.0, 540.0, 480.0), // 3 right, bottom
        bx(72.0, 280.0, 540.0, 298.0),  // 4 table, full width, 2 pt below
    ];
    assert_eq!(xy_cut(&blocks).unwrap(), [0, 1, 2, 3, 4]);
}

#[test]
fn node_without_a_cut_keeps_the_input_order() {
    // overlapping in both axes: no gap; the lower block came first and stays first
    let blocks = [bx(0.0, 0.0, 100.0, 50.0), bx(50.0, 25.0, 150.0, 75.0)];
    assert_eq!(xy_cut(&blocks).unwrap(), [0, 1]);
}

#[test]
fn blocks_are_reordered_and_running_heads_keep_their_slots() {
    let para =
        |t: &str, x: f32, y: f32, w: f32| Block::paragraph(t, Some(BBox::new(x, y, w, 100.0)));
    let note = |kind, t: &str| Block::Note {
        kind,
        text: t.into(),
        bbox: Some(BBox::new(72.0, 760.0, 400.0, 10.0)),
    };
    let mut blocks = vec![
        note(NoteKind::Header, "head"),
        para("right", 320.0, 500.0, 220.0),
        para("title", 72.0, 650.0, 468.0),
        Block::paragraph("no box", None),
        para("left", 72.0, 500.0, 220.0),
        note(NoteKind::PageNumber, "7"),
    ];
    assert!(order_blocks(&mut blocks, 10.0));
    let texts: Vec<&str> = blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph { text, .. } | Block::Note { text, .. } => text.as_str(),
            _ => "",
        })
        .collect();
    assert_eq!(texts, ["head", "title", "left", "no box", "right", "7"]);
    let mut many = vec![para("p", 0.0, 0.0, 1.0); MAX_BLOCKS + 1];
    assert!(!order_blocks(&mut many, 10.0), "over the limit: order kept");
    assert_eq!(many.len(), MAX_BLOCKS + 1);
}

/// A gutter crosser over two column blocks at the top (no gutter cut), then 1997 one-pt blocks
/// alternating left and right: gaps 5 pt within `NEAR_FS` font sizes of the top, 6 pt below.
/// Each node rejects the 6 pt gaps and peels one block at the bottom, down to `MAX_DEPTH`.
/// `shuffle`: the same blocks in a scrambled input order.
fn peel(fs: f32, shuffle: bool) -> Vec<OrderBox> {
    let b = |x0, y0, x1, y1| OrderBox {
        x0,
        y0,
        x1,
        y1,
        font_size: fs,
    };
    let mut v = vec![
        b(250.0, 0.0, 350.0, 100.0),
        b(72.0, 0.0, 290.0, 100.0),
        b(320.0, 0.0, 540.0, 100.0),
    ];
    let (mut y, mut k) = (0.0f32, 0);
    while v.len() < MAX_BLOCKS {
        y -= if -y < NEAR_FS * fs + 10.0 { 5.0 } else { 6.0 };
        let (x0, x1) = if k % 2 == 0 {
            (72.0, 290.0)
        } else {
            (320.0, 540.0)
        };
        v.push(b(x0, y - 1.0, x1, y));
        y -= 1.0;
        k += 1;
    }
    if shuffle {
        let mut r = 0x9E37_79B9_7F4A_7C15u64;
        for i in (1..v.len()).rev() {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            v.swap(i, (r % (i as u64 + 1)) as usize);
        }
    }
    v
}

#[test]
fn peeled_stack_runs_into_the_work_limit() {
    // fs 10: 1.65 M visits, under the limit, ordered; fs 100 and 1000: 13 M and 55 M visits
    // (60-570 ms without the limit): the input order is kept
    assert!(xy_cut(&peel(10.0, false)).is_some());
    for fs in [100.0, 1000.0] {
        for shuffle in [false, true] {
            let blocks = peel(fs, shuffle);
            let t = std::time::Instant::now();
            assert_eq!(xy_cut(&blocks), None, "fs {fs} shuffled {shuffle}");
            eprintln!(
                "xy_cut peeled stack fs {fs} shuffled {shuffle}: {:?}",
                t.elapsed()
            );
        }
    }
    let mut blocks: Vec<Block> = peel(1000.0, true)
        .iter()
        .map(|o| Block::paragraph("p", Some(BBox::new(o.x0, o.y0, o.x1 - o.x0, o.y1 - o.y0))))
        .collect();
    assert!(
        !order_blocks(&mut blocks, 1000.0),
        "order kept, caller warns"
    );
}

#[test]
fn deep_staircase_is_bounded_and_complete() {
    // every block offset right and down: each level peels one block
    let blocks: Vec<OrderBox> = (0..MAX_BLOCKS)
        .map(|i| {
            let f = i as f32;
            bx(
                10.0 * f,
                10_000.0 - 10.0 * f,
                10.0 * f + 4.0,
                10_004.0 - 10.0 * f,
            )
        })
        .collect();
    let mut o = xy_cut(&blocks).unwrap();
    o.sort_unstable();
    assert_eq!(o, (0..MAX_BLOCKS).collect::<Vec<_>>());
}
