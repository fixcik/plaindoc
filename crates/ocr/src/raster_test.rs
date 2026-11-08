use super::*;

fn patterned(w: u32, h: u32) -> GrayImage {
    GrayImage::from_fn(w, h, |x, y| {
        image::Luma([((x * 37 + y * 91 + x * y * 13) % 251) as u8])
    })
}

fn window(n: u32, i: u32, k: u32) -> impl Iterator<Item = i64> {
    let r = i64::from(k / 2);
    let i = i64::from(i);
    (i - r..i - r + i64::from(k)).filter(move |&j| j >= 0 && j < i64::from(n))
}

fn naive_extremum(g: &GrayImage, k: u32, max: bool) -> GrayImage {
    GrayImage::from_fn(g.width(), g.height(), |x, y| {
        let vals = window(g.width(), x, k)
            .flat_map(|xx| window(g.height(), y, k).map(move |yy| (xx, yy)))
            .map(|(xx, yy)| g.get_pixel(xx as u32, yy as u32)[0]);
        image::Luma([if max {
            vals.max().unwrap()
        } else {
            vals.min().unwrap()
        }])
    })
}

fn naive_blur(g: &GrayImage, k: u32) -> GrayImage {
    let refl = |j: i64, n: u32| -> u32 {
        let n = i64::from(n);
        let mut j = j;
        while j < 0 || j >= n {
            j = if j < 0 { -j } else { 2 * n - 2 - j };
        }
        j as u32
    };
    let r = i64::from(k / 2);
    GrayImage::from_fn(g.width(), g.height(), |x, y| {
        let mut s = 0u32;
        for dy in -r..i64::from(k) - r {
            for dx in -r..i64::from(k) - r {
                s += u32::from(
                    g.get_pixel(
                        refl(i64::from(x) + dx, g.width()),
                        refl(i64::from(y) + dy, g.height()),
                    )[0],
                );
            }
        }
        image::Luma([(s as f32 / (k * k) as f32).round() as u8])
    })
}

#[test]
fn raster_filters_match_brute_force() {
    for &(w, h) in &[(23u32, 17u32), (5, 9)] {
        let g = patterned(w, h);
        for k in [1u32, 3, 4, 7, 12] {
            let closed = naive_extremum(&naive_extremum(&g, k, true), k, false);
            assert_eq!(close_gray(&g, k), closed, "close {w}x{h} k={k}");
            assert_eq!(box_blur(&g, k), naive_blur(&g, k), "blur {w}x{h} k={k}");
        }
    }
}

// raster.rs: lines of dark dashes rotated by a known angle
pub(crate) fn lines_page(angle: f32) -> Image {
    let mut img = Image::from_pixel(1200, 1600, image::Rgb([255; 3]));
    for row in 0..40 {
        for x in (100..1100).step_by(14) {
            for y in 0..10 {
                for dx in 0..9 {
                    img.put_pixel(x + dx, 120 + row * 34 + y, image::Rgb([0; 3]));
                }
            }
        }
    }
    rotate(&img, angle, [255; 3])
}

/// The full 61-step scan of the research, for comparison with the coarse-to-fine search.
fn full_search(g: &GrayImage) -> f32 {
    let Some(mut s) = SkewScore::new(g) else {
        return 0.0;
    };
    let best = (-SKEW_STEPS..=SKEW_STEPS)
        .map(|i| Trial::at(s.score(i), i))
        .max()
        .unwrap();
    best.step as f32 * SKEW_STEP_DEG
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "slow in debug (~12 min); run with --release"
)]
fn coarse_to_fine_skew_equals_the_full_search() {
    for k in -62..=62 {
        let a = k as f32 * 0.05;
        let g = gray(&lines_page(a));
        assert_eq!(estimate_skew(&g), full_search(&g), "rotated {a}");
    }
}

/// Debug subset of the sweep: edges, coarse points, side peaks near 0, a point between.
#[test]
fn coarse_to_fine_skew_equals_the_full_search_on_key_angles() {
    for a in [
        -3.1f32, -3.0, 3.0, 3.1, -0.5, 0.5, -2.5, 2.5, -0.3, 0.3, -0.1, 0.0, 1.25,
    ] {
        let g = gray(&lines_page(a));
        assert_eq!(estimate_skew(&g), full_search(&g), "rotated {a}");
    }
}

#[test]
fn skew_is_found_within_a_tenth_of_a_degree() {
    for a in [-1.7f32, -0.6, 0.0, 1.2] {
        let est = estimate_skew(&gray(&lines_page(a)));
        assert!((est + a).abs() <= 0.15, "rotated {a}, estimated {est}"); // estimate = correction angle
    }
}

#[test]
fn rotate_follows_cv2_and_handles_degenerate_input() {
    // A dot at (150, 40) on a 200x100 page, turned by +10 deg (counter-clockwise on screen).
    let mut img = Image::from_pixel(200, 100, image::Rgb([255; 3]));
    for y in 38..43 {
        for x in 148..153 {
            img.put_pixel(x, y, image::Rgb([0; 3]));
        }
    }
    let r = rotate(&img, 10.0, [255; 3]);
    let (s, c) = 10f32.to_radians().sin_cos();
    let (x, y) = (150.0 - 100.0, 40.0 - 50.0);
    let want = (100.0 + x * c + y * s, 50.0 - x * s + y * c);
    assert!(r.get_pixel(want.0.round() as u32, want.1.round() as u32)[0] < 64);
    assert_eq!(r.get_pixel(150, 40)[0], 255, "the dot moved");
    assert_eq!(rotate(&img, 0.0, [255; 3]), img);
    assert_eq!(
        *rotate(&img, f32::NAN, [7; 3]).get_pixel(3, 3),
        image::Rgb([7; 3])
    );
    assert_eq!(rotate(&Image::new(0, 0), 5.0, [0; 3]).dimensions(), (0, 0));
    let g = patterned(60, 40);
    let rgb = Image::from_fn(60, 40, |x, y| image::Rgb([g.get_pixel(x, y)[0]; 3]));
    assert_eq!(
        rotate_gray(&g, 7.0, 255),
        gray(&rotate(&rgb, 7.0, [255; 3]))
    );
    assert_eq!(rotate_gray(&g, 0.0, 255), g);
    // cv2.warpAffine(INTER_CUBIC, BORDER_CONSTANT 255) of the same pattern by 7°; cv2 quantises
    // coordinates to 1/32 px and weights to 15 bits, hence ±2. Its INTER_LINEAR values differ by 5..32.
    let r = rotate_gray(&g, 7.0, 255);
    for (x, y, want) in [
        (10, 10, 69),
        (30, 20, 188),
        (45, 12, 54),
        (20, 30, 208),
        (50, 33, 176),
        (5, 20, 77),
        (0, 0, 255),
        (59, 39, 255),
    ] {
        let got = i32::from(r.get_pixel(x, y)[0]);
        assert!((got - want).abs() <= 2, "({x}, {y}): {got} vs cv2 {want}");
    }
    assert_eq!(estimate_skew(&GrayImage::new(0, 0)), 0.0);
    assert_eq!(estimate_skew(&GrayImage::new(1, 1)), 0.0);
    assert_eq!(
        estimate_skew(&GrayImage::from_pixel(300, 300, image::Luma([255]))),
        0.0,
        "no ink, no skew"
    );
}

#[test]
fn raster_normalize_bg_flattens_uneven_light() {
    // Background brightens left to right; a dark dot must stay dark, the background must come out near 1.
    let mut g = GrayImage::from_fn(200, 60, |x, _| image::Luma([(140 + x / 2) as u8]));
    g.put_pixel(150, 30, image::Luma([40]));
    let n = normalize_bg(&g, 31);
    assert!(n.iter().all(|v| (0.0..=1.5).contains(v)));
    assert!((n[10 * 200 + 20] - 1.0).abs() < 0.05 && (n[10 * 200 + 180] - 1.0).abs() < 0.05);
    assert!(n[30 * 200 + 150] < 0.3);
}
