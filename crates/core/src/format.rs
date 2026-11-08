use serde::Serialize;
use std::fmt;

/// Document format.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Pdf,
    Doc,
    Docx,
    Xls,
    Xlsx,
    Ppt,
    Pptx,
    Odt,
    Ods,
    Rtf,
    Csv,
    Tsv,
    Txt,
    Md,
    Html,
    Xml,
    Png,
    Jpeg,
    Tiff,
    Bmp,
    Webp,
    Msg,
    Unknown,
}

impl Format {
    /// Lowercase name, as used in JSON and CLI output (e.g. `"docx"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Format::Pdf => "pdf",
            Format::Doc => "doc",
            Format::Docx => "docx",
            Format::Xls => "xls",
            Format::Xlsx => "xlsx",
            Format::Ppt => "ppt",
            Format::Pptx => "pptx",
            Format::Odt => "odt",
            Format::Ods => "ods",
            Format::Rtf => "rtf",
            Format::Csv => "csv",
            Format::Tsv => "tsv",
            Format::Txt => "txt",
            Format::Md => "md",
            Format::Html => "html",
            Format::Xml => "xml",
            Format::Png => "png",
            Format::Jpeg => "jpeg",
            Format::Tiff => "tiff",
            Format::Bmp => "bmp",
            Format::Webp => "webp",
            Format::Msg => "msg",
            Format::Unknown => "unknown",
        }
    }

    /// Maps a file extension (case-insensitive) to a format.
    pub fn from_extension(ext: &str) -> Option<Format> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "pdf" => Format::Pdf,
            "doc" => Format::Doc,
            "docx" => Format::Docx,
            "xls" => Format::Xls,
            "xlsx" => Format::Xlsx,
            "ppt" => Format::Ppt,
            "pptx" => Format::Pptx,
            "odt" => Format::Odt,
            "ods" => Format::Ods,
            "rtf" => Format::Rtf,
            "csv" => Format::Csv,
            "tsv" | "tab" => Format::Tsv,
            "txt" | "text" => Format::Txt,
            "md" | "markdown" => Format::Md,
            "html" | "htm" => Format::Html,
            "xml" => Format::Xml,
            "png" => Format::Png,
            "jpg" | "jpeg" => Format::Jpeg,
            "tif" | "tiff" => Format::Tiff,
            "bmp" => Format::Bmp,
            "webp" => Format::Webp,
            "msg" => Format::Msg,
            _ => return None,
        })
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
