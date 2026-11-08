//! Mapping of `office_oxide` IR elements to core `Block`s.

use office_oxide::DocumentFormat;
use office_oxide::ir::Element;
use plaindoc_core::{Block, Level, ListItem, NoteKind, Warning, WarningKind};

use super::inline::{FIELD, NoteLabels, para_text};
use super::pictures::Pictures;
use super::text::{elements_text, list_text};

/// Nesting deeper than this is flattened to text.
pub(crate) const MAX_MAP_DEPTH: usize = 128;

#[derive(Default)]
pub(crate) struct MapCtx {
    pub format: DocumentFormat,
    pub date1904: bool,
    pub depth: usize,
    pub warnings: Vec<Warning>,
    pub flattened: bool,
    pub pics: Pictures,
    pub notes: NoteLabels,
    /// Look for a [`super::title`] at the start of the next section (DOCX without headings).
    pub find_title: bool,
}

/// Paragraph of `text` for content past [`MAX_MAP_DEPTH`]; warns once per context.
fn flatten_to_paragraph(text: String, ctx: &mut MapCtx) -> Block {
    if !ctx.flattened {
        ctx.flattened = true;
        ctx.warnings.push(Warning::new(
            WarningKind::ContentDropped,
            Level::Info,
            format!("office: nesting deeper than {MAX_MAP_DEPTH} flattened to text"),
        ));
    }
    Block::paragraph(text, None)
}

/// Maps `els`; table rows are taken out and freed as they are mapped (the rest stays: OCR reads
/// the images afterwards).
pub(crate) fn map_elements(els: &mut [Element], ctx: &mut MapCtx) -> Vec<Block> {
    if ctx.depth >= MAX_MAP_DEPTH {
        return vec![flatten_to_paragraph(elements_text(els), ctx)];
    }
    let mut out = Vec::new();
    // An empty field paragraph: its field goes in front of the hint under it.
    let mut lead_field = false;
    for i in 0..els.len() {
        let (e, rest) = els[i..].split_first_mut().expect("i < len");
        match e {
            Element::Heading(h) => out.push(Block::heading(
                h.clamped_level(),
                para_text(&h.content, &h.tabs, false, Some(&mut ctx.notes)),
                None,
            )),
            Element::Paragraph(p) => {
                let field = super::form::is_field(p, rest.first());
                // Spreadsheet cells are data: tabs and leading spaces stay.
                let text = if matches!(ctx.format, DocumentFormat::Xlsx | DocumentFormat::Xls) {
                    office_oxide::ir::inline_to_text(&p.content)
                } else {
                    para_text(&p.content, &p.tabs, field, Some(&mut ctx.notes))
                };
                if field && text == FIELD {
                    lead_field = true;
                    continue;
                }
                let text = if std::mem::take(&mut lead_field) {
                    format!("{FIELD} {text}")
                } else {
                    text
                };
                if text.trim().is_empty() {
                    continue;
                }
                let kind = match p.placeholder_role.as_deref() {
                    Some("hdr") => Some(NoteKind::Header),
                    Some("ftr" | "dt") => Some(NoteKind::Footer),
                    Some("sldNum") => Some(NoteKind::PageNumber),
                    _ => None,
                };
                out.push(match kind {
                    Some(kind) => Block::Note {
                        kind,
                        text,
                        bbox: None,
                    },
                    None => Block::paragraph(text, None),
                });
            }
            Element::CodeBlock(c) => out.push(Block::Code {
                lang: c.language.clone(),
                text: c.content.clone(),
                bbox: None,
            }),
            Element::Image(i) => out.push(Block::Image {
                alt: i.alt_text.clone(),
                ocr_text: None,
                bbox: None,
                image: ctx.pics.info(i),
            }),
            Element::TextBox(tb) => out.extend(map_elements(&mut tb.content, ctx)),
            Element::List(l) => out.push(map_list(l, ctx)),
            Element::Footnote(n) => {
                let text = ctx.notes.body(false, n.id, elements_text(&n.content));
                if text.trim().is_empty() {
                    continue;
                }
                out.push(Block::Note {
                    kind: NoteKind::Footnote,
                    text,
                    bbox: None,
                });
            }
            Element::Endnote(n) => {
                let text = elements_text(&n.content);
                if text.trim().is_empty() {
                    continue;
                }
                let marker = n.marker.as_deref().filter(|m| !m.is_empty());
                let marker_is_comment = marker.is_some_and(|m| {
                    m.get(..7)
                        .is_some_and(|p| p.eq_ignore_ascii_case("comment"))
                });
                let comment = marker_is_comment
                    || matches!(
                        ctx.format,
                        DocumentFormat::Xlsx | DocumentFormat::Xls | DocumentFormat::Pptx
                    );
                let (kind, text) = if comment {
                    let text = match (marker, n.author.as_deref()) {
                        (Some(m), _) if !marker_is_comment => format!("{m}: {text}"),
                        (_, Some(a)) if !a.is_empty() => format!("{a}: {text}"),
                        _ => text,
                    };
                    (NoteKind::Comment, text)
                } else {
                    (NoteKind::Endnote, ctx.notes.body(true, n.id, text))
                };
                out.push(Block::Note {
                    kind,
                    text,
                    bbox: None,
                });
            }
            Element::Table(t) => out.push(super::table::map_table(t, ctx)),
            Element::ThematicBreak
            | Element::PageBreak
            | Element::ColumnBreak
            | Element::Shape(_) => {}
            _ => {}
        }
    }
    out
}

pub(crate) fn map_list(l: &mut office_oxide::ir::List, ctx: &mut MapCtx) -> Block {
    Block::List {
        ordered: l.ordered,
        items: map_items(&mut l.items, ctx),
        bbox: None,
    }
}

pub(crate) fn map_items(
    items: &mut [office_oxide::ir::ListItem],
    ctx: &mut MapCtx,
) -> Vec<ListItem> {
    items
        .iter_mut()
        .map(|item| {
            let mut li = ListItem::new("");
            ctx.depth += 1;
            li.blocks = map_elements(&mut item.content, ctx);
            if let Some(nested) = &mut item.nested {
                if ctx.depth >= MAX_MAP_DEPTH {
                    // Same flattening as `map_elements` at the limit.
                    li.blocks.push(flatten_to_paragraph(list_text(nested), ctx));
                } else {
                    li.blocks.push(map_list(nested, ctx));
                }
            }
            ctx.depth -= 1;
            li
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::test_ir::*;
    fn map(els: &[Element]) -> Vec<Block> {
        map_elements(&mut els.to_vec(), &mut MapCtx::default())
    }

    #[test]
    fn headings_paragraphs_and_empty_skip() {
        let b = map(&[heading(9, "Title"), para("  "), para("Text")]);
        assert!(matches!(&b[0], Block::Heading { level: 6, text, .. } if text == "Title"));
        assert!(matches!(&b[1], Block::Paragraph { text, .. } if text == "Text"));
        assert_eq!(b.len(), 2);
    }
    #[test]
    fn textbox_is_flattened_in_place() {
        let tb = Element::TextBox(office_oxide::ir::TextBox {
            content: vec![para("inside")],
            ..Default::default()
        });
        let b = map(&[para("a"), tb, para("b")]);
        let texts: Vec<_> = b
            .iter()
            .map(|x| match x {
                Block::Paragraph { text, .. } => text.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(texts, ["a", "inside", "b"]);
    }
    #[test]
    fn placeholder_roles_become_notes() {
        use office_oxide::ir::{InlineContent, Paragraph, TextSpan};
        let role = |r: &str| {
            let mut p = Paragraph {
                content: vec![InlineContent::Text(TextSpan::plain("7"))],
                ..Default::default()
            };
            p.placeholder_role = Some(r.into());
            Element::Paragraph(p)
        };
        let b = map(&[role("sldNum"), role("ftr"), role("hdr")]);
        let kinds: Vec<_> = b
            .iter()
            .map(|x| match x {
                Block::Note { kind, .. } => *kind,
                _ => panic!("{x:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [NoteKind::PageNumber, NoteKind::Footer, NoteKind::Header]
        );
    }
    #[test]
    fn footnote_endnote_and_comments_by_format() {
        use office_oxide::DocumentFormat as D;
        let note = |m: Option<&str>, a: Option<&str>, t: &str| office_oxide::ir::Note {
            id: 1,
            content: vec![para(t)],
            marker: m.map(Into::into),
            author: a.map(Into::into),
        };
        let doc = |mut els: Vec<Element>, f: D| {
            map_elements(
                &mut els,
                &mut MapCtx {
                    format: f,
                    ..Default::default()
                },
            )
        };
        let got = |b: Vec<Block>| {
            b.into_iter()
                .map(|x| match x {
                    Block::Note { kind, text, .. } => (kind, text),
                    _ => panic!(),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            got(doc(
                vec![
                    Element::Footnote(note(Some("footnote"), None, "TestFootnote")),
                    Element::Endnote(note(
                        Some("comment"),
                        Some("Maxim Valyanskiy"),
                        "TestComment"
                    )),
                    Element::Endnote(note(Some("endnote"), None, "TestEndnote")),
                    Element::Endnote(note(Some("Comment (Ann)"), Some("Ann"), "looks fine")),
                ],
                D::Doc
            )),
            [
                (NoteKind::Footnote, "TestFootnote".to_string()),
                (
                    NoteKind::Comment,
                    "Maxim Valyanskiy: TestComment".to_string()
                ),
                (NoteKind::Endnote, "TestEndnote".to_string()),
                (NoteKind::Comment, "Ann: looks fine".to_string()),
            ]
        );
        assert_eq!(
            got(doc(
                vec![Element::Endnote(note(Some("A2 (Ann)"), Some("Ann"), "t"))],
                D::Xlsx
            )),
            [(NoteKind::Comment, "A2 (Ann): t".to_string())]
        );
    }
    #[test]
    fn image_keeps_alt_and_breaks_are_dropped() {
        let b = map(&[
            image(Some("logo")),
            Element::PageBreak,
            Element::ThematicBreak,
        ]);
        assert_eq!(b.len(), 1);
        assert!(matches!(&b[0], Block::Image { alt: Some(a), .. } if a == "logo"));
    }
    #[test]
    fn depth_cap_flattens_with_one_content_dropped_warning() {
        let mut ctx = MapCtx {
            depth: MAX_MAP_DEPTH,
            ..Default::default()
        };
        let b = map_elements(&mut [para("x"), para("y")], &mut ctx);
        map_elements(&mut [para("z")], &mut ctx);
        assert!(matches!(&b[..], [Block::Paragraph { text, .. }] if text == "x\ny"));
        assert_eq!(
            ctx.warnings
                .iter()
                .map(|w| (w.kind, w.level))
                .collect::<Vec<_>>(),
            [(WarningKind::ContentDropped, Level::Info)]
        );
    }
    #[test]
    fn elements_text_flattens_tables_and_lists() {
        let t = table(vec![row(vec![cell("a", 1, 1), cell("b", 1, 1)], false)]);
        let sub = ir_list(&list(false, vec![item("sub", None)]));
        let l = list(false, vec![item("one", Some(sub))]);
        assert_eq!(
            crate::office::text::elements_text(&[t, l]),
            "a | b\none\n  sub"
        );
    }
    #[test]
    fn nested_list_keeps_tree_and_own_ordered() {
        let sub = ir_list(&list(false, vec![item("one.a", None), item("one.b", None)]));
        let b = map(&[list(true, vec![item("one", Some(sub)), item("two", None)])]);
        let Block::List {
            ordered: true,
            items,
            ..
        } = &b[0]
        else {
            panic!("{b:?}")
        };
        assert_eq!(
            items.iter().map(ListItem::text).collect::<Vec<_>>(),
            ["one", "two"]
        );
        let [
            Block::Paragraph { .. },
            Block::List {
                ordered: false,
                items: sub,
                ..
            },
        ] = &items[0].blocks[..]
        else {
            panic!("{:?}", items[0])
        };
        assert_eq!(
            sub.iter().map(ListItem::text).collect::<Vec<_>>(),
            ["one.a", "one.b"]
        );
        assert_eq!(items[1].blocks.len(), 1);
    }
    #[test]
    fn list_item_may_hold_a_table() {
        let mut it = item("with table", None);
        it.content
            .push(table(vec![row(vec![cell("x", 1, 1)], false)]));
        let b = map(&[list(false, vec![it])]);
        let Block::List { items, .. } = &b[0] else {
            panic!()
        };
        assert!(matches!(
            &items[0].blocks[..],
            [Block::Paragraph { .. }, Block::Table { .. }]
        ));
    }
}
