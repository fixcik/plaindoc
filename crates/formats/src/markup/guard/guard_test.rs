use super::*;

#[test]
fn closed_and_implied_end_tags_do_not_deepen() {
    assert_eq!(max_depth("<div><div><span>x</span></div></div>"), 3);
    assert_eq!(max_depth(&"<p>para".repeat(10_000)), 0);
    let table = format!("<table>{}</table>", "<tr><td>a<td>b".repeat(50_000));
    assert_eq!(max_depth(&table), 1);
    assert_eq!(max_depth("<div></span></div><div>"), 1);
    assert_eq!(max_depth("<script>if (a<b) { '<div><div>' }</script>"), 0);
    // `<div/>` is an open div in HTML, but `<g/>` in svg is closed.
    assert_eq!(max_depth(&"<div/>".repeat(100)), 100);
    assert_eq!(max_depth(&format!("<svg>{}</svg>", "<g/>".repeat(100))), 1);
    // an end tag does not pop past a cell or a special element
    assert_eq!(max_depth("<div><table><td></div><div>"), 3);
    assert_eq!(max_depth("<span><div></span><div>"), 3);
}

#[test]
fn unclosed_tags_in_implicitly_closed_cells_and_items_are_fine() {
    for (open, row, n) in [
        ("<table>", "<TR><TD><FONT>x", 50_000),
        ("<table>", "<tr><td><div>x", 5_000),
        ("<ul>", "<li><a href=x>x", 5_000),
        ("<ul>", "<li>\n<a href=x>x\n", 5_000),
        ("<dl>", "<dt><b>t<dd><i>d", 5_000),
    ] {
        let html = format!("{open}{}", row.repeat(n));
        // unclosed formatting is re-created up to Noah's Ark (3 per tag and attributes)
        assert!(max_depth(&html) <= 8, "{row}: {}", max_depth(&html));
        assert!(check_nesting(&html).is_ok(), "{row}");
    }
    // sloppy unclosed formatting without attributes: Noah's Ark keeps 3
    assert!(check_nesting(&"<p><b>x</p>".repeat(100_000)).is_ok());
}

#[test]
fn deep_tower_is_rejected_fast() {
    let t = std::time::Instant::now();
    let err = check_nesting(&"<div>".repeat(200_000)).unwrap_err();
    assert!(
        t.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        t.elapsed()
    );
    assert!(
        matches!(err, ParseError::Limit { limit: 16_384, .. }),
        "{err:?}"
    );
    assert!(check_nesting(&"<div>".repeat(16_384)).is_ok());
}

#[test]
fn formatting_reconstruction_bomb_is_rejected_fast() {
    let fmt: String = (0..3000).map(|i| format!("<b id={i}>")).collect();
    for block in [
        "<p>x</p>",
        "<div>x</div>",
        "<div> </div>",
        "<p><span></span></p>",
    ] {
        let html = format!("<p>{fmt}</p>{}", block.repeat(3000));
        let t = std::time::Instant::now();
        let err = check_nesting(&html).unwrap_err();
        assert!(
            t.elapsed() < std::time::Duration::from_secs(2),
            "{block}: {:?}",
            t.elapsed()
        );
        assert!(
            matches!(&err, ParseError::Limit { kind, limit: 1_000_000 } if kind == "html formatting elements"),
            "{block}: {err:?}"
        );
    }
    // cleared by each cell marker, but every `<b>` still scans the cell's list (R15)
    let html = format!("<table>{}</table>", format!("<tr><td>{fmt}x").repeat(300));
    let err = check_nesting(&html).unwrap_err();
    assert!(
        matches!(&err, ParseError::Limit { kind, .. } if kind == "html formatting elements"),
        "{err:?}"
    );
}

#[test]
fn reconstruction_limit_grows_with_the_input() {
    // one stray `<b>` re-created in each of 1.1M paragraphs: over 1M, under the input length
    let html = format!("<p><b>typo</p>{}", "<p>x</p>".repeat(1_100_000));
    let s = scan(&html);
    assert!(s.reconstructed > MAX_RECONSTRUCTED, "{}", s.reconstructed);
    assert!(check_nesting(&html).is_ok());
}

#[test]
fn formatting_list_scans_are_bounded() {
    let attrs: String = (0..20).map(|j| format!(" a{j}")).collect();
    for html in [
        (0..16_000)
            .map(|i| format!("<b id={i}>"))
            .collect::<String>(),
        (0..8_000)
            .map(|i| format!("<b id={i}{attrs}>"))
            .collect::<String>(),
    ] {
        let t = std::time::Instant::now();
        let err = check_nesting(&html).unwrap_err();
        assert!(
            t.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            t.elapsed()
        );
        let cap = 50_000_000 + 16 * html.len() as u64;
        assert!(
            matches!(&err, ParseError::Limit { kind, limit } if kind == "html formatting elements" && *limit == cap),
            "{err:?}"
        );
    }
    // Noah's Ark keeps the list short for repeated identical formatting
    assert!(check_nesting(&"<p><b>x<i>y</p>".repeat(100_000)).is_ok());
}

#[test]
fn attributes_per_element_are_bounded() {
    let many = |n: usize| {
        format!(
            "<p{}>x",
            (0..n).map(|i| format!(" a{i}")).collect::<String>()
        )
    };
    assert_eq!(max_attrs(&many(8192)), 8192);
    assert!(check_nesting(&many(8192)).is_ok());
    let t = std::time::Instant::now();
    let err = check_nesting(&many(200_000)).unwrap_err();
    assert!(
        t.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        t.elapsed()
    );
    assert!(
        matches!(&err, ParseError::Limit { kind, limit: 8192 } if kind == "html attributes per element"),
        "{err:?}"
    );
    // quoted and unquoted values, `>` inside quotes, long data URIs
    assert_eq!(max_attrs("<a href=x title='a > b c' b=\"d e\" c>t</a>"), 4);
    assert_eq!(max_attrs("<p =a b =c>"), 2);
    let uri = format!(
        "<img src=\"data:image/png;base64,{}\" alt=\"x y z\">",
        "QUJD ".repeat(200_000)
    );
    assert_eq!(max_attrs(&uri), 2);
    assert!(check_nesting(&uri.repeat(3)).is_ok());
    // more than the cap in total, few per element
    assert!(check_nesting(&"<p a b c d>x</p>".repeat(10_000)).is_ok());
    // raw text and comments are not markup; `<plaintext` ends the markup
    let words = "w ".repeat(10_000);
    for html in [
        format!("<script>if (a<b) {{}} {words}</script>"),
        format!("<SCRIPT type=x>i<n {words}</Script><p a>"),
        format!("<style>a<b {words}</style>"),
        format!("<title>a<b {words}</title>"),
        format!("<!-- a<b {words} -->"),
        format!("<plaintext><p {words}>"),
    ] {
        assert!(max_attrs(&html) <= 1, "{}", &html[..20]);
    }
    assert_eq!(max_attrs(&format!("<!--><p {words}>")), 10_000);
    // the tokenizer also ends a comment at `--!>`
    let attrs: String = (0..=9000).map(|i| format!(" a{i}")).collect();
    let err = check_nesting(&format!("<!-- x --!><p{attrs}>y")).unwrap_err();
    assert!(
        matches!(&err, ParseError::Limit { kind, limit: 8192 } if kind == "html attributes per element"),
        "{err:?}"
    );
    assert_eq!(max_attrs(&format!("<!--!><p {words}> -->")), 0);
    assert_eq!(max_attrs(&format!("<script></script><p {words}>")), 10_000);
}

#[test]
fn formatting_end_tag_scans_are_bounded() {
    let open: String = (0..5_000).map(|i| format!("<b id={i}>")).collect();
    let html = format!("<div>{open}</div>{}", "</i>".repeat(100_000));
    let t = std::time::Instant::now();
    let err = check_nesting(&html).unwrap_err();
    assert!(
        t.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        t.elapsed()
    );
    assert!(
        matches!(&err, ParseError::Limit { kind, .. } if kind == "html formatting elements"),
        "{err:?}"
    );
    // without the end tags the same list is within the cap
    assert!(check_nesting(&format!("<div>{open}</div>")).is_ok());
    assert!(check_nesting(&"<p><b>x</b> <i>y</i> <a href=z>w</a></p>".repeat(100_000)).is_ok());
}
