//! Hidden text on synthetic PDFs: colour spaces and ruled tables, where the table words get the
//! same filter as the spans; the golden set is in `pdf_hidden.rs`.
use plaindoc_core::*;
use plaindoc_formats::pdf::{PdfConfig, PdfParser};

/// A one-page PDF from objects 3.. (object 3 is the page, `/Parent 2 0 R`).
fn pdf(objs: &[String]) -> Vec<u8> {
    let mut all = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
    ];
    all.extend(objs.iter().cloned());
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in all.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let x = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", all.len() + 1).bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
            all.len() + 1
        )
        .bytes(),
    );
    out
}

fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

#[test]
fn black_separation_ink_is_not_low_contrast() {
    // `/CS0 cs 1 scn` in `[/Separation /Black ...]` is full black ink (the span's `color` reads
    // it as grey 1, white); the resolved fill colour is black, on the page and inside a form
    let res = "/Font << /F1 4 0 R >> /ColorSpace << /CS0 [/Separation /Black /DeviceGray 5 0 R] >>";
    let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string();
    let tint = "<< /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>".to_string();
    let text = "/CS0 cs 1 scn BT /F1 12 Tf 72 700 Td (KEEPSEP black body text) Tj ET";
    let page = |res: &str| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << {res} >> \
             /Contents 6 0 R >>"
        )
    };
    let direct = pdf(&[page(res), font.clone(), tint.clone(), stream("", text)]);
    let form = pdf(&[
        page("/XObject << /Fm0 7 0 R >>"),
        font,
        tint,
        stream("", "/Fm0 Do"),
        stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << {res} >>"),
            text,
        ),
    ]);
    for bytes in [direct, form] {
        for mode in [HiddenText::Strip, HiddenText::Flag] {
            let mut o = ParseOptions::default();
            o.hidden_text = mode;
            let mut c = PdfConfig::default();
            c.visibility_facts = true;
            let d = PdfParser::with_config(c)
                .parse(Input::Bytes(bytes.clone()), Format::Pdf, &o)
                .unwrap();
            assert!(d.to_markdown().contains("KEEPSEP"), "{mode:?}");
            let w: Vec<_> = d.sections[0]
                .warnings
                .iter()
                .filter(|w| w.kind == WarningKind::HiddenContent)
                .map(|w| (w.code, w.level))
                .collect();
            assert_eq!(w, [], "{mode:?}");
        }
    }
}

/// A ruled 3×4 table (`DocumentBuilder`) with a 0.5 pt payload next to a cell value: the table
/// detector reads `extract_words`, not the filtered spans, so the words get the filter too.
fn ruled_table_with_tiny_payload() -> Vec<u8> {
    const XS: [f32; 4] = [72.0, 200.0, 328.0, 456.0];
    const YS: [f32; 5] = [640.0, 620.0, 600.0, 580.0, 560.0];
    let cells = [
        ["Name", "Qty", "Price"],
        ["Apples", "12", "3.50"],
        ["Pears", "7", "4.10"],
        ["Plums", "30", "1.25"],
    ];
    let mut b = pdf_oxide::writer::DocumentBuilder::new();
    let mut p = b.letter_page().font("Helvetica", 11.0);
    p = p
        .at(72.0, 700.0)
        .text("Quarterly figures are listed below.");
    for y in YS {
        p = p.line(XS[0], y, XS[3], y);
    }
    for x in XS {
        p = p.line(x, YS[0], x, YS[4]);
    }
    for (r, row) in cells.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            p = p.at(XS[c] + 6.0, YS[r] - 14.0).text(t);
        }
    }
    p = p
        .font("Helvetica", 0.5)
        .at(XS[2] + 60.0, YS[2] - 14.0)
        .text("PWNED-T");
    p.done();
    b.build().unwrap()
}

#[test]
fn payload_inside_ruled_table_is_stripped() {
    let parse = |mode| {
        let mut o = ParseOptions::default();
        o.hidden_text = mode;
        PdfParser::new()
            .parse(
                Input::Bytes(ruled_table_with_tiny_payload()),
                Format::Pdf,
                &o,
            )
            .unwrap()
    };
    let cells = |d: &Document| -> Vec<String> {
        d.sections
            .iter()
            .flat_map(|s| &s.blocks)
            .filter_map(|b| match b {
                Block::Table { rows, .. } => Some(rows),
                _ => None,
            })
            .flatten()
            .flatten()
            .map(|c| c.text())
            .collect()
    };
    let off = parse(HiddenText::Off);
    assert!(
        cells(&off).iter().any(|c| c.contains("PWNED-T")),
        "{:?}",
        cells(&off)
    );
    let strip = parse(HiddenText::Strip);
    let c = cells(&strip);
    assert!(c.iter().any(|c| c.contains("4.10")), "{c:?}");
    assert!(!c.iter().any(|c| c.contains("PWNED")), "{c:?}");
    assert!(!strip.to_markdown().contains("PWNED"));
    // spans and words see the same 7 chars: counted once
    let w: Vec<&str> = strip.sections[0]
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::HiddenContent)
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(w, ["hidden text removed: tiny=7 chars (1 spans)"]);
}

/// The ruled table of [`ruled_table_with_tiny_payload`] as a raw content stream; `price` is the
/// content of the "4.10" cell (after `/F1 11 Tf x y Td`) and `extra` is drawn last.
/// Resources: `/F1` Helvetica (object 5), `/F2` Helvetica with a ToUnicode map `<01>` → U+202E,
/// `<0B>` → U+200B (objects 6, 7), `/GS0` with `ca 0` (object 8).
fn raw_table(price: &str, extra: &str, facts: bool) -> Document {
    const XS: [u32; 4] = [72, 200, 328, 456];
    const YS: [u32; 5] = [640, 620, 600, 580, 560];
    let cells = [
        ["Name", "Qty", "Price"],
        ["Apples", "12", "3.50"],
        ["Pears", "7", "4.10"],
        ["Plums", "30", "1.25"],
    ];
    let mut c =
        String::from("BT /F1 11 Tf 72 700 Td (Quarterly figures are listed below.) Tj ET\n0.5 w\n");
    for y in YS {
        c += &format!("{} {y} m {} {y} l S\n", XS[0], XS[3]);
    }
    for x in XS {
        c += &format!("{x} {} m {x} {} l S\n", YS[0], YS[4]);
    }
    for (r, row) in cells.iter().enumerate() {
        for (k, t) in row.iter().enumerate() {
            let body = if *t == "4.10" {
                price.to_string()
            } else {
                format!("({t}) Tj")
            };
            c += &format!("BT /F1 11 Tf {} {} Td {body} ET\n", XS[k] + 6, YS[r] - 14);
        }
    }
    c += extra;
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /X def \
                /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange \
                2 beginbfchar <01> <202E> <0B> <200B> endbfchar 1 beginbfrange <20> <7E> <0020> endbfrange \
                endcmap CMapName currentdict /CMap defineresource pop end end";
    let bytes = pdf(&[
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R \
         /F2 6 0 R >> /ExtGState << /GS0 8 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        stream("", &c),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 7 0 R >>".to_string(),
        stream("", cmap),
        "<< /Type /ExtGState /ca 0 >>".to_string(),
    ]);
    let mut o = ParseOptions::default();
    o.hidden_text = HiddenText::Strip;
    let mut cfg = PdfConfig::default();
    cfg.visibility_facts = facts;
    PdfParser::with_config(cfg)
        .parse(Input::Bytes(bytes), Format::Pdf, &o)
        .unwrap()
}

/// `(text of the "4.10" cell, HiddenContent messages)`.
fn price_cell(d: &Document) -> (String, Vec<String>) {
    let cell = d
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .find_map(|b| match b {
            Block::Table { rows, .. } => Some(rows[2][2].text()),
            _ => None,
        })
        .expect("a table");
    let w = d.sections[0]
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::HiddenContent)
        .map(|w| w.message.clone())
        .collect();
    (cell, w)
}

#[test]
fn payload_glued_to_a_cell_value_is_cut() {
    // no gap between "4.10" and the 0.5 pt payload: `extract_words` gives one word
    let d = raw_table("(4.10) Tj /F1 0.5 Tf (PWNEDGLUE) Tj", "", false);
    let (cell, w) = price_cell(&d);
    assert_eq!(cell, "4.10");
    assert!(!d.to_markdown().contains("PWNED"));
    assert_eq!(w, ["hidden text removed: tiny=9 chars (1 spans)"]);
}

#[test]
fn multi_word_rlo_payload_in_a_cell_is_stripped() {
    // one span "4.10 \u{202E}DENWP LLAC EMOH": the span is removed, so are all its words
    let d = raw_table("(4.10 ) Tj /F2 11 Tf (\\001DENWP LLAC EMOH) Tj", "", false);
    let (cell, _) = price_cell(&d);
    let md = d.to_markdown();
    assert!(!md.contains("LLAC") && !md.contains("DENWP"), "{md}");
    assert_eq!(cell, "4.10");
}

#[test]
fn transparent_payload_in_a_cell_is_stripped_with_facts() {
    let extra = "q /GS0 gs BT /F1 11 Tf 400 586 Td (PWNEDALPHA) Tj ET Q\n";
    let d = raw_table("(4.10) Tj", extra, true);
    let (cell, w) = price_cell(&d);
    assert_eq!(cell, "4.10");
    assert!(!d.to_markdown().contains("PWNED"));
    assert_eq!(w, ["hidden text removed: alpha=10 chars (1 spans)"]);
    // without facts the cell keeps it (phase 1 has no alpha rule)
    assert!(
        price_cell(&raw_table("(4.10) Tj", extra, false))
            .0
            .contains("PWNEDALPHA")
    );
}

#[test]
fn zero_width_chars_do_not_hide_a_payload_in_a_cell() {
    // a ZWSP inside the 0.5 pt payload glued to "4.10"
    let d = raw_table("(4.10) Tj /F2 0.5 Tf (PW\\013NED) Tj", "", false);
    let (cell, w) = price_cell(&d);
    assert_eq!(cell, "4.10");
    assert!(!d.to_markdown().replace('\u{200B}', "").contains("NED"));
    assert_eq!(w, ["hidden text removed: tiny=6 chars (1 spans)"]);
    // an 11 pt zero-width interleave in the cell
    let d = raw_table(
        "(4.10 ) Tj /F2 11 Tf (P\\013W\\013N\\013E\\013D\\013X) Tj",
        "",
        false,
    );
    let (cell, w) = price_cell(&d);
    assert_eq!(cell, "4.10");
    assert!(!d.to_markdown().replace('\u{200B}', "").contains("PWNED"));
    assert_eq!(w, ["hidden text removed: uni_zw=11 chars (1 spans)"]);
}

#[test]
fn payload_glyphs_out_of_order_are_cut_from_a_cell() {
    // a TJ jump back over the payload and a negative word spacing interleave its glyphs
    for (price, msg) in [
        (
            "(4.10) Tj /F1 0.5 Tf [(PWNED) 20000 (EVIL)] TJ",
            "hidden text removed: tiny=9 chars (1 spans)",
        ),
        (
            "(4.10) Tj /F1 0.5 Tf -3 Tw (PW NED) Tj 0 Tw",
            "hidden text removed: tiny=5 chars (1 spans)",
        ),
    ] {
        let d = raw_table(price, "", false);
        let (cell, w) = price_cell(&d);
        assert_eq!(cell, "4.10", "{price}");
        assert_eq!(w, [msg]);
    }
}
