//! Cheap blank-page rule (OCR spike: zero misses on all three labelled sets).

use crate::Image;

/// Gray statistics on a copy downscaled to 600 px on the long side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlankStats {
    /// Share of horizontally adjacent pixel pairs differing by more than 60 levels.
    pub edge_density: f32,
    /// Standard deviation of gray levels.
    pub std: f32,
}

pub fn blank_stats(img: &Image) -> BlankStats {
    let g = image::imageops::grayscale(img);
    let (w, h) = g.dimensions();
    let s = 600.0 / w.max(h).max(1) as f32;
    let g = image::imageops::resize(
        &g,
        ((w as f32 * s) as u32).max(1),
        ((h as f32 * s) as u32).max(1),
        image::imageops::FilterType::Triangle,
    );
    let (w, h) = g.dimensions();
    let n = (w * h) as f64;
    let (mut sum, mut sum2, mut edges) = (0f64, 0f64, 0u64);
    for y in 0..h {
        for x in 0..w {
            let v = g.get_pixel(x, y)[0] as f64;
            sum += v;
            sum2 += v * v;
            if x + 1 < w && (g.get_pixel(x + 1, y)[0] as i32 - v as i32).abs() > 60 {
                edges += 1;
            }
        }
    }
    let mean = sum / n;
    BlankStats {
        edge_density: (edges as f64 / n) as f32,
        std: (sum2 / n - mean * mean).max(0.0).sqrt() as f32,
    }
}

/// `edge_density < 0.0003 && std < 3`: white sheet, noise+JPEG, grey with dust.
pub fn is_blank(img: &Image) -> bool {
    let s = blank_stats(img);
    s.edge_density < 0.0003 && s.std < 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_page_is_blank_and_a_text_line_is_not() {
        let mut img = Image::from_pixel(1240, 1754, image::Rgb([250, 250, 250]));
        assert!(is_blank(&img));
        for x in (100..1100).step_by(12) {
            for y in 300..330 {
                for dx in 0..6 {
                    img.put_pixel(x + dx, y, image::Rgb([20, 20, 20]));
                }
            }
        }
        assert!(!is_blank(&img));
    }
}
