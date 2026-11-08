//! PDF link annotations end to end on generated PDFs (Courier, 0.6 em per char, so the
//! rectangles of words are known): precision and recall of the links over the case set, the
//! URL filter, hidden text and annotations, and the switch.

use plaindoc_core::*;
use plaindoc_formats::pdf::PdfParser;
use std::collections::BTreeSet;

/// A line of Courier text: `(x, baseline y, size, text)`.
type Line = (f32, f32, f32, &'static str);

/// Box of chars `from..to` of `line` (PDF coordinates).
fn rect(line: Line, from: usize, to: usize) -> [f32; 4] {
    let (x, y, size, _) = line;
    let w = 0.6 * size;
    [
        x + from as f32 * w,
        y - 0.25 * size,
        x + to as f32 * w,
        y + 0.85 * size,
    ]
}

fn pdf_str(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// A link annotation over `rects` (`/Rect` = their union; `/QuadPoints` when `quads`), with
/// `extra` keys.
fn annot(rects: &[[f32; 4]], url: &str, quads: bool, extra: &str) -> String {
    let u = rects
        .iter()
        .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, r| {
            [
                a[0].min(r[0]),
                a[1].min(r[1]),
                a[2].max(r[2]),
                a[3].max(r[3]),
            ]
        });
    let q = if quads {
        let v: Vec<String> = rects
            .iter()
            .map(|r| {
                format!(
                    "{} {} {} {} {} {} {} {}",
                    r[0], r[3], r[2], r[3], r[0], r[1], r[2], r[1]
                )
            })
            .collect();
        format!("/QuadPoints [{}]", v.join(" "))
    } else {
        String::new()
    };
    format!(
        "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] {q} {extra} \
         /A << /S /URI /URI ({}) >> >>",
        u[0],
        u[1],
        u[2],
        u[3],
        pdf_str(url)
    )
}

/// A one-page PDF: `lines` in Courier, `paths` (content operators), `annots` as indirect objects,
/// `keys` in the page dictionary (MediaBox default `[0 0 612 792]`).
fn pdf(lines: &[Line], paths: &str, annots: &[String], keys: &str) -> Vec<u8> {
    let mut content = String::from(paths);
    for (x, y, size, t) in lines {
        content.push_str(&format!(
            "\nBT /F1 {size} Tf {x} {y} Td ({}) Tj ET",
            pdf_str(t)
        ));
    }
    let first_annot = 6;
    let refs: Vec<String> = (0..annots.len())
        .map(|i| format!("{} 0 R", first_annot + i))
        .collect();
    let media = if keys.contains("/MediaBox") {
        ""
    } else {
        "/MediaBox [0 0 612 792]"
    };
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R {media} {keys} /Annots [{}] \
             /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
            refs.join(" ")
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    objs.extend(annots.iter().cloned());
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let x = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
            objs.len() + 1
        )
        .bytes(),
    );
    out
}

fn parse(b: Vec<u8>, opts: &ParseOptions) -> Document {
    PdfParser::new()
        .parse(Input::Bytes(b), Format::Pdf, opts)
        .unwrap()
}

fn collect(b: &Block, out: &mut Vec<(String, String, usize)>) {
    match b {
        Block::Heading { text, links, .. } | Block::Paragraph { text, links, .. } => {
            let chars: Vec<char> = text.chars().collect();
            for l in links {
                let t: String = chars[l.start..l.end].iter().collect();
                let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
                out.push((t, l.url.clone(), l.start));
            }
        }
        Block::Table { rows, .. } => rows
            .iter()
            .flatten()
            .flat_map(|c| &c.blocks)
            .for_each(|x| collect(x, out)),
        Block::List { items, .. } => items
            .iter()
            .flat_map(|i| &i.blocks)
            .for_each(|x| collect(x, out)),
        _ => {}
    }
}

/// `(anchor text with single spaces, url, start char)` of every link of the document.
fn links(d: &Document) -> Vec<(String, String, usize)> {
    let mut out = Vec::new();
    for b in d.sections.iter().flat_map(|s| &s.blocks) {
        collect(b, &mut out);
    }
    out
}

struct Case {
    name: &'static str,
    pdf: Vec<u8>,
    /// `(anchor, url)` expected.
    want: Vec<(&'static str, &'static str)>,
}

const P1: Line = (72.0, 700.0, 12.0, "Visit the example site today.");
const L1: Line = (72.0, 700.0, 12.0, "Read the full report at our");
const L2: Line = (72.0, 686.0, 12.0, "web page for all the details.");

fn cases() -> Vec<Case> {
    let one = |r: [f32; 4], u: &str| vec![annot(&[r], u, false, "")];
    let mut v = vec![
        Case {
            name: "one word",
            pdf: pdf(&[P1], "", &one(rect(P1, 18, 22), "https://e.org/site"), ""),
            want: vec![("site", "https://e.org/site")],
        },
        Case {
            name: "several words",
            pdf: pdf(&[P1], "", &one(rect(P1, 10, 22), "http://e.org/"), ""),
            want: vec![("example site", "http://e.org/")],
        },
        Case {
            name: "two lines, quads",
            pdf: pdf(
                &[L1, L2],
                "",
                &[annot(
                    &[rect(L1, 24, 27), rect(L2, 0, 8)],
                    "https://e.org/r",
                    true,
                    "",
                )],
                "",
            ),
            want: vec![("our web page", "https://e.org/r")],
        },
        Case {
            name: "two lines, one annotation per line",
            pdf: pdf(
                &[L1, L2],
                "",
                &[
                    annot(&[rect(L1, 24, 27)], "https://e.org/r", false, ""),
                    annot(&[rect(L2, 0, 8)], "https://e.org/r", false, ""),
                ],
                "",
            ),
            want: vec![("our web page", "https://e.org/r")],
        },
        Case {
            name: "mailto and two links in a line",
            pdf: pdf(
                &[P1],
                "",
                &[
                    annot(&[rect(P1, 0, 5)], "MAILTO:a@e.org", false, ""),
                    annot(&[rect(P1, 18, 22)], "https://e.org/s", false, ""),
                ],
                "",
            ),
            want: vec![("Visit", "mailto:a@e.org"), ("site", "https://e.org/s")],
        },
        Case {
            name: "half a word",
            pdf: pdf(&[P1], "", &one(rect(P1, 10, 14), "https://e.org/h"), ""),
            want: vec![("exam", "https://e.org/h")],
        },
        Case {
            name: "brackets and markup in the text, specials in the url",
            pdf: pdf(
                &[(72.0, 700.0, 12.0, "See [1] *a_b* for more.")],
                "",
                &one(
                    rect((72.0, 700.0, 12.0, ""), 4, 13),
                    "https://e.org/a b(c)[d]|e",
                ),
                "",
            ),
            want: vec![("[1] *a_b*", "https://e.org/a b(c)[d]|e")],
        },
        Case {
            name: "nonzero media box origin",
            pdf: pdf(
                &[P1],
                "",
                &one(rect(P1, 18, 22), "https://e.org/m"),
                "/MediaBox [10 20 622 812]",
            ),
            want: vec![("site", "https://e.org/m")],
        },
        Case {
            name: "rotated page",
            pdf: pdf(
                &[P1],
                "",
                &one(rect(P1, 18, 22), "https://e.org/rot"),
                "/Rotate 90",
            ),
            want: vec![("site", "https://e.org/rot")],
        },
        Case {
            name: "visible url without annotation",
            pdf: pdf(
                &[(72.0, 700.0, 12.0, "See https://e.org/x now.")],
                "",
                &[],
                "",
            ),
            want: vec![],
        },
    ];
    for bad in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "/relative/path",
        "data:text/html,x",
        "https://",
    ] {
        v.push(Case {
            name: "rejected scheme",
            pdf: pdf(&[P1], "", &one(rect(P1, 18, 22), bad), ""),
            want: vec![],
        });
    }
    // rotated text (the fork's span box runs along x, not on the ink): no link; the
    // horizontal neighbour still links
    v.push(Case {
        name: "turned text",
        pdf: pdf(
            &[P1],
            "BT /F1 12 Tf 0 1 -1 0 400 300 Tm (Rotated link text) Tj ET",
            &[
                annot(
                    &[[398.0, 298.0, 524.0, 314.0]],
                    "https://e.org/turn",
                    false,
                    "",
                ),
                annot(&[rect(P1, 18, 22)], "https://e.org/s", false, ""),
            ],
            "",
        ),
        want: vec![("site", "https://e.org/s")],
    });
    // nested rectangles (PMC2996953 p. 11: a licence line link inside a wider one): the
    // smaller link wins its glyphs, the wider one keeps the rest
    v.push(Case {
        name: "nested rectangles",
        pdf: pdf(
            &[P1],
            "",
            &[
                annot(&[rect(P1, 0, 29)], "https://e.org/wide", false, ""),
                annot(&[rect(P1, 10, 17)], "https://e.org/inner", false, ""),
            ],
            "",
        ),
        want: vec![
            ("Visit the", "https://e.org/wide"),
            ("example", "https://e.org/inner"),
            ("site today.", "https://e.org/wide"),
        ],
    });
    let long = format!("https://e.org/{}", "a".repeat(2100));
    v.push(Case {
        name: "overlong url",
        pdf: pdf(&[P1], "", &one(rect(P1, 18, 22), &long), ""),
        want: vec![],
    });
    v.push(Case {
        name: "hidden annotation",
        pdf: pdf(
            &[P1],
            "",
            &[annot(&[rect(P1, 18, 22)], "https://e.org/h", false, "/F 2")],
            "",
        ),
        want: vec![],
    });
    v
}

#[test]
fn links_of_the_case_set_have_precision_and_recall_one() {
    let opts = ParseOptions::default();
    let (mut tp, mut got_n, mut want_n) = (0, 0, 0);
    for c in cases() {
        let d = parse(c.pdf, &opts);
        let got: BTreeSet<(String, String)> =
            links(&d).into_iter().map(|(t, u, _)| (t, u)).collect();
        let want: BTreeSet<(String, String)> = c
            .want
            .iter()
            .map(|(t, u)| (t.to_string(), u.to_string()))
            .collect();
        tp += got.intersection(&want).count();
        got_n += got.len();
        want_n += want.len();
        assert_eq!(got, want, "{}: {}", c.name, d.to_markdown());
    }
    assert_eq!((tp, got_n), (want_n, want_n));
}

#[test]
fn markdown_and_json_show_the_links() {
    let b = pdf(
        &[(72.0, 700.0, 12.0, "See [1] *a_b* for more.")],
        "",
        &[annot(
            &[rect((72.0, 700.0, 12.0, ""), 4, 13)],
            "https://e.org/a b(c)",
            false,
            "",
        )],
        "",
    );
    let d = parse(b, &ParseOptions::default());
    assert!(
        d.to_markdown()
            .contains("See [\\[1\\] *a_b*](https://e.org/a%20b%28c%29) for more."),
        "{}",
        d.to_markdown()
    );
    assert!(
        d.to_json()
            .contains("\"links\": [\n            {\n              \"start\": 4,\n              \"end\": 13,\n              \"url\": \"https://e.org/a b(c)\""),
        "{}",
        d.to_json()
    );
}

#[test]
fn the_same_text_twice_links_the_covered_one() {
    let line: Line = (72.0, 700.0, 12.0, "Click here or here to go.");
    let b = pdf(
        &[line],
        "",
        &[annot(&[rect(line, 14, 18)], "https://e.org/2", false, "")],
        "",
    );
    let d = parse(b, &ParseOptions::default());
    assert_eq!(
        links(&d),
        vec![("here".to_string(), "https://e.org/2".to_string(), 14)]
    );
}

#[test]
fn rejected_links_are_reported_as_info() {
    let b = pdf(
        &[P1],
        "",
        &[annot(&[rect(P1, 18, 22)], "javascript:x()", false, "")],
        "",
    );
    let d = parse(b, &ParseOptions::default());
    let w: Vec<_> = d.sections[0]
        .warnings
        .iter()
        .filter(|w| w.code == Some("link"))
        .map(|w| (w.kind, w.level))
        .collect();
    assert_eq!(w, vec![(WarningKind::ContentDropped, Level::Info)]);
    assert!(!d.to_markdown().contains("javascript"));
}

#[test]
fn hidden_text_under_a_link_gets_no_link() {
    // a sub-point payload (`tiny`) next to the visible word, both under the link
    let tiny: Line = (72.0 + 22.0 * 7.2, 700.0, 0.8, "PWNED");
    let r = rect(P1, 18, 22);
    let b = pdf(
        &[P1, tiny],
        "",
        &[annot(
            &[[r[0], r[1], r[2] + 5.0, r[3]]],
            "https://e.org/t",
            false,
            "",
        )],
        "",
    );
    for mode in [HiddenText::Strip, HiddenText::Flag] {
        let mut opts = ParseOptions::default();
        opts.hidden_text = mode;
        let d = parse(b.clone(), &opts);
        let got = links(&d);
        assert!(
            got.iter().all(|(t, _, _)| !t.contains("PWNED")),
            "{mode:?}: {got:?}"
        );
        assert_eq!(got.len(), 1, "{mode:?}: {}", d.to_markdown());
    }
}

#[test]
fn switched_off_reads_no_annotations() {
    let b = pdf(
        &[P1],
        "",
        &[
            annot(&[rect(P1, 18, 22)], "https://e.org/s", false, ""),
            annot(&[rect(P1, 0, 5)], "javascript:x()", false, ""),
        ],
        "",
    );
    let mut off = ParseOptions::default();
    off.links = false;
    let d = parse(b.clone(), &off);
    assert!(links(&d).is_empty());
    assert!(
        d.sections[0]
            .warnings
            .iter()
            .all(|w| w.code != Some("link"))
    );
    let plain = parse(pdf(&[P1], "", &[], ""), &ParseOptions::default());
    assert_eq!(d.to_json(), plain.to_json());
}

#[test]
fn a_link_in_a_table_cell() {
    // a ruled 4 x 3 grid with one word per cell; the link covers the middle cell of row 3
    let rows = [
        (700.0, ["Name", "Site", "Year"]),
        (680.0, ["Bolt", "bolt", "2001"]),
        (660.0, ["Acme", "acme", "1999"]),
        (640.0, ["Zeta", "zeta", "2010"]),
    ];
    let mut lines: Vec<Line> = Vec::new();
    for (y, cells) in rows {
        for (i, t) in cells.into_iter().enumerate() {
            lines.push((80.0 + 100.0 * i as f32, y, 10.0, t));
        }
    }
    let mut paths = String::from("0.5 w");
    for y in [714.0, 694.0, 674.0, 654.0, 634.0] {
        paths.push_str(&format!(" 72 {y} m 372 {y} l S"));
    }
    for x in [72.0, 172.0, 272.0, 372.0] {
        paths.push_str(&format!(" {x} 634 m {x} 714 l S"));
    }
    let r = rect(lines[7], 0, 4);
    let b = pdf(
        &lines,
        &paths,
        &[annot(&[r], "https://acme.example/", false, "")],
        "",
    );
    let d = parse(b, &ParseOptions::default());
    let table = d.sections[0]
        .blocks
        .iter()
        .any(|b| matches!(b, Block::Table { .. }));
    assert!(table, "{}", d.to_markdown());
    assert_eq!(
        links(&d)
            .into_iter()
            .map(|(t, u, _)| (t, u))
            .collect::<Vec<_>>(),
        vec![("acme".to_string(), "https://acme.example/".to_string())]
    );
    assert!(
        d.to_markdown()
            .contains("| Acme | [acme](https://acme.example/) | 1999 |"),
        "{}",
        d.to_markdown()
    );
}
