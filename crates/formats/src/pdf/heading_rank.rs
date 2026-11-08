//! Levels of style-detected headings by the rank of their size over the whole document (SP8b
//! S2c): near-equal sizes form one level, the largest first. Headings from the outline, numbered
//! headings and tagged ones keep their own levels. Ranking with merged near-equal sizes follows
//! Docling 2.148 (MIT; logic only, no code taken).

use plaindoc_core::{BBox, Block, Section};

/// Sizes within this share of the larger one are one level.
const SAME_SIZE: f32 = 0.05;
/// Deepest level given by rank (H4-H6 are out of scope).
const MAX_LEVEL: u8 = 3;

/// A heading of a page whose level comes from its size rank.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RankedHead {
    pub bbox: BBox,
    pub size: f32,
}

/// Lower bounds of the size levels, largest first: level `i + first` for sizes `>= bounds[i]`.
struct Ranks {
    bounds: Vec<f32>,
    first: u8,
}

impl Ranks {
    /// `first` is 1 when there are two sizes or more (the largest is a title) or the one size is
    /// `title_ratio` times the body, else 2: one body-sized heading style is sections.
    fn new(heads: &[Vec<RankedHead>], body: f32, title_ratio: f32) -> Self {
        let mut sizes: Vec<f32> = heads
            .iter()
            .flatten()
            .map(|h| h.size)
            .filter(|s| s.is_finite() && *s > 0.0)
            .collect();
        sizes.sort_by(|a, b| b.total_cmp(a));
        // a group: sizes within SAME_SIZE of its largest one; the bound is its smallest
        let mut bounds: Vec<f32> = Vec::new();
        let mut top = f32::INFINITY;
        for s in sizes {
            match bounds.last_mut() {
                Some(low) if s >= top * (1.0 - SAME_SIZE) => *low = s,
                _ => {
                    top = s;
                    bounds.push(s);
                }
            }
        }
        let title = bounds.len() > 1
            || bounds
                .first()
                .is_some_and(|top| body > 0.0 && *top >= title_ratio * body);
        Self {
            bounds,
            first: if title { 1 } else { 2 },
        }
    }

    fn level(&self, size: f32) -> u8 {
        let i = self
            .bounds
            .iter()
            .position(|b| size >= *b)
            .unwrap_or(self.bounds.len());
        (i.min(MAX_LEVEL as usize) as u8)
            .saturating_add(self.first)
            .min(MAX_LEVEL)
    }
}

/// Sets the level of each ranked heading (found by its box in the page's section).
pub(crate) fn rank_levels(
    sections: &mut [Section],
    heads: &[Vec<RankedHead>],
    body: f32,
    title_ratio: f32,
) {
    let ranks = Ranks::new(heads, body, title_ratio);
    for (section, page) in sections.iter_mut().zip(heads) {
        if page.is_empty() {
            continue;
        }
        for b in &mut section.blocks {
            if let Block::Heading {
                level,
                bbox: Some(bb),
                ..
            } = b
                && let Some(h) = page.iter().find(|h| h.bbox == *bb)
            {
                *level = ranks.level(h.size);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(size: f32) -> RankedHead {
        RankedHead {
            bbox: BBox::new(0.0, size, 10.0, size),
            size,
        }
    }

    #[test]
    fn near_equal_sizes_share_a_level_largest_first() {
        let heads = vec![
            vec![head(22.0), head(11.0), head(11.2)],
            vec![head(10.5), head(14.0)],
        ];
        let r = Ranks::new(&heads, 10.0, 1.45);
        assert_eq!(
            [22.0, 14.0, 11.2, 11.0, 10.5].map(|s| r.level(s)),
            [1, 2, 3, 3, 3]
        );
        // a chain of sizes 4 % apart is not one group
        let r = Ranks::new(
            &[vec![head(14.0), head(13.4), head(12.8), head(12.2)]],
            10.0,
            1.45,
        );
        assert_eq!([14.0, 13.4, 12.8, 12.2].map(|s| r.level(s)), [1, 1, 2, 2]);
        // one body-sized heading style: sections at level 2
        let r = Ranks::new(&[vec![head(11.0), head(11.1)]], 10.0, 1.45);
        assert_eq!(r.level(11.0), 2);
        let r = Ranks::new(&[vec![head(16.0)]], 10.0, 1.45);
        assert_eq!(r.level(16.0), 1, "a title");
    }

    #[test]
    fn levels_are_set_on_the_matching_headings() {
        let h = head(11.0);
        let mut s = Section::new(plaindoc_core::SectionKind::Page, 0);
        s.blocks = vec![
            Block::heading(3, "Name", Some(BBox::new(0.0, 1.0, 1.0, 1.0))),
            Block::heading(3, "EDUCATION", Some(h.bbox)),
        ];
        let mut secs = vec![s];
        rank_levels(&mut secs, &[vec![head(22.0)], vec![]], 10.0, 1.45);
        rank_levels(&mut secs, &[vec![h, head(22.0)]], 10.0, 1.45);
        let levels: Vec<u8> = secs[0]
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading { level, .. } => Some(*level),
                _ => None,
            })
            .collect();
        assert_eq!(levels, [3, 2]);
    }
}
