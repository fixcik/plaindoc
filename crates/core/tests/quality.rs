use plaindoc_core::*;

fn sec(levels: &[Level]) -> Section {
    let mut s = Section::new(SectionKind::Page, 0);
    s.warnings = levels
        .iter()
        .map(|l| Warning::new(WarningKind::PartialContent, *l, ""))
        .collect();
    s
}

#[test]
fn penalties_by_level() {
    let w = QualityWeights::default();
    assert_eq!(section_penalty(&sec(&[Level::Info]).warnings, &w), 0.0);
    assert_eq!(
        section_penalty(&sec(&[Level::Warn, Level::Loss]).warnings, &w),
        0.6
    );
}

#[test]
fn document_quality_is_minimum_not_average() {
    let mut d = Document::new(vec![sec(&[]), sec(&[Level::Loss]), sec(&[])]);
    apply_quality(&mut d, &QualityWeights::default());
    assert_eq!(d.sections[0].quality, 1.0);
    assert_eq!(d.sections[1].quality, 0.5);
    assert_eq!(d.quality, 0.5);
}

#[test]
fn parser_set_quality_is_kept_if_lower_and_result_is_clamped_and_idempotent() {
    let mut low = sec(&[]);
    low.quality = 0.3;
    let mut d = Document::new(vec![low, sec(&[Level::Loss, Level::Loss, Level::Loss])]);
    let w = QualityWeights::default();
    apply_quality(&mut d, &w);
    assert_eq!(d.sections[0].quality, 0.3);
    assert_eq!(d.sections[1].quality, 0.0);
    let once = d.clone();
    apply_quality(&mut d, &w);
    assert_eq!(d, once);
}

#[test]
fn document_level_warnings_and_empty_document() {
    let mut d = Document::new(vec![]);
    apply_quality(&mut d, &QualityWeights::default());
    assert_eq!(d.quality, 1.0);
    d.warnings
        .push(Warning::new(WarningKind::Truncated, Level::Warn, ""));
    apply_quality(&mut d, &QualityWeights::default());
    assert_eq!(d.quality, 0.9);
}
