use super::RAW_TEXT;

/// Most attributes on one tag, by a byte scan that follows the tokenizer's tag states
/// (name, attribute name, value, quoted value). Comments and the contents of raw-text
/// elements are skipped, `<plaintext` ends the scan; CDATA and foreign content (svg
/// `<script>`) are not modelled. Best effort (R16): the
/// tokenizer's per-tag duplicate-attribute check is quadratic and not reachable from
/// outside, and such a desync (a fake quoted value) can hide a tag from this scan.
pub(super) fn max_attrs(text: &str) -> usize {
    #[derive(Clone, Copy, PartialEq)]
    enum St {
        Data,
        TagName,
        BeforeName,
        Name,
        AfterName,
        BeforeValue,
        Value,
        Quoted(u8),
    }
    // first index of `pat` (ASCII case-insensitive) at or after `from`
    fn find(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
        b.get(from..)?
            .windows(pat.len())
            .position(|w| w.eq_ignore_ascii_case(pat))
            .map(|p| from + p)
    }
    let b = text.as_bytes();
    let (mut st, mut n, mut max) = (St::Data, 0usize, 0usize);
    // start of the tag name; the raw-text element whose end tag is skipped to
    let (mut name_at, mut raw): (usize, Option<&[u8]>) = (0, None);
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let ws = matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c');
        if st == St::TagName && (ws || c == b'/' || c == b'>') {
            let name = &b[name_at..i];
            if b[name_at - 1] == b'<' {
                if name.eq_ignore_ascii_case(b"plaintext") {
                    return max;
                }
                raw = RAW_TEXT
                    .iter()
                    .find(|(r, _)| name.eq_ignore_ascii_case(r.as_bytes()))
                    .map(|(r, _)| r.as_bytes());
            }
        }
        st = match st {
            St::Data => {
                let next = b.get(i + 1).copied().unwrap_or(0);
                let next2 = b.get(i + 2).copied().unwrap_or(0);
                if b[i..].starts_with(b"<!--") {
                    // ends at the first `-->` (`<!-->` and `<!--->` are empty comments)
                    // or `--!>` after the opening dashes
                    let Some(end) = (i + 2..b.len()).find_map(|j| {
                        if b[j..].starts_with(b"-->") {
                            Some(j + 3)
                        } else if j >= i + 4 && b[j..].starts_with(b"--!>") {
                            Some(j + 4)
                        } else {
                            None
                        }
                    }) else {
                        return max;
                    };
                    i = end;
                    continue;
                }
                if c == b'<'
                    && (next.is_ascii_alphabetic() || (next == b'/' && next2.is_ascii_alphabetic()))
                {
                    n = 0;
                    name_at = if next == b'/' { i + 2 } else { i + 1 };
                    raw = None;
                    i = name_at;
                    st = St::TagName;
                    continue;
                } else {
                    St::Data
                }
            }
            _ if c == b'>' && !matches!(st, St::Quoted(_)) => {
                if let Some(r) = raw.take() {
                    let close = [b"</".as_slice(), r].concat();
                    let Some(end) = find(b, i + 1, &close) else {
                        return max;
                    };
                    i = end;
                    st = St::Data;
                    continue;
                }
                St::Data
            }
            St::TagName if ws || c == b'/' => St::BeforeName,
            St::TagName => St::TagName,
            St::BeforeName | St::AfterName if ws || c == b'/' => {
                if c == b'/' {
                    St::BeforeName
                } else {
                    st
                }
            }
            St::AfterName if c == b'=' => St::BeforeValue,
            St::BeforeName | St::AfterName => {
                n += 1;
                max = max.max(n);
                St::Name
            }
            St::Name if ws => St::AfterName,
            St::Name if c == b'/' => St::BeforeName,
            St::Name if c == b'=' => St::BeforeValue,
            St::Name => St::Name,
            St::BeforeValue if ws => St::BeforeValue,
            St::BeforeValue if c == b'"' || c == b'\'' => St::Quoted(c),
            St::BeforeValue | St::Value if !ws => St::Value,
            St::Value => St::BeforeName,
            St::BeforeValue => St::BeforeValue,
            St::Quoted(q) if c == q => St::BeforeName,
            St::Quoted(q) => St::Quoted(q),
        };
        i += 1;
    }
    max
}
