//! Mapping of `office_oxide` errors to core [`ParseError`].

use office_oxide::OfficeError;
use office_oxide::core::Error as CoreError;
use office_oxide::doc::DocError;
use office_oxide::docx::DocxError;
use office_oxide::ppt::PptError;
use office_oxide::pptx::PptxError;
use office_oxide::xls::XlsError;
use office_oxide::xlsx::XlsxError;
use plaindoc_core::ParseError;

/// The OPC core error carried by `e`, directly or inside an OOXML wrapper.
fn core_of(e: &OfficeError) -> Option<&CoreError> {
    match e {
        OfficeError::Core(c)
        | OfficeError::Docx(DocxError::Core(c))
        | OfficeError::Xlsx(XlsxError::Core(c))
        | OfficeError::Pptx(PptxError::Core(c)) => Some(c),
        _ => None,
    }
}

/// Converts an `office_oxide` error; `package_limit` is the configured package limit.
pub(crate) fn map_error(e: OfficeError, package_limit: u64) -> ParseError {
    match core_of(&e) {
        Some(CoreError::DecompressionLimit { part, limit }) => {
            return ParseError::Limit {
                kind: format!("decompressed size of part {part}"),
                limit: *limit,
            };
        }
        Some(CoreError::PackageLimit(m)) => {
            return ParseError::Limit {
                kind: format!("package: {m}"),
                limit: package_limit,
            };
        }
        _ => {}
    }
    match e {
        OfficeError::Panic(m) => ParseError::Internal(m),
        OfficeError::UnsupportedFormat(m) if m.contains("password-protected") => {
            ParseError::Encrypted
        }
        OfficeError::UnsupportedFormat(m) if m.starts_with("thread spawn failed") => {
            ParseError::Internal(m)
        }
        OfficeError::Doc(DocError::Encrypted)
        | OfficeError::Xls(XlsError::Encrypted | XlsError::EncryptionNotSupported(_))
        | OfficeError::Ppt(PptError::Encrypted) => ParseError::Encrypted,
        OfficeError::Core(CoreError::Io(io))
        | OfficeError::Docx(DocxError::Core(CoreError::Io(io)))
        | OfficeError::Xlsx(XlsxError::Core(CoreError::Io(io)))
        | OfficeError::Pptx(PptxError::Core(CoreError::Io(io)))
        | OfficeError::Doc(DocError::Io(io))
        | OfficeError::Xls(XlsError::Io(io))
        | OfficeError::Ppt(PptError::Io(io)) => ParseError::Io(io),
        e @ (OfficeError::Core(_)
        | OfficeError::Docx(_)
        | OfficeError::Xlsx(_)
        | OfficeError::Pptx(_)
        | OfficeError::Doc(_)
        | OfficeError::Xls(_)
        | OfficeError::Ppt(_)
        | OfficeError::Incomplete(_)
        | OfficeError::UnsupportedFormat(_)) => ParseError::Corrupt(e.to_string()),
        e => ParseError::Internal(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use office_oxide::{
        OfficeError as E, core::Error as C, doc::DocError, docx::DocxError, ppt::PptError,
        xls::XlsError,
    };
    fn m(e: E) -> ParseError {
        map_error(e, 7)
    }

    #[test]
    fn panic_is_internal_not_corrupt() {
        assert!(matches!(m(E::Panic("boom".into())), ParseError::Internal(s) if s == "boom"));
    }
    #[test]
    fn encrypted_variants() {
        let s = "the file is a password-protected (encrypted) OOXML package; decryption is not supported";
        assert!(matches!(
            m(E::UnsupportedFormat(s.into())),
            ParseError::Encrypted
        ));
        assert!(matches!(
            m(E::Doc(DocError::Encrypted)),
            ParseError::Encrypted
        ));
        assert!(matches!(
            m(E::Xls(XlsError::Encrypted)),
            ParseError::Encrypted
        ));
        assert!(matches!(
            m(E::Ppt(PptError::Encrypted)),
            ParseError::Encrypted
        ));
        assert!(matches!(
            m(E::Xls(XlsError::EncryptionNotSupported("rc4".into()))),
            ParseError::Encrypted
        ));
    }
    #[test]
    fn limits_are_limit() {
        assert!(matches!(
            m(E::Docx(DocxError::Core(C::PackageLimit("x".into())))),
            ParseError::Limit { limit: 7, .. }
        ));
        match m(E::Core(C::DecompressionLimit {
            part: "word/document.xml".into(),
            limit: 1,
        })) {
            ParseError::Limit { kind, limit: 1 } => assert!(kind.contains("word/document.xml")),
            other => panic!("unexpected {other:?}"),
        }
    }
    #[test]
    fn io_is_io_and_structure_is_corrupt() {
        let io = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
        assert!(matches!(
            m(E::Docx(DocxError::Core(C::Io(io)))),
            ParseError::Io(_)
        ));
        assert!(matches!(
            m(E::Doc(DocError::MissingStream("WordDocument".into()))),
            ParseError::Corrupt(_)
        ));
        assert!(matches!(
            m(E::Docx(DocxError::Core(C::FormatMismatch {
                found: "a".into(),
                expected: "b".into()
            }))),
            ParseError::Corrupt(_)
        ));
        assert!(matches!(
            m(E::UnsupportedFormat("x".into())),
            ParseError::Corrupt(_)
        ));
        assert!(matches!(
            m(E::UnsupportedFormat("thread spawn failed: x".into())),
            ParseError::Internal(_)
        ));
    }
}
