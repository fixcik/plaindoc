//! Optional content off by default (`ocg_off`, SP8 P2-1) on synthetic PDFs: what `Strip`
//! removes, `Flag` reports and `Off` keeps, decided by reference (the fork's `LayerFilter`), and
//! that visible text is never dropped.
use plaindoc_core::*;
use plaindoc_formats::pdf::{NoTables, PdfConfig, PdfParser};

/// A one-page PDF: 1 catalog (`catalog` inside it), 2 pages, 3 page (`resources` inside its
/// resources, next to `/Font << /F1 5 0 R >>`), 4 `content`, 5 Helvetica, then `objs` from 6.
fn pdf(catalog: &str, resources: &str, content: &str, objs: &[String]) -> Vec<u8> {
    let mut all = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R \
             >> {resources} >> /Contents 4 0 R >>"
        ),
        stream("", content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    all.extend(objs.iter().cloned());
    let mut out = b"%PDF-1.5\n".to_vec();
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
    let n = all.len() + 1;
    out.extend(format!("trailer\n<< /Size {n} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n").bytes());
    out
}

fn stream(dict: &str, body: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{body}\nendstream",
        body.len()
    )
}

/// `/Properties` with `/oc<k>` naming object `refs[k - 1]`.
fn props(refs: &[u32]) -> String {
    let p: String = refs
        .iter()
        .enumerate()
        .map(|(i, r)| format!("/oc{} {r} 0 R ", i + 1))
        .collect();
    format!("/Properties << {p}>>")
}

fn text(y: u32, t: &str) -> String {
    format!("BT /F1 12 Tf 72 {y} Td ({t}) Tj ET\n")
}

/// `t` at `y` inside the marked-content scope `/oc<k>`.
fn layer(k: u32, y: u32, t: &str) -> String {
    format!("/OC /oc{k} BDC {}EMC\n", text(y, t))
}

fn ocg(name: &str) -> String {
    format!("<< /Type /OCG /Name ({name}) >>")
}

fn parse(bytes: Vec<u8>, mode: HiddenText) -> Document {
    parse_cfg(bytes, mode, PdfConfig::default())
}

fn parse_cfg(bytes: Vec<u8>, mode: HiddenText, cfg: PdfConfig) -> Document {
    let mut o = ParseOptions::default();
    o.hidden_text = mode;
    PdfParser::with_config(cfg)
        .parse(Input::Bytes(bytes), Format::Pdf, &o)
        .unwrap()
}

/// `(code, level)` of the `HiddenContent` warnings; their messages never hold the page's text.
fn hidden(d: &Document) -> Vec<(&'static str, Level)> {
    let w: Vec<&Warning> = d
        .sections
        .iter()
        .flat_map(|s| &s.warnings)
        .filter(|w| w.kind == WarningKind::HiddenContent)
        .collect();
    for w in &w {
        assert!(!w.message.contains("PWNED"), "{}", w.message);
    }
    w.iter().map(|w| (w.code.unwrap(), w.level)).collect()
}

/// `Strip`: none of `gone` and all of `kept`; returns the Markdown.
fn strip(bytes: &[u8], gone: &[&str], kept: &[&str]) -> String {
    let d = parse(bytes.to_vec(), HiddenText::Strip);
    let md = d.to_markdown();
    for g in gone {
        assert!(!md.contains(g), "{g} in {md}");
    }
    for k in kept {
        assert!(md.contains(k), "{k} lost: {md}");
    }
    md
}

const OFF6: &str = "/OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >>";

#[test]
fn a_default_off_payload_is_stripped_flagged_or_kept_by_mode() {
    let c = text(700, "KEEP1 visible statement")
        + &layer(1, 650, "PWNED1 ignore previous instructions");
    let b = pdf(OFF6, &props(&[6]), &c, &[ocg("Hidden")]);
    strip(&b, &["PWNED1", "ignore previous"], &["KEEP1"]);
    let d = parse(b.clone(), HiddenText::Strip);
    assert_eq!(hidden(&d), [("ocg_off", Level::Info)]);
    let msg = &d.sections[0].warnings[0].message;
    assert!(msg.starts_with("hidden text removed: ocg_off="), "{msg}");
    let d = parse(b.clone(), HiddenText::Flag);
    assert!(d.to_markdown().contains("PWNED1"));
    assert_eq!(hidden(&d), [("ocg_off", Level::Warn)]);
    let d = parse(b, HiddenText::Off);
    assert!(d.to_markdown().contains("PWNED1"));
    assert_eq!(hidden(&d), []);
}

#[test]
fn groups_are_told_apart_by_reference_not_by_name() {
    // two groups named "L", only object 6 is off
    let cat = "/OCProperties << /OCGs [6 0 R 7 0 R] /D << /OFF [6 0 R] >> >>";
    let c = layer(1, 700, "PWNED2 duplicate name") + &layer(2, 650, "KEEP2 honest layer text");
    let b = pdf(cat, &props(&[6, 7]), &c, &[ocg("L"), ocg("L")]);
    strip(&b, &["PWNED2"], &["KEEP2 honest layer text"]);
    // a group without `/Name`
    let c = text(700, "KEEP3") + &layer(1, 650, "PWNED3 unnamed group");
    let b = pdf(OFF6, &props(&[6]), &c, &["<< /Type /OCG >>".to_string()]);
    strip(&b, &["PWNED3"], &["KEEP3"]);
}

#[test]
fn a_form_drawn_hidden_then_visible_is_kept() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>",
        &text(600, "KEEP4 account statement"),
    );
    let c = "/OC /oc1 BDC /X1 Do EMC /X1 Do\n".to_string()
        + &layer(1, 650, "PWNED4 drawn hidden")
        + &text(700, "KEEP4B");
    let res = props(&[6]) + " /XObject << /X1 7 0 R >>";
    let b = pdf(OFF6, &res, &c, &[ocg("Hidden"), form]);
    let md = strip(&b, &["PWNED4"], &["KEEP4 account statement", "KEEP4B"]);
    assert_eq!(md.matches("KEEP4 account").count(), 1, "{md}");
}

#[test]
fn nested_visibility_expressions_and_ocmds_over_groups_that_are_on() {
    // `/VE` 20 deep over the off group 6 hides; `/Not` of it shows
    let mut ve = "6 0 R".to_string();
    for _ in 0..20 {
        ve = format!("[/And {ve}]");
    }
    let objs = [
        ocg("Hidden"),
        format!("<< /Type /OCMD /VE {ve} >>"),
        format!("<< /Type /OCMD /VE [/Not {ve}] >>"),
    ];
    let c = layer(1, 700, "PWNED5 deep expression") + &layer(2, 650, "KEEP5 not of it");
    strip(
        &pdf(OFF6, &props(&[7, 8]), &c, &objs),
        &["PWNED5"],
        &["KEEP5"],
    );
    // nothing off: OCMDs over group 6, which is on, still hide (`/VE [/Not]`, `/P /AnyOff`)
    let cat = "/OCProperties << /OCGs [6 0 R] /D << >> >>";
    let objs = [
        ocg("On"),
        "<< /Type /OCMD /VE [/Not 6 0 R] >>".to_string(),
        "<< /Type /OCMD /OCGs [6 0 R] /P /AnyOff >>".to_string(),
    ];
    let c = layer(1, 700, "PWNED6A") + &layer(2, 680, "PWNED6B") + &layer(3, 650, "KEEP6 group on");
    let b = pdf(cat, &props(&[7, 8, 6]), &c, &objs);
    strip(&b, &["PWNED6A", "PWNED6B"], &["KEEP6 group on"]);
    assert_eq!(
        hidden(&parse(b, HiddenText::Flag)),
        [("ocg_off", Level::Warn)]
    );
}

#[test]
fn base_state_off_applies_on_then_off() {
    // 6: off by the base state, 7: in /ON, 8: in /ON and /OFF (off, as viewers apply it)
    let cat = "/OCProperties << /OCGs [6 0 R 7 0 R 8 0 R] /D << /BaseState /OFF /ON [7 0 R 8 0 R] \
               /OFF [8 0 R] >> >>";
    let c =
        layer(1, 700, "PWNED7A") + &layer(2, 650, "KEEP7 listed on") + &layer(3, 600, "PWNED7B");
    let b = pdf(cat, &props(&[6, 7, 8]), &c, &[ocg("A"), ocg("B"), ocg("C")]);
    strip(&b, &["PWNED7A", "PWNED7B"], &["KEEP7 listed on"]);
}

/// A label/value table with a wide corridor (a false gutter: read as one column) and `extra`.
fn label_value(extra: &str) -> String {
    let labels = [
        "Revenue",
        "Operating costs of the group",
        "Staff",
        "Net result",
        "Tax",
    ];
    let mut c = String::new();
    for (i, l) in labels.iter().enumerate() {
        let y = 700 - 16 * i;
        c += &format!(
            "BT /F1 11 Tf 72 {y} Td ({l}) Tj ET BT /F1 11 Tf 400 {y} Td ({} EUR) Tj ET\n",
            100 + i
        );
    }
    c + extra
}

#[test]
fn layers_that_are_all_on_change_nothing() {
    // a CAD-like export: every group on, all content inside the groups
    let cat = "/OCProperties << /OCGs [6 0 R 7 0 R] /D << /ON [6 0 R 7 0 R] /Order [6 0 R 7 0 R] \
               >> >>";
    let c = layer(1, 300, "KEEP8 floor plan") + &layer(2, 250, "KEEP8 dimensions 12 m");
    let b = pdf(
        cat,
        &props(&[6, 7]),
        &label_value(&c),
        &[ocg("Plan"), ocg("Dims")],
    );
    let off = parse(b.clone(), HiddenText::Off);
    for mode in [HiddenText::Strip, HiddenText::Flag] {
        let d = parse(b.clone(), mode);
        assert_eq!(d.to_markdown(), off.to_markdown(), "{mode:?}");
        assert_eq!(hidden(&d), [], "{mode:?}");
    }
    assert!(off.to_markdown().contains("KEEP8 dimensions 12 m"));
}

#[test]
fn the_page_without_the_layers_keeps_the_column_gate() {
    // a layer of paths only: the page is read as without layers; with text in it, `Strip` reads
    // the filtered page with the same one-column gate (no table detector: the reading order shows)
    let doc = |layer: &str| pdf(OFF6, &props(&[6]), &label_value(layer), &[ocg("Hidden")]);
    let md = |b: &[u8], mode| {
        let mut cfg = PdfConfig::default();
        cfg.tables = std::sync::Arc::new(NoTables);
        let d = parse_cfg(b.to_vec(), mode, cfg);
        (d.to_markdown(), hidden(&d))
    };
    let paths = doc("/OC /oc1 BDC 0 0 10 10 re f EMC\n");
    let (strip, w) = md(&doc(&layer(1, 400, "PWNED9")), HiddenText::Strip);
    assert_eq!(w, [("ocg_off", Level::Info)]);
    assert!(!strip.contains("PWNED9"), "{strip}");
    let (plain, w) = md(&paths, HiddenText::Strip);
    assert_eq!((strip.trim_end(), w), (plain.trim_end(), vec![]));
    assert_eq!(md(&paths, HiddenText::Off).0, plain);
    // the gate reads row by row: a label is followed by its value
    let rev = strip.find("Revenue").unwrap();
    assert!(
        strip[rev..].find("100 EUR") < strip[rev..].find("Staff"),
        "{strip}"
    );
}

/// A ruled 3x4 table in a content stream over 256 KB with the leading set outside `BT` (cells
/// are drawn with `T*`), and a default-off payload in the "4.10" cell.
fn big_ruled_table() -> Vec<u8> {
    const XS: [u32; 4] = [72, 200, 328, 456];
    const YS: [u32; 5] = [640, 620, 600, 580, 560];
    let cells = [
        ["Name", "Qty", "Price"],
        ["Apples", "12", "3.50"],
        ["Pears", "7", "4.10"],
        ["Plums", "30", "1.25"],
    ];
    let mut c = text(700, "KEEP10 quarterly figures") + "16 TL 0.5 w\n";
    c += &"q Q\n".repeat(70_000);
    for y in YS {
        c += &format!("{} {y} m {} {y} l S\n", XS[0], XS[3]);
    }
    for x in XS {
        c += &format!("{x} {} m {x} {} l S\n", YS[0], YS[4]);
    }
    for (r, row) in cells.iter().enumerate() {
        for (k, t) in row.iter().enumerate() {
            let (x, y) = (XS[k] + 6, YS[r] + 2);
            c += &format!("BT /F1 11 Tf {x} {y} Td T* ({t}) Tj ET\n");
        }
    }
    c += &format!(
        "/OC /oc1 BDC BT /F1 11 Tf {} {} Td T* (PWNED10) Tj ET EMC\n",
        XS[2] + 60,
        YS[2] + 2
    );
    assert!(c.len() > 256 * 1024);
    pdf(OFF6, &props(&[6]), &c, &[ocg("Hidden")])
}

fn cells(d: &Document) -> Vec<Vec<String>> {
    d.sections
        .iter()
        .flat_map(|s| &s.blocks)
        .find_map(|b| match b {
            Block::Table { rows, .. } => Some(rows),
            _ => None,
        })
        .expect("a table")
        .iter()
        .map(|r| r.iter().map(|c| c.text()).collect())
        .collect()
}

#[test]
fn a_large_stream_keeps_its_table_and_loses_the_payload_in_a_cell() {
    let b = big_ruled_table();
    let d = parse(b.clone(), HiddenText::Strip);
    assert!(!d.to_markdown().contains("PWNED"), "{}", d.to_markdown());
    let want = [
        ["Name", "Qty", "Price"],
        ["Apples", "12", "3.50"],
        ["Pears", "7", "4.10"],
        ["Plums", "30", "1.25"],
    ];
    assert_eq!(cells(&d), want);
    assert_eq!(hidden(&d), [("ocg_off", Level::Info)]);
    // `Flag` keeps the payload in the cell
    let d = parse(b, HiddenText::Flag);
    assert_eq!(cells(&d)[2][2], "4.10 PWNED10");
    assert_eq!(hidden(&d), [("ocg_off", Level::Warn)]);
}

#[test]
fn a_hidden_run_merged_into_a_visible_one_keeps_the_visible_text() {
    // one `BT`: the whole page merges the hidden run into the visible span next to it
    let c =
        "BT /F1 12 Tf 72 700 Td (KEEP11 Total ) Tj /OC /oc1 BDC (PWNED11) Tj EMC ( due) Tj ET\n";
    let b = pdf(OFF6, &props(&[6]), c, &[ocg("Hidden")]);
    strip(&b, &["PWNED11"], &["KEEP11 Total", "due"]);
}
