//! Core model, contracts and exporters of plaindoc.

mod format;
pub use format::Format;

mod image;
mod images;
pub use image::{ImageInfo, ImageOptions, StoredImage, image_set_id, sniff_image};
pub use images::collect_images;
mod model;
pub use model::{
    BBox, Block, Cell, CellValue, Document, Level, Link, ListItem, Metadata, NoteKind, ScalarValue,
    Section, SectionKind, TextSource, Warning, WarningKind,
};

mod error;
mod input;
mod options;
pub use error::{ParseError, Result};
pub use input::{Input, Parser};
pub use options::{HiddenText, OcrMode, ParseOptions, QualityWeights};

mod quality;
pub use quality::{apply_quality, section_penalty};

mod guard;
pub mod telemetry;
pub use guard::{catch_parse, catch_parse_with_stats, run_parser, run_parser_with_stats};

mod chunks;
mod markdown;
mod md_alt;
mod md_images;
mod md_links;
pub use markdown::{MarkdownOptions, PageSeparator};
mod detect;
pub use detect::{CFB_MAGIC, DETECT_HEAD_LEN, classify_cfb_streams, detect};

mod stats;
pub use stats::{
    ImageCounts, LevelCounts, LimitKind, Outcome, PageCounts, ParseStats, Sizes, Stage,
};
