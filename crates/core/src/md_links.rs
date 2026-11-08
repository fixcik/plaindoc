//! Hyperlinks of headings and paragraphs ([`Link`]) in Markdown and HTML output.

use crate::Link;
use std::borrow::Cow;

/// Link destination: ASCII controls, space and `"()<>\`|` percent-encoded (so the destination
/// cannot end early or break a GFM table row); other chars as they are.
pub(crate) fn dest(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        if c.is_ascii_control() || " \"()<>\\`|".contains(c) {
            out.push_str(&format!("%{:02X}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// Link text: `\`, `[`, `]`, `` ` `` and `<` backslash-escaped (the text cannot close the label,
/// open a code span or raw HTML).
pub(crate) fn label(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        if matches!(c, '\\' | '[' | ']' | '`' | '<') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `text` cut into `(segment, url)` pieces: `Some(url)` inside a link. Links that are empty,
/// out of range or overlap an earlier one are ignored.
fn pieces<'a>(text: &'a str, links: &'a [Link]) -> Vec<(&'a str, Option<&'a str>)> {
    let mut sorted: Vec<&Link> = links.iter().filter(|l| l.start < l.end).collect();
    sorted.sort_by_key(|l| l.start);
    // byte offset of each char index, plus the end
    let bytes: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let n = bytes.len() - 1;
    let mut out = Vec::new();
    let mut at = 0; // char index
    for l in sorted {
        if l.start < at || l.end > n {
            continue;
        }
        if l.start > at {
            out.push((&text[bytes[at]..bytes[l.start]], None));
        }
        out.push((&text[bytes[l.start]..bytes[l.end]], Some(l.url.as_str())));
        at = l.end;
    }
    if at < n {
        out.push((&text[bytes[at]..], None));
    }
    out
}

/// Heading or paragraph text with its links as `[text](url)`; the text outside links as is.
pub(crate) fn markdown<'a>(text: &'a str, links: &[Link]) -> Cow<'a, str> {
    if links.is_empty() {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        pieces(text, links)
            .into_iter()
            .map(|(s, url)| match url {
                Some(u) => format!("[{}]({})", label(s), dest(u)),
                None => s.to_string(),
            })
            .collect(),
    )
}

/// GFM cell text with links: `esc` (the cell escaping, no trim) applies to every piece, link
/// text is escaped for the label as well.
pub(crate) fn gfm(text: &str, links: &[Link], esc: impl Fn(&str) -> String) -> String {
    pieces(text, links)
        .into_iter()
        .map(|(s, url)| match url {
            Some(u) => {
                let mut t = esc(s);
                for c in ['[', ']', '`'] {
                    t = t.replace(c, &format!("\\{c}"));
                }
                format!("[{t}]({})", dest(u))
            }
            None => esc(s),
        })
        .collect()
}

/// HTML text with links as `<a href="url">text</a>`; `esc` is the HTML escaping.
pub(crate) fn html(text: &str, links: &[Link], esc: impl Fn(&str) -> String) -> String {
    pieces(text, links)
        .into_iter()
        .map(|(s, url)| match url {
            Some(u) => format!("<a href=\"{}\">{}</a>", esc(&dest(u)), esc(s)),
            None => esc(s),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_and_label_are_escaped() {
        assert_eq!(
            dest("https://e.org/a b(c)<d>|\"`\\\u{7}é"),
            "https://e.org/a%20b%28c%29%3Cd%3E%7C%22%60%5C%07é"
        );
        assert_eq!(label(r"a[b]\c`<d*_"), r"a\[b\]\\c\`\<d*_");
    }

    #[test]
    fn pieces_skip_bad_ranges() {
        let l = |s, e| Link::new(s, e, "u");
        // char offsets, not bytes
        assert_eq!(markdown("при вет", &[l(4, 7)]), "при [вет](u)");
        // overlapping, empty and out-of-range links are ignored; order does not matter
        assert_eq!(
            markdown("abcdef", &[l(3, 4), l(0, 2), l(1, 3), l(5, 5), l(5, 9)]),
            "[ab](u)c[d](u)ef"
        );
    }
}
