//! Opening a PDF: size/emptiness checks, truncation and encryption handling, error mapping.

use pdf_oxide::PdfDocument;
use plaindoc_core::{Input, ParseError, ParseOptions, Result};

/// An opened, authenticated document plus facts reported later as document warnings.
pub(crate) struct OpenedPdf {
    pub doc: PdfDocument,
    pub truncated: bool,
    pub encrypted: bool,
}

/// Reads `input`, enforces `max_size`, opens the document and authenticates it if encrypted.
pub(crate) fn open_input(input: Input, opts: &ParseOptions) -> Result<OpenedPdf> {
    // Check a file's size before reading it into memory.
    if let (Some(limit), Input::Path(path)) = (opts.max_size, &input) {
        let size = std::fs::metadata(path)?.len();
        if size > limit {
            return Err(ParseError::TooLarge { size, limit });
        }
    }
    let bytes = input.into_bytes()?;
    let size = bytes.len() as u64;
    if let Some(limit) = opts.max_size
        && size > limit
    {
        return Err(ParseError::TooLarge { size, limit });
    }
    if bytes.is_empty() {
        return Err(ParseError::Corrupt("empty file".into()));
    }
    let truncated = !has_eof_marker(&bytes);
    let doc = PdfDocument::from_bytes(bytes).map_err(map_pdf_error)?;
    let encrypted = doc.is_encrypted();
    if encrypted && !doc.is_authenticated() {
        // Without this check `extract_structured` silently returns an empty page.
        // A wrong password returns Ok(false) and an authenticate Err (e.g. bad Encrypt dict) is
        // folded into false: both are deliberately reported as Encrypted.
        let ok = opts
            .password
            .as_ref()
            .is_some_and(|pw| doc.authenticate(pw.as_bytes()).unwrap_or(false));
        if !ok {
            return Err(ParseError::Encrypted);
        }
    }
    Ok(OpenedPdf {
        doc,
        truncated,
        encrypted,
    })
}

/// `%%EOF` within the last 1024 bytes.
pub(crate) fn has_eof_marker(bytes: &[u8]) -> bool {
    let tail = &bytes[bytes.len().saturating_sub(1024)..];
    tail.windows(5).any(|w| w == b"%%EOF")
}

/// Maps engine errors: encryption and I/O keep their kind, everything else is `Corrupt`.
pub(crate) fn map_pdf_error(e: pdf_oxide::Error) -> ParseError {
    match e {
        pdf_oxide::Error::EncryptedPdf => ParseError::Encrypted,
        pdf_oxide::Error::Io(e) => ParseError::Io(e),
        e => ParseError::Corrupt(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_oxide::editor::{EncryptionAlgorithm, EncryptionConfig, Permissions};
    use pdf_oxide::writer::DocumentBuilder;
    use plaindoc_core::{Input, ParseError, ParseOptions};

    fn encrypted(user: &str) -> Vec<u8> {
        let mut b = DocumentBuilder::new();
        b.letter_page()
            .font("Helvetica", 12.0)
            .at(72.0, 720.0)
            .paragraph("Secret body text.")
            .done();
        b.to_bytes_with_encryption(EncryptionConfig {
            user_password: user.into(),
            owner_password: "owner".into(),
            algorithm: EncryptionAlgorithm::Aes256,
            permissions: Permissions::all(),
        })
        .unwrap()
    }

    fn opts_pw(pw: Option<&str>) -> ParseOptions {
        let mut o = ParseOptions::default();
        o.password = pw.map(Into::into);
        o
    }

    #[test]
    fn eof_marker_only_in_tail() {
        assert!(has_eof_marker(b"%PDF-1.7\n...\n%%EOF\n"));
        assert!(!has_eof_marker(b"%PDF-1.7\n...truncated"));
        let mut v = b"%%EOF".to_vec();
        v.extend(vec![b' '; 2000]);
        assert!(!has_eof_marker(&v));
    }

    #[test]
    fn zero_bytes_and_garbage_are_corrupt() {
        assert!(matches!(
            open_input(Input::Bytes(vec![]), &ParseOptions::default()),
            Err(ParseError::Corrupt(_))
        ));
        let mut g = b"%PDF-1.7\n".to_vec();
        g.extend(vec![b'x'; 480]);
        assert!(matches!(
            open_input(Input::Bytes(g), &ParseOptions::default()),
            Err(ParseError::Corrupt(_))
        ));
    }

    #[test]
    fn missing_path_is_io() {
        let r = open_input(
            Input::Path("/nonexistent/plaindoc/x.pdf".into()),
            &ParseOptions::default(),
        );
        assert!(matches!(r, Err(ParseError::Io(_))));
    }

    #[test]
    fn max_size_is_too_large() {
        let mut o = ParseOptions::default();
        o.max_size = Some(10);
        assert!(matches!(
            open_input(Input::Bytes(encrypted("")), &o),
            Err(ParseError::TooLarge { limit: 10, .. })
        ));
    }

    #[test]
    fn max_size_checks_path_before_reading() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut f, &encrypted("")).unwrap();
        let mut o = ParseOptions::default();
        o.max_size = Some(10);
        assert!(matches!(
            open_input(Input::Path(f.path().into()), &o),
            Err(ParseError::TooLarge { limit: 10, size }) if size > 10
        ));
    }

    #[test]
    fn user_password_required() {
        let b = encrypted("user");
        assert!(matches!(
            open_input(Input::Bytes(b.clone()), &opts_pw(None)),
            Err(ParseError::Encrypted)
        ));
        assert!(matches!(
            open_input(Input::Bytes(b.clone()), &opts_pw(Some("wrong"))),
            Err(ParseError::Encrypted)
        ));
        let ok = open_input(Input::Bytes(b), &opts_pw(Some("user"))).unwrap();
        assert!(ok.encrypted);
        let text: String = ok
            .doc
            .extract_structured(0)
            .unwrap()
            .regions
            .iter()
            .map(|r| r.text.clone())
            .collect();
        assert!(text.contains("Secret body text."));
    }

    #[test]
    fn empty_user_password_opens_and_reports_encrypted() {
        let ok = open_input(Input::Bytes(encrypted("")), &ParseOptions::default()).unwrap();
        assert!(ok.encrypted && !ok.truncated);
    }
}
