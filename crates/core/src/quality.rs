use crate::{Document, Level, QualityWeights, Warning};

/// Info 0, Warn `w.warn`, Loss `w.loss`; summed over the warnings.
pub fn section_penalty(warnings: &[Warning], w: &QualityWeights) -> f32 {
    warnings
        .iter()
        .map(|x| match x.level {
            Level::Info => 0.0,
            Level::Warn => w.warn,
            Level::Loss => w.loss,
        })
        .sum()
}

/// Idempotent. Section quality is the minimum of the parser's value and
/// `1 - penalty`; document quality is the minimum of `1 - penalty` of its own
/// warnings and all section qualities. Results are clamped to `[0, 1]`.
pub fn apply_quality(doc: &mut Document, w: &QualityWeights) {
    let mut q = (1.0 - section_penalty(&doc.warnings, w)).clamp(0.0, 1.0);
    for s in &mut doc.sections {
        s.quality = s
            .quality
            .min(1.0 - section_penalty(&s.warnings, w))
            .clamp(0.0, 1.0);
        q = q.min(s.quality);
    }
    doc.quality = q;
}
