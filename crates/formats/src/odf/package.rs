//! ODF zip package: manifest check, size-limited part reading.

use std::io::{Cursor, Read, Seek};

use plaindoc_core::{Input, ParseError, Result};
use zip::ZipArchive;
use zip::result::ZipError;

pub(crate) trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

pub(crate) struct Package {
    pub(crate) content: String,
    pub(crate) styles: Option<String>,
    pub(crate) meta: Option<String>,
    /// The archive, kept to read pictures on demand.
    zip: ZipArchive<Box<dyn ReadSeek>>,
}

impl Package {
    /// A picture part; `Err` names why it is not taken (`no_data`: absent or unreadable,
    /// `too_large`: over `limit`, checked before and while inflating).
    pub(crate) fn picture(
        &mut self,
        name: &str,
        limit: u64,
    ) -> std::result::Result<Vec<u8>, &'static str> {
        match read_bytes(&mut self.zip, name, limit) {
            Ok(Some(b)) => Ok(b),
            Err(ParseError::Limit { .. }) => Err("too_large"),
            _ => Err("no_data"),
        }
    }
}

fn corrupt(name: &str, e: impl std::fmt::Display) -> ParseError {
    ParseError::Corrupt(format!("odf: {name}: {e}"))
}

/// One part as UTF-8 text; `None` if absent. Never holds more than `limit` + 1 bytes.
fn read_entry<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Option<String>> {
    match read_bytes(zip, name, limit)? {
        Some(buf) => String::from_utf8(buf)
            .map(Some)
            .map_err(|e| corrupt(name, e)),
        None => Ok(None),
    }
}

/// One part's bytes; `None` if absent. Never holds more than `limit` + 1 bytes.
fn read_bytes<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Option<Vec<u8>>> {
    let entry = match zip.by_name(name) {
        Ok(e) => e,
        Err(ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(corrupt(name, e)),
    };
    let too_big = || ParseError::Limit {
        kind: format!("uncompressed {name}"),
        limit,
    };
    if entry.size() > limit {
        return Err(too_big());
    }
    let mut buf = Vec::new();
    entry
        .take(limit.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| corrupt(name, e))?;
    if buf.len() as u64 > limit {
        return Err(too_big());
    }
    Ok(Some(buf))
}

fn read_package(reader: Box<dyn ReadSeek>, limit: u64) -> Result<Package> {
    let mut zip = ZipArchive::new(reader).map_err(|e| corrupt("zip", e))?;
    if let Some(manifest) = read_entry(&mut zip, "META-INF/manifest.xml", limit)?
        && manifest.contains("encryption-data")
    {
        return Err(ParseError::Encrypted);
    }
    let content = read_entry(&mut zip, "content.xml", limit)?
        .ok_or_else(|| ParseError::Corrupt("odf: content.xml is missing".into()))?;
    let styles = read_entry(&mut zip, "styles.xml", limit)?;
    let meta = read_entry(&mut zip, "meta.xml", limit)?;
    Ok(Package {
        content,
        styles,
        meta,
        zip,
    })
}

pub(crate) fn open(input: Input, limit: u64) -> Result<Package> {
    match input {
        Input::Path(p) => read_package(Box::new(std::fs::File::open(p)?), limit),
        other => read_package(Box::new(Cursor::new(other.into_bytes()?)), limit), // Bytes: no copy
    }
}
