//! Opening the input and probing CFB (OLE) containers.

use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};

use office_oxide::cfb::CfbReader;
use plaindoc_core::{Format, Input, ParseError, Result};

use crate::CfbProbe;

/// Readable, seekable, sendable byte source.
pub(crate) trait ReadSeekSend: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeekSend for T {}

/// Opened input with its length in bytes.
pub(crate) struct Opened {
    pub reader: Box<dyn ReadSeekSend>,
    pub len: u64,
}

/// Opens the input without detecting its format or checking its size.
pub(crate) fn open_input(input: Input) -> Result<Opened> {
    Ok(match input {
        Input::Path(p) => {
            let len = std::fs::metadata(&p)?.len();
            Opened {
                reader: Box::new(std::io::BufReader::new(File::open(&p)?)),
                len,
            }
        }
        Input::Bytes(b) => Opened {
            len: b.len() as u64,
            reader: Box::new(Cursor::new(b)),
        },
        _ => return Err(ParseError::Unsupported(Format::Unknown)), // Input is non_exhaustive
    })
}

/// Root stream/storage names that tell CFB-based formats apart.
pub(crate) const ROOT_STREAMS: [&str; 8] = [
    "WordDocument",
    "Workbook",
    "Book",
    "PowerPoint Document",
    "PP97_DUALSTORAGE",
    "EncryptionInfo",
    "EncryptedPackage",
    "__properties_version1.0",
];

/// Names from [`ROOT_STREAMS`] present in the container root; `r` is left at
/// position 0. A container without any of them that is empty or damaged
/// (e.g. truncated) is `Corrupt`.
pub(crate) fn cfb_root_streams<R: Read + Seek>(r: &mut R) -> Result<Vec<String>> {
    let names = root_streams(r);
    let rewind = r.seek(SeekFrom::Start(0));
    let names = names?;
    rewind?;
    Ok(names)
}

fn root_streams<R: Read + Seek>(r: &mut R) -> Result<Vec<String>> {
    let cfb = CfbReader::new(&mut *r).map_err(|e| ParseError::Corrupt(e.to_string()))?;
    let names: Vec<String> = ROOT_STREAMS
        .iter()
        .filter(|n| cfb.has_root_entry(n))
        .map(|n| (*n).to_string())
        .collect();
    if names.is_empty() && (cfb.entries().len() <= 1 || !cfb.warnings().is_empty()) {
        return Err(ParseError::Corrupt(
            "compound file is truncated or damaged: no document stream".into(),
        ));
    }
    Ok(names)
}

/// Whether the xls workbook stream contains a BIFF FORMULA record. Any error
/// gives `false`; `r` is left at position 0.
pub(crate) fn xls_has_formulas<R: Read + Seek>(r: &mut R) -> bool {
    let found = has_formula_record(r);
    let _ = r.seek(SeekFrom::Start(0));
    found
}

fn has_formula_record<R: Read + Seek>(r: &mut R) -> bool {
    const FORMULA: u16 = 0x0006;
    let Ok(mut cfb) = CfbReader::new(&mut *r) else {
        return false;
    };
    let Ok(data) = cfb
        .open_stream("Workbook")
        .or_else(|_| cfb.open_stream("Book"))
    else {
        return false;
    };
    let mut pos = 0usize;
    while let Some(head) = data.get(pos..pos + 4) {
        let kind = u16::from_le_bytes([head[0], head[1]]);
        let len = usize::from(u16::from_le_bytes([head[2], head[3]]));
        if pos + 4 + len > data.len() {
            return false;
        }
        if kind == FORMULA {
            return true;
        }
        pos += 4 + len;
    }
    false
}

/// [`CfbProbe`] over `office_oxide`'s CFB reader: bare root stream names.
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct CfbStreams;

impl CfbProbe for CfbStreams {
    fn stream_names(&self, input: &Input) -> Result<Vec<String>> {
        match input {
            Input::Path(p) => cfb_root_streams(&mut File::open(p)?),
            Input::Bytes(b) => cfb_root_streams(&mut Cursor::new(b.as_slice())),
            _ => Err(ParseError::Unsupported(Format::Unknown)), // Input is non_exhaustive
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plaindoc_core::classify_cfb_streams;

    fn fx(rel: &str) -> std::path::PathBuf {
        std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/office/"
        ))
        .join(rel)
    }
    fn classify(input: &Input) -> Result<Format> {
        let names = CfbStreams.stream_names(input)?;
        classify_cfb_streams(&names.iter().map(String::as_str).collect::<Vec<_>>())
    }

    #[test]
    fn path_and_bytes_open_with_same_length() {
        for rel in ["generated/tables.docx", "poi/table-merges.doc"] {
            let mut a = open_input(Input::Path(fx(rel))).unwrap();
            let mut b = open_input(Input::Bytes(std::fs::read(fx(rel)).unwrap())).unwrap();
            assert_eq!(a.len, b.len, "{rel}");
            let (mut da, mut db) = (Vec::new(), Vec::new());
            a.reader.read_to_end(&mut da).unwrap();
            b.reader.read_to_end(&mut db).unwrap();
            assert_eq!(da, db, "{rel}");
            assert_eq!(da.len() as u64, a.len, "{rel}");
        }
    }

    #[test]
    fn probe_classifies_legacy_and_encrypted() {
        assert_eq!(
            classify(&Input::Path(fx("poi/bullets.ppt"))).unwrap(),
            Format::Ppt
        );
        assert_eq!(
            classify(&Input::Path(fx("poi/SimpleWithFormula.xls"))).unwrap(),
            Format::Xls
        );
        assert_eq!(
            classify(&Input::Bytes(
                std::fs::read(fx("poi/table-merges.doc")).unwrap()
            ))
            .unwrap(),
            Format::Doc
        );
        assert!(matches!(
            classify(&Input::Path(fx("generated/encrypted.docx"))),
            Err(ParseError::Encrypted)
        ));
    }

    #[test]
    fn root_streams_ignore_embedded_objects() {
        let mut f = std::fs::File::open(fx("poi/table-merges.doc")).unwrap();
        assert_eq!(cfb_root_streams(&mut f).unwrap(), ["WordDocument"]);
        assert_eq!(std::io::Seek::stream_position(&mut f).unwrap(), 0);
    }

    #[test]
    fn truncated_cfb_is_corrupt_not_unknown() {
        for rel in [
            "poi/Lists.doc",
            "poi/table-merges.doc",
            "poi/SimpleWithFormula.xls",
        ] {
            let b = std::fs::read(fx(rel)).unwrap();
            let half = b[..b.len() / 2].to_vec();
            assert!(
                matches!(
                    cfb_root_streams(&mut std::io::Cursor::new(half.clone())),
                    Err(ParseError::Corrupt(_))
                ),
                "{rel}"
            );
            assert!(
                matches!(classify(&Input::Bytes(half)), Err(ParseError::Corrupt(_))),
                "{rel}"
            );
        }
        let stub = Input::Bytes(vec![
            0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0,
        ]);
        assert!(matches!(
            CfbStreams.stream_names(&stub),
            Err(ParseError::Corrupt(_))
        ));
    }

    #[test]
    fn missing_path_is_io() {
        assert!(matches!(
            open_input(Input::Path(fx("nope.docx"))),
            Err(ParseError::Io(_))
        ));
    }

    #[test]
    fn xls_formula_scan() {
        let mut f = std::fs::File::open(fx("poi/SimpleWithFormula.xls")).unwrap();
        assert!(xls_has_formulas(&mut f));
        assert_eq!(std::io::Seek::stream_position(&mut f).unwrap(), 0);
        let mut f = std::fs::File::open(fx("poi/table-merges.doc")).unwrap();
        assert!(!xls_has_formulas(&mut f));
        assert_eq!(std::io::Seek::stream_position(&mut f).unwrap(), 0);
    }
}
