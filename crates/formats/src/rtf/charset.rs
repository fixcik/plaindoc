//! RTF code pages: Windows code page number to `encoding_rs` encoding, `\fcharset` to code page.

use encoding_rs::Encoding;

pub(crate) fn codepage(cp: i32) -> Option<&'static Encoding> {
    let label = match cp {
        65001 => "utf-8".to_string(),
        866 => "ibm866".to_string(),
        932 => "shift_jis".to_string(),
        936 => "gbk".to_string(),
        949 => "euc-kr".to_string(),
        950 => "big5".to_string(),
        874 | 1250..=1258 => format!("windows-{cp}"),
        10000 => "macintosh".to_string(),
        10007 => "x-mac-cyrillic".to_string(),
        20866 => "koi8-r".to_string(),
        21866 => "koi8-u".to_string(),
        28591..=28606 => format!("iso-8859-{}", cp - 28590),
        _ => return None,
    };
    Encoding::for_label(label.as_bytes())
}

pub(crate) fn charset_codepage(charset: i32) -> Option<i32> {
    Some(match charset {
        77 => 10000,
        128 => 932,
        129 => 949,
        134 => 936,
        136 => 950,
        161 => 1253,
        162 => 1254,
        163 => 1258,
        177 => 1255,
        178 => 1256,
        186 => 1257,
        204 => 1251,
        222 => 874,
        238 => 1250,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages() {
        assert_eq!(codepage(1251).unwrap().name(), "windows-1251");
        assert_eq!(codepage(866).unwrap().name(), "IBM866");
        assert_eq!(codepage(28595).unwrap().name(), "ISO-8859-5");
        assert_eq!(codepage(20866).unwrap().name(), "KOI8-R");
        assert_eq!(codepage(28595).unwrap().name(), "ISO-8859-5");
        assert_eq!(codepage(28602), None); // no ISO-8859-12 exists
        assert_eq!(codepage(932).unwrap().name(), "Shift_JIS");
        assert_eq!(codepage(936).unwrap().name(), "GBK");
        assert_eq!(codepage(949).unwrap().name(), "EUC-KR");
        assert_eq!(codepage(950).unwrap().name(), "Big5");
        assert_eq!(codepage(437), None);
        assert_eq!(charset_codepage(204), Some(1251));
        assert_eq!(charset_codepage(0), None);
    }
}
