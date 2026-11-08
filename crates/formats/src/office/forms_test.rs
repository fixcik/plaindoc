//! Forms, tabs, note labels and the DOCX title (`inline`, `form`, `title`).

use office_oxide::DocumentFormat;
use office_oxide::ir::{
    BorderLine, BorderStyle, DocumentIR, Element, FootnoteRef, InlineContent, Note, Paragraph,
    ParagraphAlignment, ParagraphBorder, Section, TabAlignment, TabLeader, TabStop, TextSpan,
    VerticalAlign,
};
use plaindoc_core::{Block, NoteKind};

use crate::office::pictures::Pictures;
use crate::office::section::map_document;

fn span(t: &str, half_pt: u32) -> InlineContent {
    InlineContent::Text(TextSpan {
        text: t.into(),
        font_size_half_pt: Some(half_pt),
        ..Default::default()
    })
}

fn p(content: Vec<InlineContent>) -> Paragraph {
    Paragraph {
        content,
        ..Default::default()
    }
}

/// A paragraph with a line above it (top border).
fn lined(content: Vec<InlineContent>) -> Element {
    let line = BorderLine {
        style: BorderStyle::Single,
        color: None,
        size: Some(4),
        space: Some(1),
    };
    Element::Paragraph(Paragraph {
        border: Some(ParagraphBorder {
            top: Some(line),
            bottom: None,
            left: None,
            right: None,
            between: None,
        }),
        ..p(content)
    })
}

fn texts_of(els: Vec<Element>, format: DocumentFormat) -> Vec<String> {
    let ir = DocumentIR {
        sections: vec![Section {
            elements: els,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut ir = ir;
    ir.metadata.format = format;
    let (doc, _) = map_document(&mut ir, false, None, Pictures::default());
    doc.sections[0]
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph { text, .. } => text.clone(),
            Block::Heading { level, text, .. } => format!("{} {text}", "#".repeat(*level as usize)),
            Block::Note {
                kind: NoteKind::Footnote,
                text,
                ..
            } => format!("FN {text}"),
            Block::Note {
                kind: NoteKind::Endnote,
                text,
                ..
            } => format!("EN {text}"),
            other => panic!("{other:?}"),
        })
        .collect()
}

fn texts(els: Vec<Element>) -> Vec<String> {
    texts_of(els, DocumentFormat::Docx)
}

#[test]
fn tabs_never_lead_or_trail_and_keep_columns() {
    let got = texts(vec![
        Element::Paragraph(p(vec![span("Стол письменный\t1 500\tшт.\t\t2\t3 000", 22)])),
        Element::Paragraph(p(vec![span("\t \tx\t \t", 22)])),
        Element::Paragraph(p(vec![
            span("Строка один", 22),
            InlineContent::LineBreak,
            span("\t- не список", 22),
            InlineContent::LineBreak,
            span("  ещё", 22),
        ])),
    ]);
    assert_eq!(
        got,
        [
            "Стол письменный\t1 500\tшт.\t2\t3 000",
            "x",
            "Строка один\n- не список\nещё",
        ]
    );
}

fn stop(position_twips: i32, leader: TabLeader) -> TabStop {
    TabStop {
        position_twips,
        alignment: TabAlignment::Right,
        leader,
    }
}

#[test]
fn right_tab_alone_is_not_a_field() {
    let mut date = p(vec![span("г. Москва\t«01» января 2024 г.", 22)]);
    date.tabs = vec![stop(9356, TabLeader::None)];
    let got = texts(vec![
        Element::Paragraph(date),
        Element::Paragraph(p(vec![span("Текст", 22)])),
    ]);
    assert_eq!(got, ["г. Москва\t«01» января 2024 г.", "Текст"]);
}

#[test]
fn line_above_a_hint_makes_a_field() {
    let got = texts(vec![
        Element::Paragraph(p(vec![span("1.  ", 22)])),
        lined(vec![span("(фамилия, имя, отчество", 18)]),
        Element::Paragraph(p(vec![span("\t", 22), span(".", 22)])),
        lined(vec![span("должность)", 18)]),
        // An empty field paragraph: the field goes in front of its hint.
        Element::Paragraph(p(vec![])),
        lined(vec![span("(подсказка)", 18)]),
        Element::Paragraph(p(vec![
            span("б) решение по результатам\t", 22),
            span(";", 22),
        ])),
        lined(vec![]),
        // Underscores typed by hand are the field already.
        Element::Paragraph(p(vec![span("Директор ____ ", 22)])),
        lined(vec![span("(подпись)", 18)]),
    ]);
    assert_eq!(
        got,
        [
            "1. _____",
            "(фамилия, имя, отчество",
            "_____.",
            "должность)",
            "_____ (подсказка)",
            "б) решение по результатам _____;",
            "Директор ____",
            "(подпись)",
        ]
    );
}

#[test]
fn a_line_under_plain_text_is_not_a_field() {
    let mut boxed = lined(vec![span("в рамке", 18)]);
    if let Element::Paragraph(b) = &mut boxed {
        let bd = b.border.as_mut().unwrap();
        bd.left = bd.top.clone();
        bd.right = bd.top.clone();
    }
    // Two paragraphs in one group with lines above and below: one line above the first only.
    let group = |t: &str, half_pt: u32| {
        let mut e = lined(vec![span(t, half_pt)]);
        if let Element::Paragraph(b) = &mut e {
            let bd = b.border.as_mut().unwrap();
            bd.bottom = bd.top.clone();
        }
        e
    };
    let got = texts(vec![
        // No room for an answer: a rule under a name.
        Element::Paragraph(p(vec![span("Иван Петров", 22)])),
        lined(vec![]),
        // The next paragraph is a box, not a line.
        Element::Paragraph(p(vec![span("1.  ", 22)])),
        boxed,
        // A section title with a rule above: not smaller than the text before it.
        Element::Paragraph(p(vec![span("Опыт ", 22)])),
        lined(vec![span("Образование", 22)]),
        // A rule under an empty paragraph.
        Element::Paragraph(p(vec![])),
        lined(vec![]),
        group("Внимание: условия действуют до 31.12.2025 ", 24),
        group("Подробности в приложении.", 20),
    ]);
    assert_eq!(
        got,
        [
            "Иван Петров",
            "1.  ",
            "в рамке",
            "Опыт ",
            "Образование",
            "Внимание: условия действуют до 31.12.2025 ",
            "Подробности в приложении.",
        ]
    );
}

#[test]
fn only_tabs_to_an_underscore_stop_are_fields() {
    let mut a = p(vec![span("1.\tФамилия\t", 22)]);
    a.tabs = vec![
        stop(9355, TabLeader::Underscore),
        stop(709, TabLeader::None),
    ];
    let mut b = p(vec![span("Имя\tОтчество\t", 22)]);
    b.tabs = vec![
        stop(4000, TabLeader::Underscore),
        stop(9000, TabLeader::Underscore),
    ];
    assert_eq!(
        texts(vec![Element::Paragraph(a), Element::Paragraph(b)]),
        ["1.\tФамилия _____", "Имя _____ Отчество _____"]
    );
}

fn note(id: u32, content: Vec<InlineContent>) -> Note {
    Note {
        id,
        content: vec![Element::Paragraph(p(content))],
        marker: None,
        author: None,
    }
}

fn sup(t: &str) -> InlineContent {
    InlineContent::Text(TextSpan {
        text: t.into(),
        vertical_align: Some(VerticalAlign::Superscript),
        ..Default::default()
    })
}

fn fref(id: u32) -> InlineContent {
    InlineContent::FootnoteRef(FootnoteRef {
        note_id: id,
        marker: None,
    })
}

#[test]
fn note_citations_get_labels_in_citation_order() {
    let got = texts(vec![
        Element::Paragraph(p(vec![
            span("Ouch ", 22),
            fref(7),
            span(".", 22),
            InlineContent::EndnoteRef(FootnoteRef {
                note_id: 3,
                marker: None,
            }),
        ])),
        // Custom mark (`customMarkFollows`): the superscript text after the reference.
        Element::Paragraph(p(vec![
            span("наставничества ", 22),
            fref(2),
            sup("2"),
            span(".", 22),
        ])),
        Element::Footnote(note(2, vec![sup("2"), span(" Прилагается.", 20)])),
        Element::Footnote(note(7, vec![span(" A tachyon.", 20)])),
        Element::Footnote(note(9, vec![span("Uncited.", 20)])),
        Element::Endnote(note(3, vec![span(" Fin.", 20)])),
    ]);
    assert_eq!(
        got,
        [
            "Ouch[1].[1]",
            "наставничества[2].",
            "FN [2] Прилагается.",
            "FN [1] A tachyon.",
            "FN Uncited.",
            "EN [1] Fin.",
        ]
    );
}

fn title_para(t: &str, half_pt: u32, center: bool) -> Element {
    Element::Paragraph(Paragraph {
        content: vec![InlineContent::Text(TextSpan {
            text: t.into(),
            bold: true,
            font_size_half_pt: Some(half_pt),
            ..Default::default()
        })],
        alignment: center.then_some(ParagraphAlignment::Center),
        ..Default::default()
    })
}

fn body() -> Vec<Element> {
    (0..3)
        .map(|_| Element::Paragraph(p(vec![span("Основной текст документа.", 22)])))
        .collect()
}

#[test]
fn bold_centred_larger_start_is_the_title() {
    let mut els = vec![
        Element::Paragraph(p(vec![])),
        title_para("ОТЗЫВ", 26, true),
        title_para("о результатах", 26, true),
        title_para("испытания", 26, true),
    ];
    els.extend(body());
    let got = texts(els);
    assert_eq!(got[..2], ["# ОТЗЫВ о результатах", "испытания"]);
}

#[test]
fn no_title_without_every_sign() {
    let with = |first: Element| {
        let mut els = vec![first];
        els.extend(body());
        els
    };
    // Same size as the body, not centred.
    assert_eq!(texts(with(title_para("ОТЗЫВ", 22, true)))[0], "ОТЗЫВ");
    assert_eq!(texts(with(title_para("ОТЗЫВ", 26, false)))[0], "ОТЗЫВ");
    // Not DOCX.
    assert_eq!(
        texts_of(with(title_para("ОТЗЫВ", 26, true)), DocumentFormat::Doc)[0],
        "ОТЗЫВ"
    );
    // The document has headings of its own.
    let mut els = with(title_para("ОТЗЫВ", 26, true));
    els.push(crate::office::test_ir::heading(1, "Раздел"));
    assert_eq!(texts(els)[0], "ОТЗЫВ");
}
