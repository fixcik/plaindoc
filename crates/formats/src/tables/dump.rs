//! Loader of the spike geometry dumps (`pdf-spike-segs dump`, bottom-left origin) for golden tests,
//! the evaluation harness and benches.

use super::{InputPath, InputSegment, InputWord, PageInput, Rect};
use serde_json::Value;

pub(crate) fn f(v: &Value) -> f64 {
    v.as_f64().expect("number")
}

/// pdf_oxide geometry dump page (`pdf-spike-segs dump`, bottom-left origin) → detector input.
pub fn page_from_dump(p: &Value) -> PageInput {
    let h = f(&p["h"]);
    let words = p["words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| InputWord {
            rect: Rect::new(f(&w[0]), h - (f(&w[1]) + f(&w[3])), f(&w[2]), f(&w[3])),
            text: w[4].as_str().unwrap().to_string(),
            font_size: f(&w[5]),
            char_x: Vec::new(),
            rotated: false,
        })
        .collect();
    let paths = p["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| InputPath {
            rect: Rect::new(f(&q[0]), h - (f(&q[1]) + f(&q[3])), f(&q[2]), f(&q[3])),
            stroke: f(&q[4]) != 0.0,
            fill: f(&q[5]) != 0.0,
        })
        .collect();
    let segments = p["segs"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|s| InputSegment {
                    x1: f(&s[0]),
                    y1: h - f(&s[1]),
                    x2: f(&s[2]),
                    y2: h - f(&s[3]),
                    raster: false,
                })
                .collect()
        })
        .unwrap_or_default();
    PageInput {
        width: f(&p["w"]),
        height: h,
        words,
        paths,
        segments,
        model_regions: Vec::new(),
        image_boxes: Vec::new(),
    }
}
