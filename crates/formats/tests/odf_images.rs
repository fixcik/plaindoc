#![cfg(feature = "odf")]
//! Pictures of ODT/ODS: package parts, inline binary data, links, limits.
mod odf_util;
mod pictures_util;
use odf_util::{ODS, ODT, content};
use pictures_util::*;
use plaindoc_core::*;

fn frame(inner: &str, title: &str) -> String {
    format!(r#"<draw:frame><draw:image {inner}/><svg:title>{title}</svg:title></draw:frame>"#)
}

fn odt(text: &str, parts: &[(&str, &[u8])]) -> Vec<u8> {
    let c = content("", &format!("<office:text>{text}</office:text>"));
    let mut all: Vec<(&str, &[u8])> = vec![("content.xml", c.as_bytes())];
    all.extend_from_slice(parts);
    odf_of(ODT, &all)
}

fn sample() -> Vec<u8> {
    use base64::Engine;
    let inline = base64::engine::general_purpose::STANDARD.encode(BLUE);
    let text = [
        format!(
            "<text:p>Один{}</text:p>",
            frame(r#"xlink:href="Pictures/red.png""#, "Красный")
        ),
        format!(
            r#"<text:p><draw:frame><draw:image><office:binary-data>{}
{}</office:binary-data></draw:image></draw:frame></text:p>"#,
            &inline[..10],
            &inline[10..]
        ),
        format!(
            "<text:p>{}</text:p>",
            frame(r#"xlink:href="../outside.png""#, "")
        ),
        format!(
            "<text:p>{}</text:p>",
            frame(r#"xlink:href="./Pictures/red.png""#, "")
        ),
        format!(
            "<text:p>{}</text:p>",
            frame(r#"xlink:href="Pictures/none.png""#, "")
        ),
    ]
    .concat();
    odt(&text, &[("Pictures/red.png", RED)])
}

#[test]
fn odt_parts_inline_data_and_links() {
    let d = parse(sample(), &ParseOptions::default());
    let p = pictures(&d);
    assert_eq!(p.len(), 5, "{p:?}");
    let info = |i: usize| p[i].1.clone().unwrap();
    assert_eq!(info(0).file.as_deref(), Some("image1.png"));
    assert_eq!(info(0).source.as_deref(), Some("Pictures/red.png"));
    assert_eq!(info(1).file.as_deref(), Some("image2.png"));
    assert_eq!(info(1).source, None);
    assert_eq!(info(2).url.as_deref(), Some("../outside.png"));
    assert_eq!(info(3).file.as_deref(), Some("image1.png"));
    assert_eq!(info(4).skipped, Some("no_data"));
    assert_eq!(stored(&d, "image1.png"), RED);
    assert_eq!(stored(&d, "image2.png"), BLUE);
    let md = d.to_markdown();
    assert!(
        md.contains("Один\n\n![Красный](image1.png)\n\n![изображение 2](image2.png)"),
        "{md}"
    );
    assert!(md.contains("![изображение](../outside.png)"), "{md}");
    assert!(
        md.contains("image skipped: picture bytes not available (1 images)"),
        "{md}"
    );
}

#[test]
fn odt_off_and_too_large() {
    let d = parse(sample(), &no_images());
    assert!(pictures(&d).iter().all(|(_, i)| i.is_none()));
    assert!(d.warnings.iter().all(|w| !w.message.contains("image")));
    let mut o = ParseOptions::default();
    o.images.max_bytes = 10;
    let d = parse(sample(), &o);
    assert!(d.images.is_empty());
    assert_eq!(
        pictures(&d)[0].1.as_ref().unwrap().skipped,
        Some("too_large")
    );
    assert_eq!(
        pictures(&d)[1].1.as_ref().unwrap().skipped,
        Some("too_large")
    );
}

#[test]
fn ods_pictures_follow_the_table() {
    let sheet = format!(
        r#"<office:spreadsheet><table:table table:name="S"><table:shapes>{}</table:shapes><table:table-row><table:table-cell><text:p>a</text:p>{}</table:table-cell></table:table-row></table:table></office:spreadsheet>"#,
        frame(r#"xlink:href="Pictures/red.png""#, "Лого"),
        frame(r#"xlink:href="Pictures/blue.png""#, "")
    );
    let c = content("", &sheet);
    let bytes = odf_of(
        ODS,
        &[
            ("content.xml", c.as_bytes()),
            ("Pictures/red.png", RED),
            ("Pictures/blue.png", BLUE),
        ],
    );
    let d = parse(bytes.clone(), &ParseOptions::default());
    let b = &d.sections[0].blocks;
    assert!(matches!(b[0], Block::Table { .. }), "{b:?}");
    assert_eq!(
        files(&d),
        [Some("image1.png".into()), Some("image2.png".into())]
    );
    assert!(
        d.warnings.iter().all(|w| !w.message.contains("cell")),
        "{:?}",
        d.warnings
    );
    // off: as before, the cell picture is only counted
    let d = parse(bytes, &no_images());
    assert_eq!(d.sections[0].blocks.len(), 1);
}
