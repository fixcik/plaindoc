//! Pictures (`Block::Image` with an [`ImageInfo`]) in Markdown and HTML output.

use crate::ImageInfo;
use crate::md_links;

/// The link destination: `dir/imageN.ext` for a stored one (a local path: `%`, `#` and `?` of
/// the file name are encoded too), else its external URL (whose `%xx`, `#`, `?` mean something).
fn target(info: &ImageInfo, dir: Option<&str>) -> Option<String> {
    if let Some(f) = info.file.as_deref().filter(|_| info.skipped.is_none()) {
        let path = match dir.filter(|d| !d.is_empty()) {
            Some(d) => format!("{}/{f}", d.trim_end_matches('/')),
            None => f.to_string(),
        };
        let path = path
            .replace('%', "%25")
            .replace('#', "%23")
            .replace('?', "%3F");
        return Some(md_links::dest(&path));
    }
    info.url.as_deref().map(md_links::dest)
}

/// The alt text on one line, or `изображение N` (`N` of `imageN`).
fn alt_text(alt: Option<&str>, info: &ImageInfo) -> String {
    match alt.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => a.replace(['\r', '\n'], " "),
        None => match info.number() {
            Some(n) => format!("изображение {n}"),
            None => "изображение".to_string(),
        },
    }
}

/// `![alt](target)`; `None` when the picture has neither a stored file nor a URL.
pub(crate) fn markdown(alt: Option<&str>, info: &ImageInfo, dir: Option<&str>) -> Option<String> {
    let t = target(info, dir)?;
    Some(format!(
        "![{}]({})",
        md_links::label(&alt_text(alt, info)),
        t
    ))
}

/// `<img src=".." alt="..">` for HTML tables; `None` as for [`markdown`].
pub(crate) fn html(
    alt: Option<&str>,
    info: &ImageInfo,
    dir: Option<&str>,
    esc: impl Fn(&str) -> String,
) -> Option<String> {
    let t = target(info, dir)?;
    Some(format!(
        "<img src=\"{}\" alt=\"{}\">",
        esc(&t),
        esc(&alt_text(alt, info))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(file: &str) -> ImageInfo {
        ImageInfo {
            file: Some(file.into()),
            ..ImageInfo::default()
        }
    }

    #[test]
    fn stored_picture_links_into_dir() {
        let i = stored("image2.png");
        assert_eq!(
            markdown(None, &i, Some("x y.docx.assets")).unwrap(),
            "![изображение 2](x%20y.docx.assets/image2.png)"
        );
        assert_eq!(
            markdown(None, &i, Some("a #1 (v) 50%20?.docx.assets")).unwrap(),
            "![изображение 2](a%20%231%20%28v%29%2050%2520%3F.docx.assets/image2.png)"
        );
        let esc = |s: &str| s.replace('&', "&amp;");
        assert_eq!(
            html(Some("x"), &i, Some("a#b"), esc).unwrap(),
            "<img src=\"a%23b/image2.png\" alt=\"x\">"
        );
        assert_eq!(
            markdown(Some("a [b]\nc"), &i, None).unwrap(),
            "![a \\[b\\] c](image2.png)"
        );
    }

    #[test]
    fn external_and_skipped() {
        let i = ImageInfo::external("http://e.org/a (1).png");
        assert_eq!(
            markdown(Some("x"), &i, Some("d")).unwrap(),
            "![x](http://e.org/a%20%281%29.png)"
        );
        let u = ImageInfo::external("https://e.org/a%20b.png?x=1#f");
        assert_eq!(
            markdown(Some("x"), &u, None).unwrap(),
            "![x](https://e.org/a%20b.png?x=1#f)"
        );
        let mut s = stored("image1.png");
        s.skipped = Some("max_images");
        assert_eq!(markdown(None, &s, None), None);
        assert_eq!(markdown(None, &ImageInfo::default(), None), None);
    }
}
