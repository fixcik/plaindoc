#![cfg(feature = "text")]
use plaindoc_core::*;
use plaindoc_formats::text::decode;

#[test]
fn utf8_bom_is_stripped() {
    let d = decode(b"\xEF\xBB\xBFhi");
    assert_eq!((d.text.as_str(), d.encoding), ("hi", "UTF-8"));
    assert!(d.warnings.is_empty());
}

#[test]
fn utf16le_with_bom() {
    let mut b = vec![0xFF, 0xFE];
    for u in "Привет".encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    let d = decode(&b);
    assert_eq!((d.text.as_str(), d.encoding), ("Привет", "UTF-16LE"));
}

#[test]
fn windows_1251_is_guessed() {
    let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode("Привет, мир! Это проверка кодировки.");
    let d = decode(&bytes);
    assert_eq!(d.encoding, "windows-1251");
    assert_eq!(d.text, "Привет, мир! Это проверка кодировки.");
    assert!(d.warnings.is_empty());
}

#[test]
fn binary_garbage_is_flagged_not_panicking() {
    let junk: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let d = decode(&junk);
    let kinds: Vec<_> = d.warnings.iter().map(|w| (w.kind, w.level)).collect();
    assert!(
        kinds.contains(&(WarningKind::SuspectedGarbage, Level::Loss)),
        "{kinds:?}"
    );
}

#[test]
fn ansi_colour_codes_are_not_control_garbage() {
    let line = "\x1b[31mERR\x1b[0m\n";
    let d = decode(line.repeat(100).as_bytes());
    assert!(d.warnings.is_empty(), "{:?}", d.warnings.len());
}

#[test]
fn declared_encoding_comes_after_bom_and_bad_utf8_falls_back() {
    use encoding_rs::{UTF_8, WINDOWS_1251};
    use plaindoc_formats::text::decode_with;
    let cp = WINDOWS_1251.encode("Привет, мир").0.into_owned();
    assert_eq!(decode_with(&cp, Some(WINDOWS_1251)).text, "Привет, мир");
    let bom = [b"\xEF\xBB\xBF".as_slice(), "Ж".as_bytes()].concat();
    assert_eq!(decode_with(&bom, Some(WINDOWS_1251)).text, "Ж");
    let long = WINDOWS_1251
        .encode("Сумма к оплате по счёту составляет сто рублей.")
        .0
        .into_owned();
    let d = decode_with(&long, Some(UTF_8));
    assert_eq!(d.encoding, "windows-1251");
    assert!(d.warnings.is_empty());
}
