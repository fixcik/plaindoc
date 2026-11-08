//! Line box + CTC positions + ink profile -> words with boxes (port of ocr-tables `split_words`).

use crate::Recognized;

/// Binary ink map of the page (1 = ink), row-major.
pub struct InkMap<'a> {
    pub data: &'a [u8],
    pub width: usize,
    pub height: usize,
}

/// A word in pixels plus the line's estimated font size (ink height / 0.8).
#[derive(Clone, Debug, PartialEq)]
pub struct WordBox {
    pub text: String,
    pub bbox: [f32; 4],
    pub font_px: f32,
    /// Per-character x-ranges `(left, right)` in raster px, from the CTC steps of the characters.
    pub char_x: Vec<(f32, f32)>,
}

/// Runs of `profile > 0`; gaps shorter than `min_gap` are merged. Half-open `[a, b)`.
pub fn ink_runs(profile: &[u32], min_gap: f32) -> Vec<[usize; 2]> {
    let mut runs: Vec<[usize; 2]> = Vec::new();
    let mut start = None;
    for (x, &v) in profile.iter().enumerate() {
        match (v > 0, start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                runs.push([s, x]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        runs.push([s, profile.len()]);
    }
    let mut merged: Vec<[usize; 2]> = Vec::new();
    for r in runs {
        match merged.last_mut() {
            Some(m) if ((r[0] - m[1]) as f32) < min_gap => m[1] = r[1],
            _ => merged.push(r),
        }
    }
    merged
}

/// Splits one axis-aligned line box `[x0, y0, x1, y1]` (inclusive pixel bounds) into words.
/// Rows: the ink run nearest the box centre is the line. Words: space-separated CTC tokens; when
/// token and ink-run counts match they pair 1:1, otherwise a token takes the union of ink runs
/// overlapping its CTC range (padded by 0.3 x ink height), or the CTC range itself.
pub fn split_words(bx: [usize; 4], ink: &InkMap, rec: &Recognized) -> Vec<WordBox> {
    if ink.width == 0 || ink.height == 0 || ink.data.len() < ink.width * ink.height {
        return Vec::new();
    }
    let [x0, y0, x1, y1] = [
        bx[0].min(ink.width - 1),
        bx[1].min(ink.height - 1),
        bx[2].min(ink.width - 1),
        bx[3].min(ink.height - 1),
    ];
    if x1 < x0 || y1 < y0 || rec.text.is_empty() {
        return Vec::new();
    }
    let at = |x: usize, y: usize| ink.data[y * ink.width + x] as u32;
    let rows: Vec<u32> = (y0..=y1)
        .map(|y| (x0..=x1).map(|x| at(x, y)).sum())
        .collect();
    let runs_y = ink_runs(&rows, 3.0);
    let cy = (y1 - y0) as f32 / 2.0;
    let dist = |r: &[usize; 2]| {
        if (r[0] as f32) <= cy && cy <= r[1] as f32 {
            0.0
        } else {
            (r[0] as f32 - cy).abs().min((r[1] as f32 - cy).abs())
        }
    };
    // No ink in the box: fall back to the whole box.
    let ry = runs_y
        .iter()
        .min_by(|a, b| dist(a).total_cmp(&dist(b)))
        .copied()
        .unwrap_or([0, y1 - y0 + 1]);
    let (ty0, ty1) = (y0 + ry[0], y0 + ry[1]);
    let ih = ((ty1 - ty0) as f32).max(4.0);
    let font_px = ih / 0.8;
    let cols: Vec<u32> = (x0..=x1)
        .map(|x| (ty0..ty1).map(|y| at(x, y)).sum())
        .collect();
    let runs = ink_runs(&cols, (0.12 * ih).max(2.0));

    // Space-separated tokens with the CTC step of each char (never empty).
    let mut toks: Vec<(String, Vec<u32>)> = Vec::new();
    let mut cur = (String::new(), Vec::new());
    for (ch, &t) in rec.text.chars().zip(&rec.steps) {
        if ch != ' ' {
            cur.0.push(ch);
            cur.1.push(t);
        } else if !cur.0.is_empty() {
            toks.push(std::mem::take(&mut cur));
        }
    }
    if !cur.0.is_empty() {
        toks.push(cur);
    }
    let cw = (x1 + 1 - x0) as f32;
    let k = cw / (rec.width_frac.max(1e-3) * rec.total_steps.max(1) as f32);
    let mk = |(text, steps): (String, Vec<u32>), a: f32, b: f32| WordBox {
        text,
        bbox: [x0 as f32 + a, ty0 as f32, x0 as f32 + b, ty1 as f32],
        font_px,
        char_x: steps
            .iter()
            .map(|&t| (x0 as f32 + t as f32 * k, x0 as f32 + (t + 1) as f32 * k))
            .collect(),
    };
    if !toks.is_empty() && toks.len() == runs.len() {
        return toks
            .into_iter()
            .zip(runs)
            .map(|(t, r)| mk(t, r[0] as f32, r[1] as f32))
            .collect();
    }
    toks.into_iter()
        .map(|t| {
            let (sa, sb) = (t.1[0], t.1[t.1.len() - 1]);
            let (ta, tb) = (sa as f32 * k, (sb + 1) as f32 * k);
            let hit: Vec<&[usize; 2]> = runs
                .iter()
                .filter(|r| (r[1] as f32) > ta - 0.3 * ih && (r[0] as f32) < tb + 0.3 * ih)
                .collect();
            match (
                hit.iter().map(|r| r[0]).min(),
                hit.iter().map(|r| r[1]).max(),
            ) {
                (Some(l), Some(h)) => mk(t, l as f32, h as f32),
                _ => mk(t, ta.floor(), tb.floor()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::scripted_recognition;

    /// 100x20 page, ink rows 6..14, ink columns in runs.
    fn page(runs: &[(usize, usize)]) -> Vec<u8> {
        let mut d = vec![0u8; 100 * 20];
        for y in 6..14 {
            for &(a, b) in runs {
                for x in a..b {
                    d[y * 100 + x] = 1;
                }
            }
        }
        d
    }

    #[test]
    fn ink_runs_merge_small_gaps() {
        assert_eq!(
            ink_runs(&[0, 1, 1, 0, 1, 0, 0, 0, 1], 2.0),
            vec![[1, 5], [8, 9]]
        );
    }

    #[test]
    fn equal_token_and_run_counts_pair_directly() {
        let d = page(&[(10, 30), (40, 70)]);
        let ink = InkMap {
            data: &d,
            width: 100,
            height: 20,
        };
        let w = split_words([5, 2, 80, 17], &ink, &scripted_recognition("abc defg", 0.9));
        assert_eq!(w.len(), 2);
        assert_eq!(
            (w[0].text.as_str(), w[0].bbox),
            ("abc", [10.0, 6.0, 30.0, 14.0])
        );
        assert_eq!(w[1].bbox, [40.0, 6.0, 70.0, 14.0]);
        assert!((w[0].font_px - 10.0).abs() < 1e-6);
    }

    #[test]
    fn char_x_follow_ctc_steps() {
        let d = page(&[(10, 30), (40, 70)]);
        let ink = InkMap {
            data: &d,
            width: 100,
            height: 20,
        };
        let w = split_words([5, 2, 80, 17], &ink, &scripted_recognition("ab cd ef", 0.9));
        let k = 76.0 / 32.0; // box width / total steps (8 chars x 4)
        assert_eq!(w[1].char_x.len(), 2);
        // 'c' is char 3 -> step 13
        assert!((w[1].char_x[0].0 - (5.0 + 13.0 * k)).abs() < 1e-3);
        assert!((w[1].char_x[0].1 - (5.0 + 14.0 * k)).abs() < 1e-3);
        assert!(w[1].char_x[0].1 <= w[1].char_x[1].0 + 1e-3);
    }

    #[test]
    fn degenerate_input_does_not_panic() {
        let ink = InkMap {
            data: &[],
            width: 0,
            height: 0,
        };
        assert!(split_words([0, 0, 5, 5], &ink, &scripted_recognition("ab", 0.9)).is_empty());
        let d = vec![0u8; 100 * 20];
        let ink = InkMap {
            data: &d,
            width: 100,
            height: 20,
        };
        let w = split_words([5, 2, 80, 17], &ink, &scripted_recognition("ab cd", 0.9));
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn glued_cells_split_by_ctc_positions() {
        // 3 tokens, 2 ink runs: each token takes the runs its CTC range overlaps
        let d = page(&[(10, 30), (40, 70)]);
        let ink = InkMap {
            data: &d,
            width: 100,
            height: 20,
        };
        let w = split_words([5, 2, 80, 17], &ink, &scripted_recognition("ab cd ef", 0.9));
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].bbox, [10.0, 6.0, 30.0, 14.0]);
        assert_eq!((w[1].bbox[0], w[2].bbox[0]), (40.0, 40.0));
    }
}
