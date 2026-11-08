#![cfg(feature = "rtf")]
mod pictures_util;
use plaindoc_core::*;
use plaindoc_formats::Registry;
use plaindoc_formats::rtf::RtfParser;

fn run(b: &[u8]) -> Result<Document> {
    RtfParser.parse(
        Input::Bytes(b.to_vec()),
        Format::Rtf,
        &ParseOptions::default(),
    )
}
fn doc(s: &str) -> Document {
    run(s.as_bytes()).unwrap()
}
fn blocks(s: &str) -> Vec<Block> {
    doc(s).sections.remove(0).blocks
}
fn p(t: &str) -> Block {
    Block::paragraph(t, None)
}
fn kinds(d: &Document) -> Vec<(WarningKind, Level)> {
    d.warnings
        .iter()
        .chain(d.sections.iter().flat_map(|s| &s.warnings))
        .map(|w| (w.kind, w.level))
        .collect()
}

#[test]
fn paragraphs_headings_and_special_characters() {
    let b = blocks(
        r"{\rtf1\ansi\deff0{\fonttbl{\f0 Times;}}\pard\outlinelevel0 Title\par\pard Hello {\b bold} world\line next\tab x\emdash y\par\pard \'93q\'94 \{a\}\par}",
    );
    assert_eq!(
        b,
        vec![
            Block::heading(1, "Title", None),
            p("Hello bold world\nnext\tx\u{2014}y"),
            p("\u{201c}q\u{201d} {a}")
        ]
    );
}

#[test]
fn ansicpg_1251() {
    assert_eq!(
        blocks(r"{\rtf1\ansi\ansicpg1251 \'cf\'f0\'e8\'e2\'e5\'f2\par}"),
        vec![p("Привет")]
    );
}

#[test]
fn font_charset_overrides_the_document_code_page() {
    let b = blocks(
        r"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fswiss\fcharset0 Arial;}{\f1\froman\fcharset204 Times New Roman CYR;}}\pard\f1 \'cf\'f0\'e8\'e2\'e5\'f2 \f0 caf\'e9\par}",
    );
    assert_eq!(b, vec![p("Привет café")]);
}

#[test]
fn unicode_escapes_skip_their_fallback() {
    // `@` is a backslash (the escapes stay out of the Rust source); `\uc2` is still
    // in effect after the surrogate pair, so it also skips the space before "end".
    let b = blocks(
        &r"{\rtf1\ansi\uc1 @u1055?@u1088?\uc2@u1080??\u-10179?\u-8704? end\uc0@u1046 x\par}"
            .replace('@', "\\"),
    );
    assert_eq!(b, vec![p("При\u{1F600}endЖx")]);
}

#[test]
fn non_text_destinations_are_skipped_and_pictures_become_images() {
    let mut src = br#"{\rtf1{\fonttbl{\f0 Arial;}}{\colortbl;\red0\green0\blue0;}{\stylesheet{\s0 Normal;}}{\*\generator Msftedit;}\pard a {\field{\*\fldinst{HYPERLINK "http://x"}}{\fldrslt{link}}} b{\*\shppict{\pict\pngblip 89504e47}}{\nonshppict{\pict\wmetafile8 0102}}{\pict\bin4 "#.to_vec();
    src.extend([0u8, b'{', b'}', 1]);
    src.extend(br"}\par}");
    let img = |i: ImageInfo| Block::Image {
        alt: None,
        ocr_text: None,
        bbox: None,
        image: Some(Box::new(i)),
    };
    // the hex data is taken; `\bin` data (skipped by the lexer) is not; `\nonshppict` is a copy
    assert_eq!(
        run(&src).unwrap().sections[0].blocks,
        vec![
            p("a link b"),
            img(ImageInfo::embedded(&b"\x89PNG"[..], None)),
            img(ImageInfo::skipped("no_data", None))
        ]
    );
    let mut off = ParseOptions::default();
    off.images.enabled = false;
    let bare = Block::image(None, None, None);
    assert_eq!(
        RtfParser
            .parse(Input::Bytes(src), Format::Rtf, &off)
            .unwrap()
            .sections[0]
            .blocks,
        vec![p("a link b"), bare.clone(), bare]
    );
}

#[test]
fn rtf_pictures_are_stored_and_capped() {
    let hex: String = pictures_util::RED
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let src = format!(
        "{{\\rtf1 a{{\\*\\shppict{{\\pict{{\\*\\picprop{{\\sp{{\\sn x}}{{\\sv 1}}}}}}\\pngblip\r\n{}\r\n{}}}}}{{\\nonshppict{{\\pict\\wmetafile8 0102}}}}\\par}}",
        &hex[..20],
        &hex[20..]
    );
    let d = Registry::with_defaults()
        .parse(
            Input::Bytes(src.clone().into_bytes()),
            &ParseOptions::default(),
        )
        .unwrap();
    assert_eq!(pictures_util::files(&d), [Some("image1.png".to_string())]);
    assert_eq!(pictures_util::stored(&d, "image1.png"), pictures_util::RED);
    assert_eq!(d.to_markdown(), "a\n\n![изображение 1](image1.png)\n");
    let mut o = ParseOptions::default();
    o.images.max_bytes = 10;
    let d = Registry::with_defaults()
        .parse(Input::Bytes(src.into_bytes()), &o)
        .unwrap();
    assert!(d.images.is_empty());
    assert!(d.to_markdown().contains("<!-- image -->"));
}

#[test]
fn lists_nest_by_ilvl_and_take_ordered_from_the_label() {
    let d = doc(
        "{\\rtf1\\ansi\n{\\listtext\\pard\\plain 1.\\tab}\\pard\\ls1\\ilvl0 one\\par\n{\\listtext\\pard\\plain \\'b7\\tab}\\pard\\ls1\\ilvl1 sub\\par\n{\\listtext\\pard\\plain 2.\\tab}\\pard\\ls1\\ilvl0 two\\par\n\\pard after\\par}",
    );
    assert_eq!(d.to_markdown(), "1. one\n   - sub\n2. two\n\nafter\n");
    let b = blocks(
        r"{\rtf1\ansi{\listtext 1.\tab}\pard\ls1 a{\pict\pngblip 89}\par{\listtext 2.\tab}\pard\ls1 b\par\pard after\par}",
    );
    let Block::List { items, .. } = &b[0] else {
        panic!("{b:?}")
    };
    assert_eq!(items.len(), 2);
    assert!(matches!(
        items[0].blocks[..],
        [Block::Paragraph { .. }, Block::Image { .. }]
    ));
    assert_eq!(b[1], p("after"));
    assert_eq!(b.len(), 2);
    let b = blocks(
        r"{\rtf1\ansi{\listtext 1.\tab}\pard\ls1 a{\footnote n}\par\pard after\par\pard more\par}",
    );
    assert!(matches!(b[0], Block::List { .. }), "{b:?}");
    assert!(matches!(b[1], Block::Note { .. }), "{b:?}");
    assert_eq!(b[2], p("after"));
    let b = blocks(
        r"{\rtf1\ansi{\listtext 1.\tab}\pard\ls1 a{\footnote n}\par\trowd\cellx1000\pard\intbl c\cell\row\pard after\par}",
    );
    assert!(matches!(b[0], Block::List { .. }), "{b:?}");
    assert!(matches!(b[1], Block::Note { .. }), "{b:?}");
    assert!(matches!(b[2], Block::Table { .. }), "{b:?}");
    assert_eq!(b[3], p("after"));
    // R12: picture-only paragraph after a list, picture between items, note in a cell,
    // note in an empty list paragraph
    let b = blocks(
        r"{\rtf1\ansi{\listtext 1.\tab}\pard\ls1 a\par\pard{\pict\pngblip 89}\par\pard after\par}",
    );
    assert!(matches!(b[0], Block::List { .. }), "{b:?}");
    assert!(matches!(b[1], Block::Image { .. }), "{b:?}");
    assert_eq!(b[2], p("after"));

    let b = blocks(
        r"{\rtf1\ansi{\listtext 1.\tab}\pard\ls1 a\par{\pict\pngblip 89}\par{\listtext 2.\tab}\pard\ls1 b\par}",
    );
    assert!(
        matches!(&b[0], Block::List { items, .. } if items.len() == 2 && matches!(items[0].blocks[..], [_, Block::Image { .. }]) && items[1].blocks.len() == 1),
        "{b:?}"
    );
    let b =
        blocks(r"{\rtf1\ansi\trowd\cellx1000\pard\intbl c{\footnote n}\cell\row\pard after\par}");
    assert!(matches!(b[1], Block::Note { .. }), "{b:?}");
    assert_eq!(b[2], p("after"));
    // a note between the last \cell and \row comes right after the table
    let b = blocks(
        r"{\rtf1\ansi\trowd\cellx1000\pard\intbl c\cell{\footnote n}\row\pard after\par\pard next\par}",
    );
    assert!(matches!(b[0], Block::Table { .. }), "{b:?}");
    assert!(matches!(b[1], Block::Note { .. }), "{b:?}");
    assert_eq!(b[2], p("after"));
    assert_eq!(b[3], p("next"));
    let b =
        blocks(r"{\rtf1\ansi{\listtext 1.\tab}\pard\ls1 a\par{\footnote n}\par\pard after\par}");
    assert!(matches!(b[1], Block::Note { .. }), "{b:?}");
    assert_eq!(b[2], p("after"));
    // the first paragraph after a table keeps its own pictures and notes
    let t = r"{\rtf1\ansi\trowd\cellx1000\pard\intbl c\cell\row";
    let b = blocks(&format!(r"{t}\pard x{{\footnote xn}}\par\pard after\par}}"));
    assert!(matches!(b[0], Block::Table { .. }), "{b:?}");
    assert_eq!(b[1], p("x"));
    assert!(matches!(b[2], Block::Note { .. }), "{b:?}");
    assert_eq!(b[3], p("after"));
    assert_eq!(b.len(), 4);
    let b = blocks(&format!(
        r"{t}{{\listtext 1.\tab}}\pard\ls1 a{{\pict\pngblip 89}}\par{{\listtext 2.\tab}}\pard\ls1 b\par}}"
    ));
    assert!(matches!(b[0], Block::Table { .. }), "{b:?}");
    assert!(
        matches!(&b[1], Block::List { items, .. } if items.len() == 2 && matches!(items[0].blocks[..], [_, Block::Image { .. }]) && items[1].blocks.len() == 1),
        "{b:?}"
    );
    assert_eq!(b.len(), 2);
    let b = blocks(&format!(
        r"{t}{{\listtext 1.\tab}}\pard\ls1 a{{\footnote ln}}\par{{\listtext 2.\tab}}\pard\ls1 b\par\pard after\par}}"
    ));
    assert!(matches!(b[0], Block::Table { .. }), "{b:?}");
    assert!(matches!(b[1], Block::List { .. }), "{b:?}");
    assert!(matches!(b[2], Block::Note { .. }), "{b:?}");
    assert_eq!(b[3], p("after"));
    assert_eq!(b.len(), 4);
}

#[test]
fn tables_with_horizontal_and_vertical_merges() {
    let d = doc(concat!(
        r"{\rtf1\ansi",
        r"\trowd\trhdr\clmgf\cellx2000\clmrg\cellx4000\cellx6000",
        r"\pard\intbl H\cell\pard\intbl\cell\pard\intbl C\cell\row",
        r"\trowd\clvmgf\cellx2000\cellx4000\cellx6000",
        r"\pard\intbl a\cell\pard\intbl b\par b2\cell\pard\intbl\cell\row",
        r"\trowd\clvmrg\cellx2000\cellx4000\cellx6000",
        r"\pard\intbl\cell\pard\intbl c\cell\pard\intbl\cell\row",
        r"\pard after\par}"
    ));
    let b = &d.sections[0].blocks;
    let Block::Table {
        rows, header_rows, ..
    } = &b[0]
    else {
        panic!("{b:?}")
    };
    let sig: Vec<Vec<(String, u32, u32)>> = rows
        .iter()
        .map(|r| r.iter().map(|c| (c.text(), c.colspan, c.rowspan)).collect())
        .collect();
    assert_eq!(
        sig,
        vec![
            vec![("H".into(), 2, 1), ("C".into(), 1, 1)],
            vec![
                ("a".into(), 1, 2),
                ("b\nb2".into(), 1, 1),
                ("".into(), 1, 1)
            ],
            vec![("c".into(), 1, 1), ("".into(), 1, 1)],
        ]
    );
    assert_eq!(*header_rows, 1);
    assert_eq!(b[1], p("after"));
    assert!(kinds(&d).is_empty(), "{:?}", kinds(&d));
}

#[test]
fn nested_tables_are_flattened_into_the_cell() {
    let d = doc(
        r"{\rtf1\trowd\cellx5000\pard\intbl\itap2 x\nestcell{\*\nesttableprops\trowd\cellx1000\nestrow}{\nonesttables\par}\pard\intbl\itap1 y\cell\row\pard z\par}",
    );
    let Block::Table { rows, .. } = &d.sections[0].blocks[0] else {
        panic!()
    };
    assert_eq!(rows[0][0].blocks, vec![p("x"), p("y")]);
    assert_eq!(kinds(&d), vec![(WarningKind::ContentDropped, Level::Info)]);
}

#[test]
fn notes_comments_headers_footers_and_info() {
    let d = doc(
        r"{\rtf1\ansi{\info{\title Report}{\author Alex}{\company X}}{\header \pard Head\par}{\footer \pard Foot\par}\pard Text{\super\chftn}{\footnote \pard\plain {\super\chftn} Note text\par} more{\*\atnid I}{\*\annotation{\*\atndate 1} Comment}\par\pard next\par}",
    );
    assert_eq!(
        d.sections[0].blocks,
        vec![
            Block::Note {
                kind: NoteKind::Header,
                text: "Head".into(),
                bbox: None
            },
            p("Text more"),
            Block::Note {
                kind: NoteKind::Footnote,
                text: "Note text".into(),
                bbox: None
            },
            Block::Note {
                kind: NoteKind::Comment,
                text: "Comment".into(),
                bbox: None
            },
            p("next"),
            Block::Note {
                kind: NoteKind::Footer,
                text: "Foot".into(),
                bbox: None
            },
        ]
    );
    assert_eq!(d.metadata.title.as_deref(), Some("Report"));
    assert_eq!(d.metadata.author.as_deref(), Some("Alex"));
    // header and footer stay in the model, Markdown shows them only on request
    assert_eq!(
        d.to_markdown(),
        "Text more\n\n> Footnote: Note text\n\n> Comment: Comment\n\nnext\n"
    );
}

#[test]
fn bad_inputs_are_errors_or_warnings() {
    assert!(matches!(run(b"not rtf"), Err(ParseError::Corrupt(_))));
    let deep = format!("{{\\rtf1 {}x", "{".repeat(5000));
    assert!(
        matches!(run(deep.as_bytes()), Err(ParseError::Limit { kind, limit: 1024 }) if kind == "rtf group depth")
    );
    let d = doc(r"{\rtf1 Hello\par");
    assert_eq!(d.sections[0].blocks, vec![p("Hello")]);
    assert_eq!(kinds(&d), vec![(WarningKind::Truncated, Level::Warn)]);
    let d = doc(r"{\rtf1\ansi{\fonttbl{\f0 Arial;}}}");
    assert_eq!(kinds(&d), vec![(WarningKind::EmptyOutput, Level::Warn)]);
    // F1: hostile \ilvl values never nest lists deeper than 9 levels
    let mut src = String::from(r"{\rtf1\ansi");
    for i in 0..200 {
        src.push_str(&format!(r"{{\listtext 1.\tab}}\pard\ls1\ilvl{i} x\par"));
    }
    src.push('}');
    fn depth(b: &[Block]) -> usize {
        b.iter()
            .map(|b| match b {
                Block::List { items, .. } => {
                    1 + items.iter().map(|i| depth(&i.blocks)).max().unwrap_or(0)
                }
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }
    assert!(depth(&blocks(&src)) <= 9);
}

#[test]
fn rtf_saved_as_doc_goes_through_the_registry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("letter.doc");
    std::fs::write(
        &path,
        r"{\rtf1\ansi\ansicpg1251 \'c4\'ee\'e1\'f0\'fb\'e9 \'e4\'e5\'ed\'fc\par}",
    )
    .unwrap();
    let d = Registry::with_defaults()
        .parse(Input::Path(path), &ParseOptions::default())
        .unwrap();
    assert_eq!(d.metadata.format, Some(Format::Rtf));
    assert_eq!(d.to_markdown(), "Добрый день\n");
}

#[test]
fn parallel_parse_matches_sequential() {
    let docs: Vec<Vec<u8>> = (0..16).map(|i| format!(r"{{\rtf1\trowd\cellx1000\cellx2000\pard\intbl {i}\cell x\cell\row\pard\outlinelevel1 H{i}\par}}").into_bytes()).collect();
    let r = Registry::with_defaults();
    let o = ParseOptions::default();
    let seq: Vec<String> = docs
        .iter()
        .map(|b| r.parse(Input::Bytes(b.clone()), &o).unwrap().to_json())
        .collect();
    let par: Vec<String> = r
        .parse_many(docs.into_iter().map(Input::Bytes).collect(), &o)
        .into_iter()
        .map(|d| d.unwrap().to_json())
        .collect();
    assert_eq!(seq, par);
}

#[test]
fn whitespace_run_is_not_rescanned_per_row() {
    let src = format!(
        r"{{\rtf1\trowd\cellx1000{}{}}}",
        " ".repeat(1 << 20),
        r"\row".repeat(200_000)
    );
    let t = std::time::Instant::now();
    run(src.as_bytes()).unwrap();
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn upr_keeps_the_unicode_copy() {
    assert_eq!(
        blocks(r"{\rtf1 {\upr{\fs20 a}{\*\ud{\fs20 b}}} c\par}"),
        vec![p("b c")]
    );
}

// Ruling 6 and I2, ruling 4; F10: the text cap does not apply to rtf.
#[test]
fn input_cap_is_max_rtf_bytes_or_max_size() {
    let small = format!(r"{{\rtf1 {}\par}}", "x".repeat(80));
    let mut o = ParseOptions::default();
    o.max_text_bytes = 16;
    let d = RtfParser
        .parse(Input::Bytes(small.clone().into_bytes()), Format::Rtf, &o)
        .unwrap();
    assert!(d.to_markdown().contains("xxxx"));
    let mut o = ParseOptions::default();
    o.max_size = Some(10);
    let r = RtfParser.parse(Input::Bytes(small.into_bytes()), Format::Rtf, &o);
    assert!(
        matches!(r, Err(ParseError::TooLarge { limit: 10, .. })),
        "{r:?}"
    );
}

#[test]
fn too_many_cells_is_a_limit_and_cell_pictures_are_counted() {
    let src = format!(
        r"{{\rtf1\trowd\cellx1000{}\row}}",
        r"\cell".repeat(2_000_001)
    );
    let r = run(src.as_bytes());
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, limit: 2_000_000 }) if kind == "table cells"),
        "{:?}",
        r.err()
    );
    let d = doc(r"{\rtf1\trowd\cellx1000\cellx2000\pard\intbl{\pict\pngblip 89}a\cell b\cell\row}");
    assert_eq!(kinds(&d), vec![(WarningKind::ContentDropped, Level::Info)]);
    assert_eq!(
        d.sections[0].warnings[0].message,
        "rtf: 1 images in table cells dropped"
    );
}
