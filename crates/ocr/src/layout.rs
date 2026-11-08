//! Reading order: recursive XY-cut into blocks, then lines by y-centre inside a block.

/// `[x0, y0, x1, y1]` boxes of line fragments (det boxes).
pub type Rect = [f32; 4];

/// Cut thresholds in units of the median fragment height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutParams {
    /// Horizontal whitespace band (between rows) that splits blocks.
    pub y_gap: f32,
    /// Vertical whitespace band (between columns) that splits blocks.
    pub x_gap: f32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            y_gap: 1.2,
            x_gap: 1.5,
        }
    }
}

fn median_height(boxes: &[Rect]) -> f32 {
    let mut h: Vec<f32> = boxes.iter().map(|b| b[3] - b[1]).collect();
    h.sort_by(f32::total_cmp);
    h.get(h.len() / 2).copied().unwrap_or(1.0).max(1.0)
}

/// Largest gap `(size, cut)` between merged intervals, if any.
fn widest_gap(mut iv: Vec<(f32, f32)>) -> Option<(f32, f32)> {
    iv.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut end = iv.first()?.1;
    let mut best: Option<(f32, f32)> = None;
    for &(a, b) in &iv[1..] {
        if a > end {
            let g = a - end;
            if best.is_none_or(|(bg, _)| g > bg) {
                best = Some((g, end + g / 2.0));
            }
        }
        end = end.max(b);
    }
    best
}

/// Blocks in reading order, each a list of indices into `boxes`. Row cuts (y) win over column
/// cuts (x) at the same level, so a full-width title is split off before the columns.
/// Iterative (heap work stack), so depth does not depend on the row count.
pub fn xy_cut(boxes: &[Rect], p: &LayoutParams) -> Vec<Vec<usize>> {
    let mh = median_height(boxes);
    let mut out = Vec::new();
    let mut stack = vec![(0..boxes.len()).collect::<Vec<usize>>()];
    while let Some(idx) = stack.pop() {
        if idx.len() <= 1 {
            if !idx.is_empty() {
                out.push(idx);
            }
            continue;
        }
        match split(boxes, &idx, mh, p) {
            Some((a, b)) => {
                // Pop order = reading order: first half is processed before the second.
                stack.push(b);
                stack.push(a);
            }
            None => out.push(idx),
        }
    }
    out
}

/// Splits `idx` by the y gap, else the x gap. `None` when no gap is wide enough or a half would
/// be empty (inverted or extreme boxes), which keeps the caller from looping forever.
fn split(
    boxes: &[Rect],
    idx: &[usize],
    mh: f32,
    p: &LayoutParams,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let ys = idx.iter().map(|&i| (boxes[i][1], boxes[i][3])).collect();
    if let Some((g, at)) = widest_gap(ys)
        && g >= p.y_gap * mh
    {
        let (a, b): (Vec<usize>, Vec<usize>) = idx.iter().partition(|&&i| boxes[i][3] <= at);
        if !a.is_empty() && !b.is_empty() {
            return Some((a, b));
        }
    }
    let xs = idx.iter().map(|&i| (boxes[i][0], boxes[i][2])).collect();
    if let Some((g, at)) = widest_gap(xs)
        && g >= p.x_gap * mh
    {
        let (a, b): (Vec<usize>, Vec<usize>) = idx.iter().partition(|&&i| boxes[i][2] <= at);
        if !a.is_empty() && !b.is_empty() {
            return Some((a, b));
        }
    }
    None
}

/// Lines inside one block: fragments sorted by y-centre; a fragment joins the current line when
/// its centre is within half the line's first fragment height. Fragments in a line sorted by x.
pub fn lines_in_block(boxes: &[Rect], block: &[usize]) -> Vec<Vec<usize>> {
    let cy = |i: usize| (boxes[i][1] + boxes[i][3]) / 2.0;
    let mut idx = block.to_vec();
    idx.sort_by(|&a, &b| {
        cy(a)
            .total_cmp(&cy(b))
            .then(boxes[a][0].total_cmp(&boxes[b][0]))
    });
    let mut lines: Vec<Vec<usize>> = Vec::new();
    for i in idx {
        let joins = lines.last().is_some_and(|l| {
            let f = boxes[l[0]];
            (cy(i) - (f[1] + f[3]) / 2.0).abs() < (f[3] - f[1]) * 0.5
        });
        if joins {
            lines.last_mut().expect("checked").push(i);
        } else {
            lines.push(vec![i]);
        }
    }
    for l in &mut lines {
        l.sort_by(|&a, &b| boxes[a][0].total_cmp(&boxes[b][0]));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        [x0, y0, x1, y1]
    }

    #[test]
    fn two_columns_under_a_title_read_title_left_right() {
        let b = vec![
            r(100.0, 10.0, 900.0, 30.0),  // 0 title
            r(520.0, 60.0, 900.0, 80.0),  // 1 right col line 1
            r(100.0, 60.0, 480.0, 80.0),  // 2 left col line 1
            r(100.0, 86.0, 480.0, 106.0), // 3 left col line 2
            r(520.0, 86.0, 900.0, 106.0), // 4 right col line 2
        ];
        let blocks = xy_cut(&b, &LayoutParams::default());
        assert_eq!(blocks, vec![vec![0], vec![2, 3], vec![1, 4]]);
    }

    #[test]
    fn fragments_on_one_row_form_one_line_left_to_right() {
        let b = vec![
            r(300.0, 12.0, 400.0, 30.0),
            r(10.0, 10.0, 200.0, 30.0),
            r(10.0, 40.0, 200.0, 60.0),
        ];
        assert_eq!(lines_in_block(&b, &[0, 1, 2]), vec![vec![1, 0], vec![2]]);
    }

    #[test]
    fn single_and_empty_inputs() {
        assert_eq!(
            xy_cut(&[r(0.0, 0.0, 1.0, 1.0)], &LayoutParams::default()),
            vec![vec![0]]
        );
        assert!(xy_cut(&[], &LayoutParams::default()).is_empty());
    }

    fn all_indices(blocks: &[Vec<usize>]) -> Vec<usize> {
        let mut v: Vec<usize> = blocks.iter().flatten().copied().collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn inverted_huge_and_nan_boxes_terminate_and_keep_every_index() {
        let p = LayoutParams::default();
        for b in [
            vec![r(0.0, 0.0, 10.0, 10.0), r(0.0, 40.0, 10.0, 5.0)],
            vec![
                r(0.0, 2.5e38, 10.0, 2.5e38 + 1e32),
                r(0.0, 3e38, 10.0, 3e38 + 1e32),
            ],
            vec![
                r(f32::NAN, f32::NAN, f32::NAN, f32::NAN),
                r(0.0, 0.0, 10.0, 10.0),
                r(0.0, 50.0, 10.0, f32::NAN),
            ],
        ] {
            let blocks = xy_cut(&b, &p);
            assert_eq!(all_indices(&blocks), (0..b.len()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn two_thousand_lines_on_a_small_stack() {
        let b: Vec<Rect> = (0..2000)
            .map(|i| r(0.0, i as f32 * 50.0, 100.0, i as f32 * 50.0 + 20.0))
            .collect();
        let blocks = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || xy_cut(&b, &LayoutParams::default()))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(blocks.len(), 2000);
        assert!(blocks.iter().enumerate().all(|(i, bl)| bl == &vec![i]));
    }
}
