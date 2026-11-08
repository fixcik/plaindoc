use super::*;
use crate::tables::TableConfig;
use crate::tables::testkit::{grid_page, hline};

#[test]
fn link_touching_matches_the_x_sweep() {
    let mut seed = 0xD1B5_4A32_D192_ED03_u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    for case in 0..1200 {
        let dec = case % 3 == 1;
        let n = 1 + next(if case % 20 == 0 { 300 } else { 90 }) as usize + if dec { 70 } else { 0 };
        let odd = case % 5 == 0; // inverted and NaN boxes: the plain x sweep must be used
        let boxes: Vec<[f64; 4]> = (0..n)
            .map(|_| {
                // Every third case: non-dyadic 2-decimal coordinates, so gaps of exactly 3.0 +- an ulp occur.
                let (x, y) = if dec {
                    (
                        next(150) as f64 * 0.01 + 0.01,
                        next(150) as f64 * 0.01 + 0.01,
                    )
                } else {
                    (next(40) as f64 * 2.5, next(40) as f64 * 2.5)
                };
                let (w, h) = if next(2) == 0 {
                    (
                        if dec {
                            next(300) as f64 * 0.01
                        } else {
                            next(60) as f64
                        },
                        0.5,
                    )
                } else {
                    (
                        0.5,
                        if dec {
                            next(300) as f64 * 0.01
                        } else {
                            next(60) as f64
                        },
                    )
                };
                let mut b = [x, y, x + w, y + h];
                if odd && next(8) == 0 {
                    b[2] = x - 3.0;
                }
                if odd && next(30) == 0 {
                    b[1] = f64::NAN;
                }
                b
            })
            .collect();
        let mut want = Dsu::new(n);
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&i, &j| boxes[i][0].total_cmp(&boxes[j][0]));
        for (ii, &i) in order.iter().enumerate() {
            for &j in &order[ii + 1..] {
                if boxes[j][0] > boxes[i][2] + REGION_TOL {
                    break;
                }
                if boxes[j][1] <= boxes[i][3] + REGION_TOL
                    && boxes[j][3] >= boxes[i][1] - REGION_TOL
                {
                    want.union(i, j);
                }
            }
        }
        let mut got = Dsu::new(n);
        link_touching(&boxes, &mut got);
        // Same partition: equal roots exactly where the reference has equal roots.
        for i in 0..n {
            for j in (0..n).step_by(7).chain([i]) {
                assert_eq!(
                    got.find(i) == got.find(j),
                    want.find(i) == want.find(j),
                    "case {case}"
                );
            }
        }
    }
}

/// Reviewer's counterexample: gaps of exactly 3.0 +- 1 ulp must give the same regions whichever sweep runs.
#[test]
#[allow(clippy::approx_constant)]
fn region_sweeps_agree_at_the_tolerance_edge() {
    let mut p = Prims {
        hs: vec![],
        vs: vec![],
        rects: vec![],
    };
    p.hs.push(HLine {
        a: 100.2,
        b: 200.0,
        y: 6.28,
        raster: false,
    });
    p.hs.push(HLine {
        a: 100.2,
        b: 200.0,
        y: 3.28,
        raster: false,
    });
    p.vs.push(VLine {
        a: 9.780000000000001,
        b: 50.0,
        x: 100.0,
        raster: false,
    });
    let direct = regions(&p).len();
    // 80 far stacked full-width rulings make the y sweep the cheaper one.
    for k in 0..80 {
        p.hs.push(HLine {
            a: 20.0,
            b: 520.0,
            y: 1000.0 + 5.0 * k as f64,
            raster: false,
        });
    }
    let stacked = regions(&p).into_iter().filter(|r| r.y0 < 500.0).count();
    assert_eq!(stacked, direct);
}

#[test]
fn thin_paths_become_rulings_and_stroked_boxes_add_edges() {
    let mut page = grid_page(&[50.0, 150.0], &[100.0, 120.0], &[]);
    page.paths.push(crate::tables::InputPath {
        rect: crate::tables::Rect::new(10.0, 10.0, 30.0, 20.0),
        stroke: true,
        fill: false,
    });
    let p = prims(&page);
    assert_eq!((p.hs.len(), p.vs.len(), p.rects.len()), (2 + 2, 2 + 2, 1));
}

#[test]
fn segments_of_composite_paths_are_rulings() {
    let mut page = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    page.segments.push(crate::tables::InputSegment {
        x1: 10.0,
        y1: 50.0,
        x2: 200.0,
        y2: 50.0,
        raster: false,
    });
    page.segments.push(crate::tables::InputSegment {
        x1: 10.0,
        y1: 50.0,
        x2: 10.0,
        y2: 90.0,
        raster: false,
    });
    page.segments.push(crate::tables::InputSegment {
        x1: 10.0,
        y1: 50.0,
        x2: 60.0,
        y2: 90.0,
        raster: false,
    });
    let p = prims(&page);
    assert_eq!((p.hs.len(), p.vs.len()), (1, 1));
    assert_eq!(p.vs[0].a, 50.0);
}

#[test]
fn skewed_raster_rule_is_a_ruling() {
    // 540 pt with a residual skew of 0.3° (Δy = 2.8 pt): within 1°, over 2.5 pt.
    let page = |raster: bool| {
        let mut p = PageInput {
            width: 600.0,
            height: 800.0,
            ..Default::default()
        };
        p.segments.push(crate::tables::InputSegment {
            x1: 30.0,
            y1: 100.0,
            x2: 570.0,
            y2: 102.8,
            raster,
        });
        p
    };
    let raster_on = prims_with(&page(true), true);
    assert_eq!((raster_on.hs.len(), raster_on.vs.len()), (1, 0));
    assert!(raster_on.hs[0].raster);
    assert!((raster_on.hs[0].y - 101.4).abs() < 1e-9);
    assert!(prims_with(&page(true), false).hs.is_empty(), "rule off");
    assert!(prims_with(&page(false), true).hs.is_empty(), "PDF segment");
}

fn cell_rules(gap: f64) -> PageInput {
    let mut page = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    let mut x = 51.0;
    for w in [23.0, 110.0, 110.0, 71.0] {
        page.paths.push(hline(x, x + w, 100.0));
        x += w + gap;
    }
    page
}

#[test]
fn collinear_cell_rules_merge_into_one() {
    let mut p = prims(&cell_rules(0.0));
    assert_eq!(p.hs.len(), 4);
    merge_collinear(&mut p);
    assert_eq!(p.hs.len(), 1);
    assert_eq!((p.hs[0].a, p.hs[0].b), (51.0, 365.0));
}

#[test]
fn gap_over_tolerance_keeps_rules_apart() {
    let mut p = prims(&cell_rules(2.0));
    merge_collinear(&mut p);
    assert_eq!(p.hs.len(), 4);
}

#[test]
fn merged_coord_is_running_mean() {
    let mut p = prims(&cell_rules(0.0));
    p.hs[3].y = 100.6;
    merge_collinear(&mut p);
    assert_eq!(p.hs.len(), 1);
    assert!((p.hs[0].y - 100.15).abs() < 1e-9, "{}", p.hs[0].y);
}

#[test]
fn merge_off_in_reference() {
    assert!(!TableConfig::reference().merge_lines);
}

#[test]
fn regions_need_three_primitives_and_size() {
    let page = grid_page(&[50.0, 150.0, 250.0], &[100.0, 120.0, 140.0], &[]);
    let r = regions(&prims(&page));
    assert_eq!(r.len(), 1);
    assert!((r[0].x0 - 50.0).abs() < 1e-9 && (r[0].x1 - 250.5).abs() < 1e-9);
    let mut lone = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    lone.paths.push(hline(50.0, 400.0, 100.0));
    lone.paths.push(hline(50.0, 400.0, 300.0));
    assert!(
        regions(&prims(&lone)).is_empty(),
        "two far rulings are not a region"
    );
}
