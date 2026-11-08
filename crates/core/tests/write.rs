//! `write_markdown_with` / `write_json` give the bytes of `to_markdown_with` / `to_json` and pass
//! write errors through.

use plaindoc_core::*;
use std::io::{self, Write};

fn section(kind: SectionKind, index: usize, blocks: Vec<Block>) -> Section {
    let mut s = Section::new(kind, index);
    s.blocks = blocks;
    s
}

fn row(cells: &[&str]) -> Vec<Cell> {
    cells.iter().map(|t| Cell::new(*t)).collect()
}

fn table(rows: Vec<Vec<Cell>>, header_rows: usize) -> Block {
    Block::Table {
        rows,
        header_rows,
        bbox: None,
    }
}

fn linked(text: &str, links: Vec<Link>) -> Block {
    let mut b = Block::paragraph(text, None);
    if let Block::Paragraph { links: l, .. } = &mut b {
        *l = links;
    }
    b
}

fn picture(file: Option<&str>) -> Block {
    let mut info = match file {
        Some(f) => {
            let mut i = ImageInfo::default();
            i.file = Some(f.into());
            i
        }
        None => ImageInfo::external("http://e.org/a (1).png"),
    };
    info.size = Some(3);
    let mut b = Block::image(None, Some("ocr".into()), None);
    if let Block::Image { image, .. } = &mut b {
        *image = Some(Box::new(info));
    }
    b
}

fn docs() -> Vec<Document> {
    let mut list = ListItem::new("item");
    list.blocks
        .push(table(vec![row(&["a", "b"]), row(&["1", "2"])], 1));
    let rich = vec![
        Block::heading(1, "Title", None),
        Block::paragraph("", None),
        linked("see here", vec![Link::new(4, 8, "http://e.org/x y")]),
        Block::paragraph("Рисунок 1. Схема", None),
        picture(Some("image1.png")),
        picture(None),
        table(Vec::new(), 0),
        table(vec![row(&["h|1", "h2"]), row(&["a\nb", "c"])], 1),
        table(
            vec![
                vec![Cell::new("m").with_span(2, 1)],
                row(&["x", "y"]),
                row(&["<", "&"]),
            ],
            1,
        ),
        Block::List {
            ordered: true,
            items: vec![list, ListItem::new("two")],
            bbox: None,
        },
        Block::Note {
            kind: NoteKind::Header,
            text: "running head".into(),
            bbox: None,
        },
        Block::List {
            ordered: false,
            items: Vec::new(),
            bbox: None,
        },
    ];
    let mut warned = Document::new(vec![
        section(SectionKind::Page, 0, rich.clone()),
        section(SectionKind::Page, 1, Vec::new()),
        section(SectionKind::Page, 2, vec![Block::paragraph("last", None)]),
        section(SectionKind::Sheet, 0, vec![table(vec![row(&["s"])], 0)]),
    ]);
    warned.warnings.push(Warning::new(
        WarningKind::NoTextLayer,
        Level::Warn,
        "doc -- level",
    ));
    warned.sections[1].warnings.push(Warning::new(
        WarningKind::TableUncertain,
        Level::Loss,
        "page",
    ));
    vec![
        Document::default(),
        Document::new(vec![section(SectionKind::Body, 0, Vec::new())]),
        Document::new(vec![section(SectionKind::Body, 0, rich)]),
        warned,
    ]
}

fn option_sets() -> Vec<MarkdownOptions> {
    let mut sets = Vec::new();
    for sep in [
        PageSeparator::Rule,
        PageSeparator::Comment,
        PageSeparator::None,
    ] {
        for (warnings, headers_footers) in [(true, false), (false, true)] {
            let mut o = MarkdownOptions::default();
            o.page_separator = sep;
            o.warnings = warnings;
            o.headers_footers = headers_footers;
            o.image_dir = headers_footers.then(|| "img-abc".to_string());
            sets.push(o);
        }
    }
    sets
}

#[test]
fn writers_give_the_same_bytes() {
    for d in docs() {
        let mut j = Vec::new();
        d.write_json(&mut j).unwrap();
        assert_eq!(j, d.to_json().into_bytes());
        let mut m = Vec::new();
        d.write_markdown(&mut m).unwrap();
        assert_eq!(m, d.to_markdown().into_bytes());
        for o in option_sets() {
            let mut m = Vec::new();
            d.write_markdown_with(&o, &mut m).unwrap();
            assert_eq!(m, d.to_markdown_with(&o).into_bytes(), "{o:?}");
        }
    }
    assert_eq!(Document::default().to_markdown(), "");
}

/// Accepts `left` bytes, then fails with `kind`.
struct FailAfter {
    got: Vec<u8>,
    left: usize,
    kind: io::ErrorKind,
}

impl Write for FailAfter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(self.kind.into());
        }
        let n = buf.len().min(self.left);
        self.got.extend_from_slice(&buf[..n]);
        self.left -= n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn fails_at_every_byte(full: &[u8], write: impl Fn(&mut FailAfter) -> io::Result<()>) {
    for left in 0..full.len() {
        for kind in [io::ErrorKind::BrokenPipe, io::ErrorKind::StorageFull] {
            let mut w = FailAfter {
                got: Vec::new(),
                left,
                kind,
            };
            let e = write(&mut w).expect_err("the sink failed");
            assert_eq!(e.kind(), kind);
            assert_eq!(w.got, full[..left]);
        }
    }
}

#[test]
fn write_errors_are_returned() {
    let mut o = MarkdownOptions::default();
    o.page_separator = PageSeparator::Comment;
    for d in docs() {
        fails_at_every_byte(d.to_markdown_with(&o).as_bytes(), |w| {
            d.write_markdown_with(&o, w)
        });
        fails_at_every_byte(d.to_json().as_bytes(), |w| d.write_json(w));
    }
}

/// A document with streamed tables next to other parts, as plain Markdown and as a Markdown
/// source (no escaping); the expected text is the output of the string renderer before streaming.
fn golden() -> Vec<(Document, String)> {
    let blocks = vec![
        Block::paragraph("a", None),
        table(Vec::new(), 0),
        table(vec![row(&["x", "y"]), row(&["1", "2"])], 1),
        Block::paragraph("# b", None),
    ];
    let merged = table(
        vec![vec![Cell::new("m").with_span(2, 1)], row(&["1", "2"])],
        0,
    );
    let doc = || {
        Document::new(vec![
            section(SectionKind::Page, 0, blocks.clone()),
            section(SectionKind::Page, 1, vec![merged.clone()]),
        ])
    };
    let mut md = doc();
    md.metadata.format = Some(Format::Md);
    let tail = "\n\n---\n\n<table>\n<tbody>\n<tr><td colspan=\"2\">m</td></tr>\n\
                <tr><td>1</td><td>2</td></tr>\n</tbody>\n</table>\n";
    vec![
        (doc(), "a\n\n| x | y |\n| --- | --- |\n| 1 | 2 |\n\n\\# b"),
        (md, "a\n\n| x | y |\n| --- | --- |\n| 1 | 2 |\n\n# b"),
    ]
    .into_iter()
    .map(|(d, head)| (d, format!("{head}{tail}")))
    .collect()
}

#[test]
fn streamed_tables_join_their_neighbours() {
    for (d, want) in golden() {
        assert_eq!(d.to_markdown(), want);
        let mut buf = Vec::new();
        let out: &mut dyn Write = &mut buf;
        d.write_markdown(out).unwrap();
        assert_eq!(buf, want.as_bytes());
    }
}

#[test]
fn large_pieces_keep_their_place() {
    let big = "x".repeat(200_000);
    let d = Document::new(vec![section(
        SectionKind::Body,
        0,
        vec![
            Block::paragraph("a", None),
            Block::paragraph(big.clone(), None),
            Block::paragraph("b", None),
        ],
    )]);
    let want = format!("a\n\n{big}\n\nb\n");
    let mut m = Vec::new();
    d.write_markdown(&mut m).unwrap();
    assert_eq!(m, want.as_bytes());
    assert_eq!(d.to_markdown(), want);
    for left in [0, 1, 3, 70_000, 200_003, want.len() - 1] {
        let mut w = FailAfter {
            got: Vec::new(),
            left,
            kind: io::ErrorKind::BrokenPipe,
        };
        let e = d.write_markdown(&mut w).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(w.got, want.as_bytes()[..left]);
    }
}
