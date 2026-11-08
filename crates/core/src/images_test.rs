use super::*;
use crate::image_set_id;
use crate::{Cell, ImageInfo, ListItem, Section, SectionKind};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-one";
const PNG2: &[u8] = b"\x89PNG\r\n\x1a\n-two";
const JPG: &[u8] = b"\xff\xd8\xff\xe0-three";

fn img(data: &[u8]) -> Block {
    Block::Image {
        alt: None,
        ocr_text: None,
        bbox: None,
        image: Some(Box::new(ImageInfo::embedded(data.to_vec(), None))),
    }
}

fn info(b: &Block) -> &ImageInfo {
    match b {
        Block::Image { image: Some(i), .. } => i,
        other => panic!("{other:?}"),
    }
}

fn doc(blocks: Vec<Vec<Block>>) -> Document {
    Document::new(
        blocks
            .into_iter()
            .enumerate()
            .map(|(i, b)| {
                let mut s = Section::new(SectionKind::Page, i);
                s.blocks = b;
                s
            })
            .collect(),
    )
}

#[test]
fn numbers_in_document_order_and_dedupes_by_content() {
    let mut cell = Cell::new("c");
    cell.blocks.push(img(JPG));
    let mut item = ListItem::new("i");
    item.blocks.push(img(PNG));
    let mut d = doc(vec![
        vec![
            img(PNG2),
            Block::List {
                ordered: false,
                items: vec![item],
                bbox: None,
            },
        ],
        vec![Block::Table {
            rows: vec![vec![cell]],
            header_rows: 0,
            bbox: None,
        }],
        vec![img(PNG2)],
    ]);
    collect_images(&mut d, &ImageOptions::default());
    let files: Vec<&str> = d.images.iter().map(|s| s.file.as_str()).collect();
    assert_eq!(files, ["image1.png", "image2.png", "image3.jpg"]);
    assert_eq!(&*d.images[2].data, JPG);
    assert_eq!(d.images[2].mime, "image/jpeg");
    let last = info(&d.sections[2].blocks[0]);
    assert_eq!(last.file.as_deref(), Some("image1.png"));
    assert_eq!(last.size, Some(PNG2.len() as u64));
    assert_eq!(last.sha256.as_deref().map(str::len), Some(64));
    assert!(last.pending_data().is_none());
    assert!(d.warnings.is_empty());
}

#[test]
fn limits_skip_with_one_warning_per_reason() {
    let opts = ImageOptions {
        max_count: 1,
        max_bytes: 11,
        ..ImageOptions::default()
    };
    const GIF: &[u8] = b"GIF89a-x";
    // JPG (10 bytes) fits; PNG (12) is too large; GIF is over the count, twice; JPG again is a
    // duplicate of the stored one.
    let mut d = doc(vec![vec![img(JPG), img(PNG), img(GIF), img(GIF), img(JPG)]]);
    collect_images(&mut d, &opts);
    assert_eq!(d.images.len(), 1);
    let s = &d.sections[0].blocks;
    assert_eq!(info(&s[1]).skipped, Some("too_large"));
    assert_eq!(info(&s[1]).file, None);
    assert_eq!(info(&s[2]).skipped, Some("max_images"));
    assert_eq!(info(&s[4]).file.as_deref(), Some("image1.jpg"));
    let msgs: Vec<&str> = d.warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(
        msgs,
        [
            "image skipped: larger than 11 bytes (1 images)",
            "image skipped: more than 1 images (2 images)"
        ]
    );
    assert!(d.warnings.iter().all(|w| w.level == Level::Warn));
}

#[test]
fn total_budget_and_kept_data_off() {
    let opts = ImageOptions {
        max_total_bytes: 20,
        keep_data: false,
        ..ImageOptions::default()
    };
    let mut d = doc(vec![vec![img(PNG), img(PNG2)]]);
    collect_images(&mut d, &opts);
    assert!(d.images.is_empty());
    let s = &d.sections[0].blocks;
    assert_eq!(info(&s[0]).file.as_deref(), Some("image1.png"));
    assert_eq!(info(&s[1]).skipped, Some("max_total_bytes"));
}

#[test]
fn disabled_removes_pictures_and_adds_nothing() {
    let mut d = doc(vec![vec![img(PNG)]]);
    let off = ImageOptions {
        enabled: false,
        ..ImageOptions::default()
    };
    collect_images(&mut d, &off);
    assert_eq!(d, doc(vec![vec![Block::image(None, None, None)]]));
}

#[test]
fn external_and_parser_skips_are_reported() {
    let ext = |t: &str| Block::Image {
        alt: None,
        ocr_text: None,
        bbox: None,
        image: Some(Box::new(ImageInfo::external(t))),
    };
    let mut d = doc(vec![vec![
        ext("https://e.org/a.png"),
        ext("file:///x.png"),
        Block::Image {
            alt: None,
            ocr_text: None,
            bbox: None,
            image: Some(Box::new(ImageInfo::skipped("no_data", None))),
        },
    ]]);
    collect_images(&mut d, &ImageOptions::default());
    let msgs: Vec<(Level, &str)> = d
        .warnings
        .iter()
        .map(|w| (w.level, w.message.as_str()))
        .collect();
    assert_eq!(
        msgs,
        [
            (
                Level::Info,
                "image skipped: linked to a local or non-web target (1 images)"
            ),
            (
                Level::Warn,
                "image skipped: picture bytes not available (1 images)"
            ),
            (
                Level::Info,
                "image not saved: 1 linked to an external target, not downloaded"
            ),
        ]
    );
}

#[test]
fn json_has_metadata_only_and_markdown_links() {
    let mut d = doc(vec![vec![
        img(PNG),
        Block::image(Some("old".into()), None, None),
    ]]);
    collect_images(&mut d, &ImageOptions::default());
    let json = d.to_json();
    assert!(json.contains("\"file\": \"image1.png\""), "{json}");
    assert!(json.contains("\"mime\": \"image/png\""), "{json}");
    assert!(
        !json.contains("\"data\"") && !json.contains("\"images\""),
        "{json}"
    );
    // the second block has no picture: no `image` key at all
    assert_eq!(json.matches("\"image\"").count(), 3, "{json}");
    let o = crate::MarkdownOptions {
        image_dir: Some("a.docx.assets".into()),
        ..Default::default()
    };
    assert_eq!(
        d.to_markdown_with(&o),
        "![изображение 1](a.docx.assets/image1.png)\n\n<!-- image: old -->\n"
    );
}

#[test]
fn second_pass_changes_nothing() {
    let mut d = doc(vec![vec![img(PNG), img(JPG)]]);
    collect_images(&mut d, &ImageOptions::default());
    let once = d.clone();
    collect_images(&mut d, &ImageOptions::default());
    assert_eq!(d, once);
}

/// Markdown of `blocks` (pictures stored as `img-3fa9c2/imageN.png`).
fn md_of(blocks: Vec<Block>) -> String {
    let mut d = doc(vec![blocks]);
    collect_images(&mut d, &ImageOptions::default());
    d.to_markdown_with(&crate::MarkdownOptions {
        image_dir: Some("img-3fa9c2".into()),
        ..Default::default()
    })
}

fn pic(alt: Option<&str>) -> Block {
    Block::Image {
        alt: alt.map(Into::into),
        ocr_text: None,
        bbox: None,
        image: Some(Box::new(ImageInfo::embedded(PNG.to_vec(), None))),
    }
}

fn p(t: &str) -> Block {
    Block::paragraph(t, None)
}

#[test]
fn author_description_is_the_alt_and_is_not_dropped_for_a_caption() {
    assert_eq!(
        md_of(vec![pic(Some(" Сеть\n [схема] ")), p("Схема 1")]),
        "![Сеть \\[схема\\]](img-3fa9c2/image1.png)\n\nСхема 1\n"
    );
}

#[test]
fn auto_names_are_not_a_description() {
    for auto in ["Picture 3", "Рисунок 1", "image1.png", "IMG_0042.jpg", "  "] {
        assert_eq!(
            md_of(vec![pic(Some(auto))]),
            "![изображение 1](img-3fa9c2/image1.png)\n",
            "{auto}"
        );
    }
    // a caption still serves when the name is automatic
    assert!(md_of(vec![pic(Some("Picture 3")), p("Рис. 2. Вид")]).starts_with("![Рис. 2. Вид]("));
    assert!(md_of(vec![pic(Some("Схема 1.png"))]).starts_with("![Схема 1.png]("));
}

#[test]
fn caption_below_then_above_else_label() {
    let below = md_of(vec![p("Рисунок 9"), pic(None), p("Рисунок 1 - Вид")]);
    assert!(
        below.contains("![Рисунок 1 - Вид](img-3fa9c2/image1.png)"),
        "{below}"
    );
    assert!(below.starts_with("Рисунок 9\n\n") && below.ends_with("Рисунок 1 - Вид\n"));
    let above = md_of(vec![p("Figure 2: overview"), pic(None), p("Обычный текст")]);
    assert!(
        above.contains("![Figure 2: overview](img-3fa9c2/image1.png)"),
        "{above}"
    );
    for other in ["Обычный текст", "Рисунки в тексте", "Схематично"]
    {
        assert_eq!(
            md_of(vec![p(other), pic(None), p(other)]),
            format!("{other}\n\n![изображение 1](img-3fa9c2/image1.png)\n\n{other}\n")
        );
    }
    // a heading is not a caption
    let h = md_of(vec![pic(None), Block::heading(2, "Рисунок", None)]);
    assert!(h.starts_with("![изображение 1]("), "{h}");
}

#[test]
fn long_caption_is_cut_and_brackets_are_escaped() {
    let long = format!("Рисунок 1 [a] {}", "слово ".repeat(40));
    let md = md_of(vec![pic(None), p(&long)]);
    let alt = md.strip_prefix("![").unwrap().split("](").next().unwrap();
    assert!(alt.starts_with("Рисунок 1 \\[a\\] слово"), "{alt}");
    assert!(alt.ends_with('…') && alt.chars().count() < 130, "{alt}");
    assert!(md.ends_with(&format!("{long}\n")));
}

#[test]
fn image_set_id_depends_on_pictures_only() {
    let stored = |data: &[&[u8]]| {
        let mut d = doc(vec![data.iter().map(|b| img(b)).collect()]);
        collect_images(&mut d, &ImageOptions::default());
        d.images
    };
    let a = stored(&[PNG, JPG]);
    let id = image_set_id(&a, 6);
    assert_eq!(id.len(), 6);
    assert!(
        id.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
    );
    assert_eq!(id, image_set_id(&stored(&[PNG, JPG]), 6));
    // longer ids extend the shorter one; the length is capped at 64
    assert!(image_set_id(&a, 10).starts_with(&id));
    assert_eq!(image_set_id(&a, 999).len(), 64);
    // other pictures or another order: another id
    assert_ne!(id, image_set_id(&stored(&[JPG, PNG]), 6));
    assert_ne!(id, image_set_id(&stored(&[PNG]), 6));
}
