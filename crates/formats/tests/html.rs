#![cfg(feature = "markup")]
mod pictures_util;
use plaindoc_core::*;
use plaindoc_formats::Registry;
use plaindoc_formats::markup::HtmlParser;

fn parse_bytes(b: &[u8]) -> Result<Document> {
    HtmlParser.parse(
        Input::Bytes(b.to_vec()),
        Format::Html,
        &ParseOptions::default(),
    )
}
fn parse(s: &str) -> Document {
    parse_bytes(s.as_bytes()).unwrap()
}
fn blocks(s: &str) -> Vec<Block> {
    parse(s).sections.remove(0).blocks
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
/// Text, colspan and rowspan of every cell.
type Grid = Vec<Vec<(String, u32, u32)>>;
fn table(b: &Block) -> (Grid, usize) {
    let Block::Table {
        rows, header_rows, ..
    } = b
    else {
        panic!("{b:?}")
    };
    let sig = rows
        .iter()
        .map(|r| r.iter().map(|c| (c.text(), c.colspan, c.rowspan)).collect())
        .collect();
    (sig, *header_rows)
}
fn s(t: &str, c: u32, r: u32) -> (String, u32, u32) {
    (t.into(), c, r)
}
fn cp1251(s: &str) -> Vec<u8> {
    encoding_rs::WINDOWS_1251.encode(s).0.into_owned()
}

#[test]
fn headings_paragraphs_inline_text_and_br() {
    let b = blocks(
        "<h1>Title</h1><p>Hello <b>bold</b>\n   world<br>next</p><div>loose <span>text</span><p>inner</p>tail</div><hr><h7>x</h7>",
    );
    assert_eq!(
        b,
        vec![
            Block::heading(1, "Title", None),
            p("Hello bold world\nnext"),
            p("loose text"),
            p("inner"),
            p("tail"),
            p("x"),
        ]
    );
}

#[test]
fn nested_list_keeps_its_own_ordered() {
    let d = parse("<ul><li>a<ol><li>x</li><li>y</li></ol></li><li>b</li><li></li></ul>");
    let Block::List {
        ordered: false,
        items,
        ..
    } = &d.sections[0].blocks[0]
    else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    assert!(
        matches!(&items[0].blocks[..], [Block::Paragraph { .. }, Block::List { ordered: true, items, .. }] if items.len() == 2)
    );
    assert_eq!(d.to_markdown(), "- a\n  1. x\n  2. y\n- b\n");
}

#[test]
fn list_directly_inside_list_joins_previous_item() {
    let b = blocks("<ol><li>a</li><ul><li>x</li></ul><li>b</li></ol>");
    let Block::List {
        ordered: true,
        items,
        ..
    } = &b[0]
    else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    assert!(matches!(
        &items[0].blocks[..],
        [_, Block::List { ordered: false, .. }]
    ));
}

#[test]
fn thead_and_spans_render_as_html_table() {
    let d = parse(
        "<table><thead><tr><th colspan=2>H</th></tr></thead><tbody><tr><td rowspan=2>a</td><td>b</td></tr><tr><td>c</td></tr></tbody></table>",
    );
    let (sig, header) = table(&d.sections[0].blocks[0]);
    assert_eq!(
        sig,
        vec![
            vec![s("H", 2, 1)],
            vec![s("a", 1, 2), s("b", 1, 1)],
            vec![s("c", 1, 1)]
        ]
    );
    assert_eq!(header, 1);
    assert!(kinds(&d).is_empty());
    let md = d.to_markdown();
    assert!(
        md.contains("<th colspan=\"2\">H</th>") && md.contains("<td rowspan=\"2\">a</td>"),
        "{md}"
    );
}

#[test]
fn leading_th_rows_are_header_and_tfoot_goes_last() {
    let b = blocks(
        "<table><tfoot><tr><td>sum</td><td>3</td></tr></tfoot><tr><th>k</th><th>v</th></tr><tr><td>a</td><td>3</td></tr></table>",
    );
    let (sig, header) = table(&b[0]);
    assert_eq!(
        sig.iter().map(|r| r[0].0.as_str()).collect::<Vec<_>>(),
        ["k", "a", "sum"]
    );
    assert_eq!(header, 1);
}

#[test]
fn simple_table_is_gfm() {
    let d =
        parse("<table><tr><th>a</th><th>b</th></tr><tr><td>1 | 2</td><td>x<br>y</td></tr></table>");
    assert_eq!(
        d.to_markdown(),
        "| a | b |\n| --- | --- |\n| 1 \\| 2 | x<br>y |\n"
    );
}

#[test]
fn rowspan_zero_and_bad_colspan_are_normalized() {
    let d = parse(
        "<table><tr><td rowspan=0>a</td><td colspan=0>b</td></tr><tr><td>c</td></tr><tr><td>d</td></tr></table>",
    );
    let (sig, _) = table(&d.sections[0].blocks[0]);
    assert_eq!(sig[0], vec![s("a", 1, 3), s("b", 1, 1)]);
    assert!(kinds(&d).is_empty(), "{:?}", kinds(&d));
    // leading digits count (M3); a rowspan stays inside its row group (M2)
    let d = parse(
        "<table><thead><tr><td colspan=\"2px\" rowspan=5>h</td></tr></thead><tbody><tr><td>x</td><td>y</td></tr></tbody></table>",
    );
    let (sig, _) = table(&d.sections[0].blocks[0]);
    assert_eq!(sig[0], vec![s("h", 2, 1)]);
    let d = parse("<table><tr><td colspan=99999>a</td></tr><tr><td>b</td></tr></table>");
    let (sig, _) = table(&d.sections[0].blocks[0]);
    assert_eq!(sig[0], vec![s("a", 1000, 1)]);
    assert_eq!(kinds(&d), vec![(WarningKind::TableUncertain, Level::Warn)]);
}

#[test]
fn nested_table_in_cell_keeps_order() {
    let b = blocks(
        "<table><tr><td>E<table><tr><td>1</td><td>2</td></tr></table>F</td><td>x</td></tr><tr><td>y</td><td>z</td></tr></table>",
    );
    let Block::Table { rows, .. } = &b[0] else {
        panic!()
    };
    assert!(
        matches!(&rows[0][0].blocks[..], [Block::Paragraph { text: e, .. }, Block::Table { .. }, Block::Paragraph { text: f, .. }] if e == "E" && f == "F")
    );
}

#[test]
fn one_cell_layout_table_is_unwrapped() {
    assert_eq!(
        blocks("<table><tr><td><h2>T</h2><p>x</p></td></tr></table>"),
        vec![Block::heading(2, "T", None), p("x")]
    );
}

#[test]
fn pre_keeps_whitespace_and_language() {
    let b = blocks(
        "<pre><code class=\"language-rust\">fn main() {\n    x &lt; 1;\n}</code></pre><pre>\n  plain\n</pre><pre>\n\nx</pre>",
    );
    assert_eq!(
        b,
        vec![
            Block::Code {
                lang: Some("rust".into()),
                text: "fn main() {\n    x < 1;\n}\n".into(),
                bbox: None
            },
            Block::Code {
                lang: None,
                text: "  plain\n".into(),
                bbox: None
            },
            // html5ever drops only the first newline after <pre> (M1)
            Block::Code {
                lang: None,
                text: "\nx\n".into(),
                bbox: None
            },
        ]
    );
}

#[test]
fn scripts_styles_templates_and_hidden_are_skipped() {
    let d = parse(
        "<style>p{}</style><script>alert(1)</script><template><p>t</p></template><noscript><b>ns</b></noscript><p hidden>h</p><div style=\"DISPLAY : none\">d</div><select><option>o</select><p>ok</p>",
    );
    assert_eq!(d.sections[0].blocks, vec![p("ok")]);
    assert_eq!(kinds(&d), vec![(WarningKind::HiddenContent, Level::Info)]);
    assert_eq!(
        d.sections[0].warnings[0].message,
        "html: 2 hidden elements skipped"
    );
}

#[test]
fn entities_and_nbsp() {
    assert_eq!(
        blocks("<p>a&nbsp;&amp;&lt;b&gt; &#1046; &copy;</p>"),
        vec![p("a &<b> Ж ©")]
    );
}

#[test]
fn title_author_and_images() {
    let d = parse(
        "<html><head><title> Report </title><meta name=Author content=\"Alex\"></head><body><img src=a.png alt=\" Logo \"><p>t<img src=b.png>u</p><table><tr><td><img alt=icon>x</td><td>y</td></tr><tr><td>1</td><td>2</td></tr></table></body></html>",
    );
    assert_eq!(d.metadata.title.as_deref(), Some("Report"));
    assert_eq!(d.metadata.author.as_deref(), Some("Alex"));
    let b = &d.sections[0].blocks;
    assert_eq!(
        b[0],
        Block::Image {
            alt: Some("Logo".into()),
            ocr_text: None,
            bbox: None,
            image: Some(Box::new(ImageInfo::external("a.png"))),
        }
    );
    // an inline image goes after its paragraph (as in odt and rtf)
    assert_eq!(b[1], p("tu"));
    assert_eq!(
        b[2],
        Block::Image {
            alt: None,
            ocr_text: None,
            bbox: None,
            image: Some(Box::new(ImageInfo::external("b.png"))),
        }
    );
    let (sig, _) = table(&b[3]);
    assert_eq!(sig[0][0].0, "x");
    assert_eq!(kinds(&d), vec![(WarningKind::ContentDropped, Level::Info)]);
    assert_eq!(
        d.sections[0].warnings[0].message,
        "html: 1 images in table cells dropped"
    );
}

#[test]
fn html_data_uri_is_stored_links_stay_links() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(pictures_util::RED);
    let html = format!(
        r#"<html><body><p>a</p><img src="data:image/png;base64,{b64}" alt="red"><img src="https://e.org/p.png"><img src="file:///etc/x.png"><img src="data:image/png;base64,{b64}"></body></html>"#
    );
    let d = Registry::with_defaults()
        .parse(
            Input::Bytes(html.clone().into_bytes()),
            &ParseOptions::default(),
        )
        .unwrap();
    assert_eq!(
        pictures_util::files(&d),
        [
            Some("image1.png".into()),
            None,
            None,
            Some("image1.png".into())
        ]
    );
    assert_eq!(pictures_util::stored(&d, "image1.png"), pictures_util::RED);
    let md = d.to_markdown();
    assert!(!md.contains("base64"), "{md}");
    assert!(
        md.contains("a\n\n![red](image1.png)\n\n![изображение](https://e.org/p.png)\n\n<!-- image -->\n\n![изображение 1](image1.png)"),
        "{md}"
    );
    let mut off = ParseOptions::default();
    off.images.enabled = false;
    let d = Registry::with_defaults()
        .parse(Input::Bytes(html.into_bytes()), &off)
        .unwrap();
    assert!(d.images.is_empty() && pictures_util::files(&d).iter().all(Option::is_none));
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
}

#[test]
fn cp1251_by_meta_charset_and_by_detection() {
    let body = "Привет, мир! Это проверка кодировки windows-1251.";
    for head in [
        "<meta charset=windows-1251>",
        "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=windows-1251\">",
        "",
    ] {
        let mut bytes = format!("<html><head>{head}</head><body><p>").into_bytes();
        bytes.extend(cp1251(body));
        bytes.extend(b"</p></body></html>");
        let d = parse_bytes(&bytes).unwrap();
        assert_eq!(d.sections[0].blocks, vec![p(body)], "{head}");
        assert!(kinds(&d).is_empty(), "{head}: {:?}", kinds(&d));
    }
}

#[test]
fn declared_utf8_with_cp1251_bytes_falls_back_to_detection() {
    let body = "Сумма к оплате по счёту составляет сто рублей.";
    let mut bytes = b"<meta charset=utf-8><p>".to_vec();
    bytes.extend(cp1251(body));
    assert_eq!(
        parse_bytes(&bytes).unwrap().sections[0].blocks,
        vec![p(body)]
    );
}

#[test]
fn html_saved_as_xls_by_1c_through_registry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report.xls");
    let mut bytes = b"<HTML><HEAD><META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=windows-1251\"></HEAD><BODY><TABLE BORDER=1><TR><TD>".to_vec();
    bytes.extend(cp1251("Номер<TD>Сумма<TR><TD>1<TD>10,5</TABLE>"));
    std::fs::write(&path, bytes).unwrap();
    let d = Registry::with_defaults()
        .parse(Input::Path(path), &ParseOptions::default())
        .unwrap();
    assert_eq!(d.metadata.format, Some(Format::Html));
    assert_eq!(d.quality, 1.0);
    assert_eq!(
        d.to_markdown(),
        "|  |  |\n| --- | --- |\n| Номер | Сумма |\n| 1 | 10,5 |\n"
    );
}

#[test]
fn large_unclosed_table_is_accepted() {
    let html = format!(
        "<TABLE>{}</TABLE>",
        (0..50_000)
            .map(|i| format!("<TR><TD><FONT>{i}<TD>x"))
            .collect::<String>()
    );
    let (sig, _) = table(&blocks(&html)[0]);
    assert_eq!(sig.len(), 50_000);
    assert_eq!(sig[49_999][0].0, "49999");
}

#[test]
fn deep_tower_is_a_limit_error_and_fast() {
    let t = std::time::Instant::now();
    let err = parse_bytes("<div>".repeat(200_000).as_bytes()).unwrap_err();
    assert!(
        t.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        t.elapsed()
    );
    assert!(
        matches!(&err, ParseError::Limit { kind, limit: 16_384 } if kind == "html nesting depth"),
        "{err:?}"
    );
}

#[test]
fn nesting_below_guard_is_flattened_on_a_small_stack() {
    let html = format!("{}deep", "<div><ul><li>".repeat(1000));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .stack_size(2 << 20)
        .build()
        .unwrap();
    let d = pool.install(|| parse(&html));
    assert!(kinds(&d).contains(&(WarningKind::ContentDropped, Level::Info)));
    assert!(d.to_markdown().contains("deep"));
}

#[test]
fn empty_html_is_empty_output() {
    let d = Registry::with_defaults()
        .parse(
            Input::Bytes(b"<html><body> </body></html>".to_vec()),
            &ParseOptions::default(),
        )
        .unwrap();
    assert_eq!(kinds(&d), vec![(WarningKind::EmptyOutput, Level::Warn)]);
    assert_eq!(d.quality, 0.9);
}

#[test]
fn too_large_html_is_rejected_before_parsing() {
    let mut o = ParseOptions::default();
    o.max_text_bytes = 10;
    let r = HtmlParser.parse(
        Input::Bytes(b"<p>more than ten</p>".to_vec()),
        Format::Html,
        &o,
    );
    assert!(matches!(r, Err(ParseError::TooLarge { limit: 10, .. })));
}

#[test]
fn parallel_parse_matches_sequential() {
    let docs: Vec<Vec<u8>> = (0..16)
        .map(|i| format!("<h1>{i}</h1><table><tr><td rowspan=2>a</td><td>b</td></tr><tr><td>{i}</td></tr></table><ul><li>x<ol><li>y</li></ol></li></ul>").into_bytes())
        .collect();
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
fn nested_lists_are_bounded_on_a_small_stack() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .stack_size(2 << 20)
        .build()
        .unwrap();
    for html in [
        format!("{}x", "<ul><li>".repeat(4_000)),
        format!("{}<li>x", "<ul>".repeat(4_000)),
    ] {
        let d = pool.install(|| parse(&html));
        assert!(kinds(&d).contains(&(WarningKind::ContentDropped, Level::Info)));
        assert!(d.to_markdown().contains('x'));
    }
}

#[test]
fn quadratic_tree_builder_work_is_a_limit_error_and_fast() {
    // shapes the nesting guard lets through (R09): optgroups outside select,
    // pops stopped at SVG integration points, and full stack scans by `</p>`
    for html in [
        "<optgroup><option>".repeat(16_000),
        format!(
            "{}{}",
            "<div><svg><desc></div>".repeat(20_000),
            "</template>".repeat(10_000)
        ),
        format!("{}{}", "<div>".repeat(4_000), "</p>".repeat(100_000)),
    ] {
        let t = std::time::Instant::now();
        let err = parse_bytes(html.as_bytes()).unwrap_err();
        let bound = if cfg!(debug_assertions) { 20 } else { 2 };
        assert!(
            t.elapsed() < std::time::Duration::from_secs(bound),
            "{:?}",
            t.elapsed()
        );
        let cap = 130_000_000 + 32 * html.len() as u64;
        assert!(
            matches!(&err, ParseError::Limit { kind, limit } if kind == "html parser work" && *limit == cap),
            "{err:?}"
        );
    }
}

#[test]
fn formatting_scans_and_attribute_floods_are_limit_errors_and_fast() {
    let attrs: String = (0..20).map(|j| format!(" a{j}")).collect();
    let flood = format!(
        "<p{}>x",
        (0..200_000).map(|i| format!(" a{i}")).collect::<String>()
    );
    for (html, want) in [
        (
            (0..8_000)
                .map(|i| format!("<b id={i}{attrs}>"))
                .collect::<String>(),
            "html formatting elements",
        ),
        (flood, "html attributes per element"),
    ] {
        let t = std::time::Instant::now();
        let err = parse_bytes(html.as_bytes()).unwrap_err();
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
        assert!(
            matches!(&err, ParseError::Limit { kind, .. } if kind == want),
            "{err:?}"
        );
    }
}

#[test]
fn scripts_with_inline_data_are_not_attributes() {
    let points: String = (0..5_000).map(|i| format!("[{i}, {i}], ")).collect();
    let html = format!(
        "<html><head><script>if (window.innerWidth<width) {{ draw(); }} var points = [ {points} ];</script></head><body><p>chart</p><!-- i<n {points} --></body></html>"
    );
    assert!(parse(&html).to_markdown().contains("chart"));
}
