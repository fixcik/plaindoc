use super::*;
use pdf_oxide::layout::FontWeight;

fn span(text: &str, x: f32, y: f32, size: f32, bold: bool) -> TextSpan {
    TextSpan {
        text: text.into(),
        bbox: Rect::new(x, y, 6.0 * text.chars().count() as f32, size),
        font_size: size,
        font_weight: if bold {
            FontWeight::Bold
        } else {
            FontWeight::Normal
        },
        font_name: "Times".into(),
        ..Default::default()
    }
}

fn stats(mode: f32, above: &[f32]) -> FontStats {
    FontStats {
        mode,
        above: above.to_vec(),
    }
}

fn page_paras(spans: &[TextSpan]) -> Vec<Para> {
    paragraphs(&styled_lines(spans))
}

#[test]
fn bold_line_of_body_size_between_paragraphs_is_a_heading() {
    let spans = [
        span(
            "Body text of the previous paragraph goes here",
            72.0,
            700.0,
            11.0,
            false,
        ),
        span(
            "and continues on a second line of text",
            72.0,
            687.0,
            11.0,
            false,
        ),
        span("Conclusion", 72.0, 660.0, 11.0, true),
        span(
            "We have shown that the method works well",
            72.0,
            640.0,
            11.0,
            false,
        ),
        span(
            "on all of the corpora that we tried",
            72.0,
            627.0,
            11.0,
            false,
        ),
    ];
    let paras = page_paras(&spans);
    assert_eq!(paras.len(), 3);
    let refs: Vec<&Para> = paras.iter().collect();
    let s = scores(&refs, &vec![false; refs.len()], &stats(11.0, &[]));
    assert!(s[1] >= HEADING_THRESHOLD, "{s:?}");
    assert!(
        s[0] < HEADING_THRESHOLD && s[2] < HEADING_THRESHOLD,
        "{s:?}"
    );
}

#[test]
fn heading_above_a_table_keeps_its_contrast() {
    let spans = [
        span(
            "Body text of the previous paragraph goes here",
            72.0,
            700.0,
            11.0,
            false,
        ),
        span("Indicators", 72.0, 660.0, 13.0, true),
        span("Description", 72.0, 560.0, 13.0, true),
        span(
            "Body text of the following paragraph goes here",
            72.0,
            540.0,
            11.0,
            false,
        ),
    ];
    let paras = page_paras(&spans);
    let refs: Vec<&Para> = paras.iter().collect();
    let st = stats(11.0, &[]);
    // same style next to each other: no contrast
    let joined = scores(&refs, &[false; 4], &st);
    assert!(joined[1] < HEADING_THRESHOLD, "{joined:?}");
    // a table between them is a boundary like a page edge
    let s = scores(&refs, &[false, false, true, false], &st);
    assert!(
        s[1] >= HEADING_THRESHOLD && s[2] >= HEADING_THRESHOLD,
        "{s:?}"
    );
}

#[test]
fn line_between_two_tables_is_not_a_heading() {
    let spans = [
        span(
            "Body text of the first paragraph goes here",
            72.0,
            700.0,
            11.0,
            false,
        ),
        span("Total for the year", 72.0, 600.0, 11.0, false),
        span(
            "Body text of the last paragraph goes here",
            72.0,
            500.0,
            11.0,
            false,
        ),
    ];
    let paras = page_paras(&spans);
    let refs: Vec<&Para> = paras.iter().collect();
    let s = scores(&refs, &[false, true, true], &stats(11.0, &[]));
    assert!(s[1] < HEADING_THRESHOLD, "{s:?}");
    // a table on one side and the page edge on the other: same
    let s = scores(&refs[1..], &[false, true], &stats(11.0, &[]));
    assert!(s[0] < HEADING_THRESHOLD, "{s:?}");
}

#[test]
fn heading_directly_under_a_larger_heading_is_a_heading() {
    let body = |t: &str, y: f32| span(t, 72.0, y, 11.0, false);
    let spans = [
        body("Body text of the previous section goes here", 700.0),
        body("and continues on a second line of text", 687.0),
        span("4 Results", 72.0, 655.0, 14.0, true),
        span("4.1 Experimental Details", 72.0, 625.0, 12.0, true),
        body("We trained the model on all of the corpora", 600.0),
        body("that we could find for this experiment", 587.0),
    ];
    let paras = page_paras(&spans);
    assert_eq!(paras.len(), 4);
    let refs: Vec<&Para> = paras.iter().collect();
    let s = scores(&refs, &[false; 4], &stats(11.0, &[12.0, 14.0]));
    assert!(
        s[1] >= HEADING_THRESHOLD && s[2] >= HEADING_THRESHOLD,
        "{s:?}"
    );
    // a title right above, a table right below (ru.pdf «Показатели»)
    let spans = [
        span("Annual report", 72.0, 700.0, 21.0, true),
        span("Indicators", 72.0, 660.0, 15.75, true),
        span("Description", 72.0, 560.0, 15.75, true),
        span(
            "Body text of the description goes here",
            72.0,
            540.0,
            10.5,
            false,
        ),
    ];
    let paras = page_paras(&spans);
    let refs: Vec<&Para> = paras.iter().collect();
    let st = stats(10.5, &[15.75, 21.0]);
    let s = scores(&refs, &[false, false, true, false], &st);
    assert!(s.iter().take(3).all(|&x| x >= HEADING_THRESHOLD), "{s:?}");
}

#[test]
fn footnote_mark_after_a_sentence_end_is_trailing_punctuation() {
    assert!(ends_with_punctuation("as particular human rights focus.73"));
    assert!(ends_with_punctuation("Results:"));
    assert!(ends_with_punctuation("AGING, June 2012, Vol.4 No.6"));
    for t in [
        "Appendix A.1",
        "Part II.1",
        "Раздел IV.2",
        "Section 2.1",
        "Chapter 12",
        "Results",
    ] {
        assert!(!ends_with_punctuation(t), "{t}");
    }
}

#[test]
fn line_tight_under_a_larger_heading_keeps_it_as_context() {
    let body = |t: &str, y: f32| span(t, 72.0, y, 11.0, false);
    // gap 0 < 0.5 h: the larger line above is context, the smaller one gets no heading score
    let spans = [
        body("Body text of the previous section goes here", 700.0),
        body("and continues on a second line of text", 687.0),
        span("4 Results", 72.0, 655.0, 14.0, true),
        span("4.1 Experimental Details", 72.0, 643.0, 12.0, true),
        body("We trained the model on all of the corpora", 618.0),
        body("that we could find for this experiment", 605.0),
    ];
    let paras = page_paras(&spans);
    assert_eq!(paras.len(), 4);
    let refs: Vec<&Para> = paras.iter().collect();
    let s = scores(&refs, &[false; 4], &stats(11.0, &[12.0, 14.0]));
    assert!(s[2] < HEADING_THRESHOLD, "{s:?}");
}

#[test]
fn body_line_under_a_heading_above_a_table_is_not_a_heading() {
    let spans = [
        span("Results", 72.0, 700.0, 14.0, true),
        span("Total for the year", 72.0, 670.0, 11.0, false),
        span(
            "Body text of the last paragraph goes here",
            72.0,
            500.0,
            11.0,
            false,
        ),
    ];
    let paras = page_paras(&spans);
    let refs: Vec<&Para> = paras.iter().collect();
    let st = stats(11.0, &[14.0]);
    // the heading is not context and the table cuts the paragraph below: compared with the
    // body style, the line has no contrast
    let s = scores(&refs, &[false, false, true], &st);
    assert!(
        s[0] >= HEADING_THRESHOLD && s[1] < HEADING_THRESHOLD,
        "{s:?}"
    );
}

#[test]
fn larger_line_gets_the_rank_boost() {
    let spans = [
        span("Our Mission", 72.0, 700.0, 13.0, false),
        span(
            "We help people to read documents better",
            72.0,
            680.0,
            11.0,
            false,
        ),
    ];
    let paras = page_paras(&spans);
    let refs: Vec<&Para> = paras.iter().collect();
    let s = scores(&refs, &vec![false; refs.len()], &stats(11.0, &[13.0]));
    assert!(s[0] >= HEADING_THRESHOLD, "{s:?}");
}

#[test]
fn filters_reject_captions_punctuation_long_and_small_text() {
    let st = stats(11.0, &[]);
    let one = |t: &str, size: f32| page_paras(&[span(t, 72.0, 700.0, size, true)]).remove(0);
    assert!(passes_filters(&one("Введение", 11.0), &st));
    assert!(!passes_filters(&one("Таблица 3 Итоги", 11.0), &st));
    assert!(!passes_filters(&one("Definition 5.", 11.0), &st));
    assert!(!passes_filters(&one("ab", 11.0), &st));
    assert!(!passes_filters(&one(&"x".repeat(151), 11.0), &st));
    assert!(!passes_filters(&one("Footnote", 9.0), &st));
}

#[test]
fn same_baseline_neighbour_marks_cells() {
    let lines = styled_lines(&[
        span("Наименование", 72.0, 700.0, 11.0, true),
        span("Значение", 72.0 + 6.0 * 12.0 + 20.0, 700.0, 11.0, true),
        span("Отдельная строка", 72.0, 650.0, 11.0, true),
    ]);
    // group_lines joins the first two spans into one line: no neighbour; split them by region
    assert_eq!(lines.len(), 2);
    let a = styled_lines(&[span("Наименование", 72.0, 700.0, 11.0, true)]);
    let b = styled_lines(&[span("Значение", 160.0, 700.0, 11.0, true)]);
    let c = styled_lines(&[span("Отдельная строка", 72.0, 650.0, 11.0, true)]);
    let all = [&a[0], &b[0], &c[0]];
    assert_eq!(baseline_neighbours(&all), [true, true, false]);
}

#[test]
fn numbering_depth_for_levels() {
    assert_eq!(numbering_depth("3 Results"), Some(1));
    assert_eq!(numbering_depth("4.3 Ablation Studies"), Some(2));
    assert_eq!(numbering_depth("1.2.3. Детали"), Some(3));
    assert_eq!(numbering_depth("IV. Discussion"), Some(1));
    assert_eq!(numbering_depth("Глава 2 Методы"), Some(1));
    assert_eq!(numbering_depth("2024 was a year"), None);
    assert_eq!(numbering_depth("Conclusion"), None);
    assert_eq!(numbering_depth("6"), None);
}

#[test]
fn font_stats_mode_in_window_and_sizes_above() {
    let reg = |spans: Vec<TextSpan>| pdf_oxide::StructuredRegion {
        kind: RegionRole::BodyBlock,
        text: String::new(),
        bbox: spans[0].bbox,
        spans,
        column_index: None,
        section_id: None,
    };
    let p = StructuredPage {
        page_index: 0,
        page_width: 612.0,
        page_height: 792.0,
        regions: vec![reg(vec![
            span("a", 0.0, 0.0, 8.0, false),
            span("b", 0.0, 0.0, 8.0, false),
            span("c", 0.0, 0.0, 8.0, false),
            span("d", 0.0, 0.0, 11.0, false),
            span("e", 0.0, 0.0, 14.0, false),
            span("f", 0.0, 0.0, 40.0, false),
        ])],
    };
    assert_eq!(font_stats(&[&p]), stats(11.0, &[14.0]));
    assert_eq!(font_stats(&[]), stats(0.0, &[]));
}

/// One 10 pt line per `(text, x, y)`, 6 pt per char.
fn lines_at(rows: &[(&str, f32, f32)]) -> Vec<StyledLine> {
    let spans: Vec<TextSpan> = rows
        .iter()
        .map(|(t, x, y)| span(t, *x, *y, 10.0, false))
        .collect();
    let lines = styled_lines(&spans);
    assert_eq!(lines.len(), rows.len());
    lines
}

#[test]
fn gap_splits_paragraphs() {
    // pitch 1.35 inside paragraphs, 1.9 between them
    let rows = |gap: f32| {
        lines_at(&[
            ("first paragraph runs on here", 72.0, 700.0),
            ("and keeps going to the edge", 72.0, 686.5),
            ("of the column without a stop", 72.0, 673.0),
            ("second paragraph starts here", 72.0, 673.0 - gap),
            ("and goes on to its own end x", 72.0, 659.5 - gap),
        ])
    };
    assert_eq!(split_points(&rows(19.0), 1.35), [3]);
    assert!(split_points(&rows(13.5), 1.35).is_empty());
}

#[test]
fn hanging_indent_does_not_split() {
    let lines = lines_at(&[
        ("- first item of the list goes", 72.0, 700.0),
        ("on under a hanging indent", 84.0, 688.0),
        ("- second item of the list too", 72.0, 676.0),
    ]);
    assert_eq!(split_points(&lines, 1.2), [2]);
}

#[test]
fn first_line_indent_splits() {
    let lines = lines_at(&[
        ("the paragraph runs to the edge", 72.0, 700.0),
        ("and ends here without a stop", 72.0, 688.0),
        ("A new one is indented", 102.0, 676.0),
        ("and continues at the margin x", 72.0, 664.0),
    ]);
    assert_eq!(split_points(&lines, 1.2), [2]);
}

#[test]
fn ragged_edge_with_long_word_does_not_split() {
    let rows = |second: &str, third: &str| {
        lines_at(&[
            ("a line that reaches the right edge", 72.0, 700.0),
            (second, 72.0, 688.0),
            (third, 72.0, 676.0),
        ])
    };
    // short line without a stop: the ragged right edge, not a paragraph end
    let ragged = rows("a shorter line of text", "Extraordinarily long words here");
    assert!(split_points(&ragged, 1.2).is_empty());
    // ends a sentence, but the next first word would not have fitted
    let full = rows("a shorter line of text.", "Extraordinarily long words here");
    assert!(split_points(&full, 1.2).is_empty());
    // ends a sentence and the next word would have fitted: a paragraph end
    let short = rows("a short line.", "A new paragraph begins here and on");
    assert_eq!(split_points(&short, 1.2), [2]);
}

#[test]
fn phrase_continuation_across_columns_does_not_split() {
    let rows = |left_end: &str| {
        lines_at(&[
            ("text of the left column runs", 72.0, 112.0),
            (left_end, 72.0, 100.0),
            ("continues at the right column", 320.0, 700.0),
        ])
    };
    assert!(split_points(&rows("down and stops in the"), 1.2).is_empty());
    assert_eq!(split_points(&rows("down and ends a phrase."), 1.2), [2]);
}

#[test]
fn loose_leading_does_not_split_every_line() {
    let rows: Vec<(String, f32, f32)> = (0..8)
        .map(|i| {
            (
                "loose leading body text line".to_string(),
                72.0,
                700.0 - 15.5 * i as f32,
            )
        })
        .collect();
    let spans: Vec<TextSpan> = rows
        .iter()
        .map(|(t, x, y)| span(t, *x, *y, 10.0, false))
        .collect();
    let page = StructuredPage {
        page_index: 0,
        page_width: 612.0,
        page_height: 792.0,
        regions: vec![pdf_oxide::StructuredRegion {
            kind: RegionRole::BodyBlock,
            text: String::new(),
            bbox: spans[0].bbox,
            spans: spans.clone(),
            column_index: None,
            section_id: None,
        }],
    };
    let ratio = line_pitch_ratio(&[&page]);
    assert!((ratio - 1.55).abs() < 1e-4, "{ratio}");
    assert_eq!(line_pitch_ratio(&[]), DEFAULT_PITCH);
    let lines = styled_lines(&spans);
    assert!(split_points(&lines, ratio).is_empty());
    // an absolute threshold of the default pitch would split every line
    assert_eq!(split_points(&lines, DEFAULT_PITCH).len(), 7);
}

#[test]
fn non_finite_geometry_does_not_panic() {
    let mut s = span("Head", 72.0, 700.0, 11.0, true);
    s.bbox = Rect::new(f32::NAN, f32::INFINITY, f32::NAN, f32::NAN);
    s.font_size = f32::NAN;
    let paras = page_paras(&[s.clone(), s]);
    let refs: Vec<&Para> = paras.iter().collect();
    let _ = scores(&refs, &vec![false; refs.len()], &stats(11.0, &[12.0]));
    let lines = styled_lines(&[span("x y z", f32::NAN, 1.0, 11.0, false)]);
    let _ = baseline_neighbours(&lines.iter().collect::<Vec<_>>());
}
