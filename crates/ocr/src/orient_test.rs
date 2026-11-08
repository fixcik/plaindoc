use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::{OcrParams, ocr_image, scripted_recognition};

/// Box whose first edge runs bottom-to-top (`angle_deg` = -90).
fn vbox(x0: f32, y0: f32, x1: f32, y1: f32) -> TextBox {
    TextBox {
        quad: [[x0, y1], [x0, y0], [x1, y0], [x1, y1]],
        score: 0.9,
    }
}

/// A page whose text is vertical on portrait rasters and horizontal on landscape ones (a sideways
/// landscape page). Vertical boxes are read with `flipped = flip(i)` (true: the text runs top-to-bottom).
struct Sideways {
    n: usize,
    conf: f32,
    text: &'static str,
    flip: fn(usize) -> bool,
    /// The detector finds vertical boxes whatever the raster (the turn is then rejected).
    always_vertical: bool,
    recognized: AtomicUsize,
    detected: AtomicUsize,
}

fn sideways(flip: fn(usize) -> bool) -> Sideways {
    Sideways {
        n: 5,
        conf: 0.95,
        text: "вертикальная строка",
        flip,
        always_vertical: false,
        recognized: AtomicUsize::new(0),
        detected: AtomicUsize::new(0),
    }
}

impl OcrEngine for Sideways {
    fn detect(&self, img: &Image) -> Result<Vec<TextBox>, OcrError> {
        self.detected.fetch_add(1, Ordering::Relaxed);
        let s = 60.0;
        Ok((0..self.n)
            .map(|i| {
                let a = 50.0 + s * i as f32;
                if img.height() > img.width() || self.always_vertical {
                    vbox(a, 50.0, a + 30.0, 450.0)
                } else {
                    TextBox::from_aabb(50.0, a, 450.0, a + 30.0, 0.9)
                }
            })
            .collect())
    }
    fn recognize(&self, _img: &Image, b: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        self.recognized.fetch_add(b.len(), Ordering::Relaxed);
        Ok(b.iter()
            .enumerate()
            .map(|(i, b)| {
                let mut r = scripted_recognition(self.text, self.conf);
                r.flipped = b.is_vertical() && (self.flip)(i);
                r
            })
            .collect())
    }
}

fn params() -> OcrParams {
    OcrParams {
        blank_check: false,
        deskew: false,
        ..Default::default()
    }
}

fn portrait() -> Image {
    Image::from_pixel(500, 900, image::Rgb([255; 3]))
}

#[test]
fn sideways_page_is_turned_by_the_reading_direction() {
    for (flip, rotation) in [(|_| true, 270), (|_| false, 90)] as [(fn(usize) -> bool, u32); 2] {
        let e = sideways(flip);
        let p = ocr_image(&e, &portrait(), &params()).unwrap();
        assert_eq!(p.rotation, rotation);
        assert_eq!(
            (p.width, p.height),
            (900, 500),
            "boxes in the turned raster"
        );
        assert_eq!(
            (p.frame.src_w, p.frame.src_h, p.frame.rotation),
            (500, 900, rotation)
        );
        assert_eq!(p.lines.len(), 5);
        assert!(p.mean_conf > 0.9);
        // One det on the source, one on the turned raster; the vote reads the 5 boxes once.
        assert_eq!(e.detected.load(Ordering::Relaxed), 2);
        assert_eq!(e.recognized.load(Ordering::Relaxed), 10);
    }
}

#[test]
fn few_unsure_or_mixed_vertical_boxes_do_not_turn() {
    let few = Sideways {
        n: 2,
        ..sideways(|_| true)
    };
    let unsure = Sideways {
        conf: 0.5,
        ..sideways(|_| true)
    };
    let short = Sideways {
        text: "12",
        ..sideways(|_| true)
    };
    let mixed = sideways(|i| i % 2 == 0);
    for (name, e) in [
        ("few", few),
        ("unsure", unsure),
        ("short", short),
        ("mixed", mixed),
    ] {
        let p = ocr_image(&e, &portrait(), &params()).unwrap();
        assert_eq!(p.rotation, 0, "{name}");
        assert_eq!((p.width, p.height), (500, 900), "{name}");
        assert_eq!(
            e.detected.load(Ordering::Relaxed),
            1,
            "{name}: no det on a turned raster"
        );
    }
}

#[test]
fn turn_is_dropped_when_the_turned_page_is_not_horizontal() {
    let e = Sideways {
        always_vertical: true,
        ..sideways(|_| true)
    };
    let p = ocr_image(&e, &portrait(), &params()).unwrap();
    assert_eq!((p.rotation, p.skew_deg, p.width), (0, 0.0, 500));
    assert_eq!(e.detected.load(Ordering::Relaxed), 2);
}

#[test]
fn upright_page_pays_for_no_orientation_probe() {
    // Landscape raster: horizontal boxes read with high confidence; plus a page with two vertical labels.
    let e = sideways(|_| true);
    let img = Image::from_pixel(900, 500, image::Rgb([255; 3]));
    let p = ocr_image(&e, &img, &params()).unwrap();
    assert_eq!(p.rotation, 0);
    assert_eq!(e.detected.load(Ordering::Relaxed), 1);
    assert_eq!(
        e.recognized.load(Ordering::Relaxed),
        5,
        "only the page itself is read"
    );
}

#[test]
fn unsure_horizontal_page_is_probed_upside_down_but_kept() {
    // Reads the same both ways at 0.5: probed (5 + 5 boxes) but not turned.
    let e = Sideways {
        conf: 0.5,
        ..sideways(|_| true)
    };
    let img = Image::from_pixel(900, 500, image::Rgb([255; 3]));
    let p = ocr_image(&e, &img, &params()).unwrap();
    assert_eq!(p.rotation, 0);
    assert_eq!(e.recognized.load(Ordering::Relaxed), 15);
}

#[test]
fn vote_reads_the_direction_from_angle_and_flip() {
    let img = portrait();
    let down = |flipped: bool| {
        struct One(bool);
        impl OcrEngine for One {
            fn detect(&self, _: &Image) -> Result<Vec<TextBox>, OcrError> {
                Ok(vec![])
            }
            fn recognize(&self, _: &Image, b: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
                Ok(b.iter()
                    .map(|_| Recognized {
                        flipped: self.0,
                        ..scripted_recognition("вертикальная строка", 0.9)
                    })
                    .collect())
            }
        }
        One(flipped)
    };
    // quad[0] -> quad[1] running down (angle +90): unflipped text runs top-to-bottom -> undo with 270.
    let downward: Vec<TextBox> = (0..3)
        .map(|i| {
            let a = 50.0 + 60.0 * i as f32;
            TextBox {
                quad: [[a + 30.0, 50.0], [a + 30.0, 450.0], [a, 450.0], [a, 50.0]],
                score: 0.9,
            }
        })
        .collect();
    assert!(downward[0].angle_deg() > 0.0 && downward[0].is_vertical());
    assert_eq!(sideways_turn(&down(false), &img, &downward), Some(270));
    assert_eq!(sideways_turn(&down(true), &img, &downward), Some(90));
    assert_eq!(
        sideways_turn(&down(false), &img, &downward[..2]),
        None,
        "too few boxes"
    );
}

fn sideways_turn(e: &dyn OcrEngine, img: &Image, b: &[TextBox]) -> Option<u32> {
    super::sideways(e, img, b).unwrap()
}

#[test]
fn area_share_and_mean_conf_handle_empty_input() {
    assert_eq!(area_share(&[], true), 0.0);
    assert_eq!(mean_conf(&[]), 0.0);
    let h = TextBox::from_aabb(0.0, 0.0, 100.0, 10.0, 0.9);
    let v = vbox(0.0, 0.0, 10.0, 100.0);
    assert!((area_share(&[h, v, v, v], true) - 0.75).abs() < 1e-6);
}

#[test]
fn single_glyphs_reading_better_upside_down_do_not_turn() {
    // A sparse page of one-letter cells: the flipped reading wins on every probe but has too few chars.
    struct Glyphs;
    impl OcrEngine for Glyphs {
        fn detect(&self, _: &Image) -> Result<Vec<TextBox>, OcrError> {
            Ok((0..5)
                .map(|i| {
                    TextBox::from_aabb(
                        50.0 + 40.0 * i as f32,
                        50.0,
                        70.0 + 40.0 * i as f32,
                        80.0,
                        0.9,
                    )
                })
                .collect())
        }
        fn recognize(&self, img: &Image, b: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
            let up = img.get_pixel(5, 5)[0] < 128;
            Ok(b.iter()
                .map(|_| scripted_recognition("а", if up { 0.95 } else { 0.4 }))
                .collect())
        }
    }
    let mut img = Image::from_pixel(900, 500, image::Rgb([255; 3]));
    for y in 490..500 {
        for x in 890..900 {
            img.put_pixel(x, y, image::Rgb([0; 3]));
        }
    } // marker bottom-right: the page reads better upside down
    assert_eq!(ocr_image(&Glyphs, &img, &params()).unwrap().rotation, 0);
}

/// Upright table with turned column headers (review K1) on landscape rasters: six vertical headers over
/// six one-glyph cells; portrait rasters (the page turned) show horizontal boxes. A cell reads `cell` as it
/// stands and `cell_turned` when read along y (a vertical quad over the cell).
struct Matrix {
    header: &'static str,
    cell: f32,
    cell_turned: f32,
    detected: AtomicUsize,
}

fn matrix(header: &'static str, cell: f32, cell_turned: f32) -> Matrix {
    Matrix {
        header,
        cell,
        cell_turned,
        detected: AtomicUsize::new(0),
    }
}

impl OcrEngine for Matrix {
    fn detect(&self, img: &Image) -> Result<Vec<TextBox>, OcrError> {
        self.detected.fetch_add(1, Ordering::Relaxed);
        let x = |i: usize| 50.0 + 60.0 * i as f32;
        if img.height() > img.width() {
            return Ok((0..6)
                .map(|i| TextBox::from_aabb(50.0, x(i), 450.0, x(i) + 30.0, 0.9))
                .collect());
        }
        let headers = (0..6).map(|i| vbox(x(i), 50.0, x(i) + 30.0, 450.0));
        let cells = (0..6).map(|i| TextBox::from_aabb(x(i), 470.0, x(i) + 30.0, 490.0, 0.9));
        Ok(headers.chain(cells).collect())
    }
    fn recognize(&self, _img: &Image, b: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        Ok(b.iter()
            .map(|b| {
                let [_, y0, _, y1] = b.aabb();
                match (b.is_vertical(), y1 - y0 > 100.0) {
                    (true, true) => scripted_recognition(self.header, 0.95),
                    (true, false) => scripted_recognition("3", self.cell_turned),
                    _ => scripted_recognition("3", self.cell),
                }
            })
            .collect())
    }
}

#[test]
fn vertical_text_on_an_upright_page_is_not_rotated() {
    let img = Image::from_pixel(900, 500, image::Rgb([255; 3]));
    let e = matrix("Соблюдение сроков поставки", 0.95, 0.3);
    let p = ocr_image(&e, &img, &params()).unwrap();
    assert_eq!(
        (p.rotation, p.width),
        (0, 900),
        "cells read upright: keep 0°"
    );
    assert_eq!(
        e.detected.load(Ordering::Relaxed),
        1,
        "no det on a turned raster"
    );
    // Cells that read better turned are fragments of turned text: the page is sideways.
    let e = matrix("Соблюдение сроков поставки", 0.3, 0.95);
    assert_eq!(ocr_image(&e, &img, &params()).unwrap().rotation, 90);
    // Long vertical lines are body text: turned even with upright-looking cells (a page whose table
    // headers were turned, then the whole page turned).
    let e = matrix(
        "Соблюдение сроков поставки товара по договору подряда",
        0.95,
        0.3,
    );
    assert_eq!(ocr_image(&e, &img, &params()).unwrap().rotation, 90);
}

#[test]
fn short_recognition_result_is_an_engine_error() {
    struct Short;
    impl OcrEngine for Short {
        fn detect(&self, _: &Image) -> Result<Vec<TextBox>, OcrError> {
            Ok((0..3)
                .map(|i| vbox(50.0 + 60.0 * i as f32, 50.0, 80.0 + 60.0 * i as f32, 450.0))
                .collect())
        }
        fn recognize(&self, _: &Image, _: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
            Ok(vec![scripted_recognition("строка", 0.9)])
        }
    }
    let r = ocr_image(&Short, &portrait(), &params());
    assert!(matches!(r, Err(OcrError::Engine(_))), "{r:?}");
}
