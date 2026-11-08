//! `\pict` groups: the hex picture data collected while the group is open.

use plaindoc_core::{Block, ImageInfo};

use crate::picture_data::from_hex;

/// One open `\pict` group.
pub(super) struct Pict {
    /// Hex digits seen so far (other bytes dropped).
    hex: Vec<u8>,
    /// Largest picture taken, in bytes.
    max: u64,
    over: bool,
    /// Data given as `\binN` raw bytes (skipped by the lexer).
    bin: bool,
}

impl Pict {
    pub(super) fn new(max: u64) -> Self {
        Pict {
            hex: Vec::new(),
            max,
            over: false,
            bin: false,
        }
    }

    pub(super) fn word(&mut self, name: &str) {
        if name == "bin" {
            self.bin = true;
        }
    }

    pub(super) fn text(&mut self, t: &[u8]) {
        if self.over {
            return;
        }
        self.hex
            .extend(t.iter().copied().filter(u8::is_ascii_hexdigit));
        if self.hex.len() as u64 / 2 > self.max {
            self.over = true;
            self.hex = Vec::new();
        }
    }

    /// The image block of the closed group.
    pub(super) fn finish(self) -> Block {
        let info = if self.over {
            ImageInfo::skipped("too_large", None)
        } else if self.bin {
            ImageInfo::skipped("no_data", None)
        } else {
            match from_hex(&self.hex, self.max) {
                Some(b) if !b.is_empty() => ImageInfo::embedded(b, None),
                _ => ImageInfo::skipped("no_data", None),
            }
        };
        Block::Image {
            alt: None,
            ocr_text: None,
            bbox: None,
            image: Some(Box::new(info)),
        }
    }
}
