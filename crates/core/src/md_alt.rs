//! Alt text of a picture in Markdown: the author's description, else a nearby caption.
//! (`изображение N` is the last resort, see `md_images`.)

use crate::Block;

/// Longest caption used as alt text, in characters.
const MAX_ALT: usize = 120;

/// Words that open a figure caption (lower case); a word is complete when no letter follows.
const CAPTION_WORDS: [&str; 10] = [
    "рисунок",
    "рис",
    "схема",
    "диаграмма",
    "фото",
    "figure",
    "fig",
    "chart",
    "diagram",
    "picture",
];

/// Names that tools give to pictures (`Picture 3`, `Рисунок 1`, `image1.png`, `IMG_0042.jpg`) are
/// not a description.
fn is_auto_name(alt: &str) -> bool {
    let a = alt.trim().to_lowercase();
    const EXT: [&str; 10] = [
        "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "emf", "wmf", "webp",
    ];
    if let Some((stem, ext)) = a.rsplit_once('.')
        && EXT.contains(&ext)
    {
        // a file name (`image1.png`, `IMG_0042.jpg`)
        return !stem.contains(char::is_whitespace);
    }
    let name: String = a.chars().filter(|c| c.is_alphabetic()).collect();
    const NAMES: [&str; 8] = [
        "picture",
        "image",
        "img",
        "pic",
        "рисунок",
        "изображение",
        "картинка",
        "graphic",
    ];
    NAMES.contains(&name.as_str()) && a.chars().all(|c| c.is_alphanumeric() || " _-".contains(c))
}

/// The text of a paragraph that starts like a figure caption, on one line and cut to [`MAX_ALT`].
fn caption(b: Option<&Block>) -> Option<String> {
    let Some(Block::Paragraph { text, .. }) = b else {
        return None;
    };
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = one.to_lowercase();
    let opens = CAPTION_WORDS.iter().any(|w| {
        lower
            .strip_prefix(w)
            .is_some_and(|rest| !rest.starts_with(char::is_alphabetic))
    });
    if !opens {
        return None;
    }
    if one.chars().count() <= MAX_ALT {
        return Some(one);
    }
    let cut: String = one.chars().take(MAX_ALT).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// The alt text of the picture `b` between the blocks `prev` and `next`: its own description;
/// else the caption below, then the caption above; `None` leaves the default label.
pub(crate) fn resolve(prev: Option<&Block>, b: &Block, next: Option<&Block>) -> Option<String> {
    let Block::Image { alt, .. } = b else {
        return None;
    };
    if let Some(a) = alt
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty() && !is_auto_name(a))
    {
        return Some(a.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    caption(next).or_else(|| caption(prev))
}
