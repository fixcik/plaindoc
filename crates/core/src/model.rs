use crate::{Format, ImageInfo, StoredImage};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Document {
    pub metadata: Metadata,
    pub sections: Vec<Section>,
    pub warnings: Vec<Warning>,
    pub quality: f32,
    /// Distinct pictures in document order, as named by [`ImageInfo::file`]; not in JSON.
    /// Empty when `ImageOptions::keep_data` is off.
    #[serde(skip)]
    pub images: Vec<StoredImage>,
}

impl Document {
    pub fn new(sections: Vec<Section>) -> Self {
        Self {
            metadata: Metadata::default(),
            sections,
            warnings: Vec::new(),
            quality: 1.0,
            images: Vec::new(),
        }
    }

    /// Pretty-printed JSON, without a trailing newline.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = Vec::new();
        self.write_json(&mut out)
            .expect("Document is always serializable");
        String::from_utf8(out).expect("JSON is UTF-8")
    }

    /// Writes the same pretty-printed JSON as [`Document::to_json`] into `out`, without holding the
    /// whole text in memory. `out` gets the text in chunks of about 64 KiB, so it needs no buffer of
    /// its own. Nothing is flushed. The first write error is returned with its kind kept (e.g.
    /// `BrokenPipe`) once the document is rendered; nothing after it is written, what was written
    /// before it stays in `out`.
    pub fn write_json<W: std::io::Write + ?Sized>(&self, out: &mut W) -> std::io::Result<()> {
        let mut chunks = crate::chunks::Chunks::new(out);
        serde_json::to_writer_pretty(&mut chunks, self).map_err(std::io::Error::from)?;
        chunks.finish()
    }
}

impl Default for Document {
    fn default() -> Self {
        Document::new(Vec::new())
    }
}

/// К-3: `format` заполняет Registry.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Metadata {
    pub format: Option<Format>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub page_count: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SectionKind {
    Page,
    Sheet,
    Slide,
    Body,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TextSource {
    TextLayer,
    Ocr,
    Mixed,
}

/// К-8: `index` считается с 0; Markdown показывает `index + 1`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Section {
    pub kind: SectionKind,
    pub index: usize,
    pub name: Option<String>,
    pub blocks: Vec<Block>,
    pub warnings: Vec<Warning>,
    pub quality: f32,
    pub source: TextSource,
}

impl Section {
    pub fn new(kind: SectionKind, index: usize) -> Self {
        Self {
            kind,
            index,
            name: None,
            blocks: Vec::new(),
            warnings: Vec::new(),
            quality: 1.0,
            source: TextSource::TextLayer,
        }
    }
}

/// К-16: JSON-форма `{"type": "heading", ...}`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Block {
    Heading {
        level: u8,
        text: String,
        bbox: Option<BBox>,
        /// Hyperlinks over parts of `text`; omitted from JSON when empty.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        links: Vec<Link>,
    },
    Paragraph {
        text: String,
        bbox: Option<BBox>,
        /// Hyperlinks over parts of `text`; omitted from JSON when empty.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        links: Vec<Link>,
    },
    List {
        ordered: bool,
        items: Vec<ListItem>,
        bbox: Option<BBox>,
    },
    Table {
        rows: Vec<Vec<Cell>>,
        header_rows: usize,
        bbox: Option<BBox>,
    },
    Image {
        alt: Option<String>,
        ocr_text: Option<String>,
        bbox: Option<BBox>,
        /// The picture (bytes stored as `Document::images`, or an external target); omitted from
        /// JSON when the source gave none or pictures are off.
        #[serde(skip_serializing_if = "Option::is_none")]
        image: Option<Box<ImageInfo>>,
    },
    Code {
        lang: Option<String>,
        text: String,
        bbox: Option<BBox>,
    },
    Note {
        kind: NoteKind,
        text: String,
        bbox: Option<BBox>,
    },
}

impl Block {
    /// An image block without its picture.
    pub fn image(alt: Option<String>, ocr_text: Option<String>, bbox: Option<BBox>) -> Self {
        Block::Image {
            alt,
            ocr_text,
            bbox,
            image: None,
        }
    }

    /// A paragraph without links.
    pub fn paragraph(text: impl Into<String>, bbox: Option<BBox>) -> Self {
        Block::Paragraph {
            text: text.into(),
            bbox,
            links: Vec::new(),
        }
    }

    /// A heading without links.
    pub fn heading(level: u8, text: impl Into<String>, bbox: Option<BBox>) -> Self {
        Block::Heading {
            level,
            text: text.into(),
            bbox,
            links: Vec::new(),
        }
    }
}

/// A hyperlink over `text[start..end]` of a heading or paragraph. `start` and `end` count chars
/// (Unicode scalar values), not bytes; ranges of one block are sorted and do not overlap. `url`
/// is an absolute `http`, `https` or `mailto` URL as the source gave it, cleaned: control and
/// format chars removed, trimmed, the scheme lower-cased (no percent-encoding added; Markdown
/// output encodes it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub url: String,
}

impl Link {
    pub fn new(start: usize, end: usize, url: impl Into<String>) -> Self {
        Self {
            start,
            end,
            url: url.into(),
        }
    }
}

/// D5: содержимое пункта по порядку; первый `Paragraph` = текст пункта;
/// вложенный список = `Block::List` со своим `ordered`; допускаются таблицы и код.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ListItem {
    pub blocks: Vec<Block>,
}

impl ListItem {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            blocks: text_blocks(text.into()),
        }
    }

    pub fn text(&self) -> String {
        plain_text(&self.blocks)
    }
}

/// D2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NoteKind {
    Header,
    Footer,
    Footnote,
    Endnote,
    SpeakerNotes,
    Comment,
    PageNumber,
    Marginal,
}

/// D5, К-6: содержимое ячейки по порядку; в строках только исходные ячейки,
/// позиции под span не повторяются (модель HTML).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Cell {
    pub blocks: Vec<Block>,
    pub colspan: u32,
    pub rowspan: u32,
    pub value: Option<CellValue>,
}

impl Cell {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            blocks: text_blocks(text.into()),
            colspan: 1,
            rowspan: 1,
            value: None,
        }
    }

    pub fn with_span(mut self, colspan: u32, rowspan: u32) -> Self {
        self.colspan = colspan.max(1);
        self.rowspan = rowspan.max(1);
        self
    }

    pub fn is_merged(&self) -> bool {
        self.colspan > 1 || self.rowspan > 1
    }

    pub fn text(&self) -> String {
        plain_text(&self.blocks)
    }

    /// В `blocks` только `Paragraph` (годится для GFM).
    pub fn is_simple(&self) -> bool {
        self.blocks
            .iter()
            .all(|b| matches!(b, Block::Paragraph { .. }))
    }
}

/// D5: скаляр как в источнике.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ScalarValue {
    Number(f64),
    Text(String),
    Bool(bool),
    /// Серийная дата таблиц (дни; система 1900, если источник не сказал иного).
    Date(f64),
    /// Код ошибки, например "#DIV/0!".
    Error(String),
}

/// `value` = литерал или кэшированный результат `formula`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub struct CellValue {
    pub value: Option<ScalarValue>,
    /// Без ведущего '='.
    pub formula: Option<String>,
    /// Например "0.00%" или "dd.mm.yyyy".
    pub number_format: Option<String>,
}

impl CellValue {
    pub fn new(value: Option<ScalarValue>) -> Self {
        Self {
            value,
            formula: None,
            number_format: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub struct BBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl BBox {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Level {
    Info,
    Warn,
    Loss,
}

/// Сериализуется как есть: "TableUncertain".
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[non_exhaustive]
pub enum WarningKind {
    NoTextLayer,
    ImageTextPage,
    SuspectedGarbage,
    PartialContent,
    Truncated,
    Encrypted,
    OcrUsed,
    LowOcrConfidence,
    OcrEmpty,
    OcrUnavailable,
    TableUncertain,
    FormulaSkipped,
    EmptyOutput,
    HiddenContent,
    ContentDropped,
}

impl WarningKind {
    /// The serialized name (`"TableUncertain"`): a code for statistics and telemetry.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            WarningKind::NoTextLayer => "NoTextLayer",
            WarningKind::ImageTextPage => "ImageTextPage",
            WarningKind::SuspectedGarbage => "SuspectedGarbage",
            WarningKind::PartialContent => "PartialContent",
            WarningKind::Truncated => "Truncated",
            WarningKind::Encrypted => "Encrypted",
            WarningKind::OcrUsed => "OcrUsed",
            WarningKind::LowOcrConfidence => "LowOcrConfidence",
            WarningKind::OcrEmpty => "OcrEmpty",
            WarningKind::OcrUnavailable => "OcrUnavailable",
            WarningKind::TableUncertain => "TableUncertain",
            WarningKind::FormulaSkipped => "FormulaSkipped",
            WarningKind::EmptyOutput => "EmptyOutput",
            WarningKind::HiddenContent => "HiddenContent",
            WarningKind::ContentDropped => "ContentDropped",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Warning {
    pub kind: WarningKind,
    pub level: Level,
    pub message: String,
    /// Machine-readable rule code within `kind` (e.g. `"tiny"` for `HiddenContent`); omitted from
    /// JSON when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
}

impl Warning {
    pub fn new(kind: WarningKind, level: Level, message: impl Into<String>) -> Self {
        Self {
            kind,
            level,
            message: message.into(),
            code: None,
        }
    }
}

fn plain_text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn text_blocks(text: String) -> Vec<Block> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![Block::paragraph(text, None)]
    }
}
