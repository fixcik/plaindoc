//! Table/region dedup and table placement among page blocks (spec §4 "Дедупликация таблиц").

use pdf_oxide::StructuredPage;
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;
use plaindoc_core::{BBox, Block};

use super::layout::group_lines;
use super::tables::DetectedTable;

pub(crate) fn holds(b: &BBox, x: f32, y: f32) -> bool {
    b.x <= x && x <= b.x + b.w && b.y <= y && y <= b.y + b.h
}

fn in_tables(tables: &[DetectedTable], r: &Rect) -> bool {
    let (x, y) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
    tables.iter().any(|t| holds(&t.text_bbox, x, y))
}

/// The page without text whose centre lies inside a table's text box ([`DetectedTable::text_bbox`]: words of
/// the region the grid did not take stay); `None` when nothing is inside a table.
/// A cut region keeps its role and is split where table text was removed (so a heading above a table
/// does not merge with text below it); each piece's text is rebuilt from its spans, one line per row.
pub(crate) fn strip_table_text(
    page: &StructuredPage,
    tables: &[DetectedTable],
) -> Option<StructuredPage> {
    let hit = |r: &pdf_oxide::StructuredRegion| {
        if r.spans.is_empty() {
            in_tables(tables, &r.bbox)
        } else {
            r.spans.iter().any(|s| in_tables(tables, &s.bbox))
        }
    };
    if tables.is_empty() || !page.regions.iter().any(hit) {
        return None;
    }
    let mut out = StructuredPage {
        page_index: page.page_index,
        page_width: page.page_width,
        page_height: page.page_height,
        regions: Vec::with_capacity(page.regions.len()),
    };
    for r in &page.regions {
        if !hit(r) {
            out.regions.push(r.clone());
            continue;
        }
        let mut runs: Vec<Vec<TextSpan>> = vec![Vec::new()];
        for s in &r.spans {
            if in_tables(tables, &s.bbox) {
                runs.push(Vec::new());
            } else {
                runs.last_mut().expect("run").push(s.clone());
            }
        }
        for spans in runs.into_iter().filter(|v| !v.is_empty()) {
            let mut piece = r.clone();
            piece.bbox = spans
                .iter()
                .skip(1)
                .fold(spans[0].bbox, |b, s| b.union(&s.bbox));
            piece.text = group_lines(&spans)
                .into_iter()
                .map(|l| l.text)
                .collect::<Vec<_>>()
                .join("\n");
            piece.spans = spans;
            out.regions.push(piece);
        }
    }
    Some(out)
}

/// Inserts each table before the first block that starts at or below the table top and overlaps it
/// horizontally (PDF y grows upwards); otherwise appends it.
pub(crate) fn place_tables(blocks: &mut Vec<Block>, tables: &[DetectedTable]) {
    for t in tables {
        let top = t.bbox.y + t.bbox.h;
        let at = blocks.iter().position(|b| {
            block_bbox(b).is_some_and(|bb| {
                bb.y + bb.h <= top + 1.0 && bb.x < t.bbox.x + t.bbox.w && t.bbox.x < bb.x + bb.w
            })
        });
        blocks.insert(at.unwrap_or(blocks.len()), t.block.clone());
    }
}

pub(crate) fn block_bbox(b: &Block) -> Option<BBox> {
    match b {
        Block::Heading { bbox, .. }
        | Block::Paragraph { bbox, .. }
        | Block::List { bbox, .. }
        | Block::Table { bbox, .. }
        | Block::Image { bbox, .. }
        | Block::Code { bbox, .. }
        | Block::Note { bbox, .. } => *bbox,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_oxide::{RegionRole, StructuredRegion};

    fn span(text: &str, x: f32, y: f32) -> TextSpan {
        TextSpan {
            text: text.into(),
            bbox: Rect::new(x, y, 6.0 * text.len() as f32, 10.0),
            font_size: 10.0,
            ..Default::default()
        }
    }
    fn page(spans: Vec<TextSpan>) -> StructuredPage {
        let bbox = spans
            .iter()
            .skip(1)
            .fold(spans[0].bbox, |b, s| b.union(&s.bbox));
        let text = spans
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let region = StructuredRegion {
            kind: RegionRole::BodyBlock,
            text,
            bbox,
            spans,
            column_index: None,
            section_id: None,
        };
        StructuredPage {
            page_index: 0,
            page_width: 600.0,
            page_height: 800.0,
            regions: vec![region],
        }
    }
    fn table(x: f32, y: f32, w: f32, h: f32) -> DetectedTable {
        let bbox = BBox::new(x, y, w, h);
        DetectedTable::new(
            bbox,
            Block::Table {
                rows: vec![],
                header_rows: 0,
                bbox: Some(bbox),
            },
        )
    }

    #[test]
    fn table_text_is_removed_from_regions() {
        let p = page(vec![
            span("Intro text", 72.0, 700.0),
            span("Name", 72.0, 600.0),
            span("Qty", 200.0, 600.0),
            span("After", 72.0, 500.0),
        ]);
        let s = strip_table_text(&p, &[table(60.0, 580.0, 300.0, 40.0)]).unwrap();
        let texts: Vec<&str> = s.regions.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["Intro text", "After"], "split where the table was");
        assert!(
            strip_table_text(&p, &[table(300.0, 100.0, 50.0, 50.0)]).is_none(),
            "nothing inside"
        );
        let all = strip_table_text(&p, &[table(0.0, 0.0, 600.0, 800.0)]).unwrap();
        assert!(all.regions.is_empty());
    }

    #[test]
    fn tables_go_between_blocks_by_position() {
        let para = |t: &str, y: f32| Block::paragraph(t, Some(BBox::new(72.0, y, 300.0, 12.0)));
        let mut blocks = vec![
            para("above", 700.0),
            para("below", 500.0),
            Block::paragraph("nobox", None),
        ];
        place_tables(&mut blocks, &[table(60.0, 560.0, 300.0, 100.0)]);
        assert!(matches!(&blocks[1], Block::Table { .. }));
        let mut side = vec![Block::paragraph(
            "right column",
            Some(BBox::new(400.0, 500.0, 150.0, 12.0)),
        )];
        place_tables(&mut side, &[table(60.0, 560.0, 300.0, 100.0)]);
        assert!(
            matches!(side.last(), Some(Block::Table { .. })),
            "no horizontal overlap: appended"
        );
    }
}
