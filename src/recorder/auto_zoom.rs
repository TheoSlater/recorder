mod activities;
mod regions;

use super::{
    cursor::{CursorEvent, CursorSample},
    zoom::ZoomRegion,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct GenerationReport {
    pub(super) clicks: usize,
    pub(super) clusters: usize,
    pub(super) candidates: usize,
    pub(super) generated: usize,
}

/// Turns explicit click telemetry into ordinary editable zoom regions.
///
/// Only completed clicks are considered. The generator is deliberately a pure
/// function: it does not know about the decoder or editor state beyond the
/// existing regions it must protect, so every result remains a normal timeline
/// region that can be edited like a manually created one.
#[cfg(test)]
pub(super) fn generate(
    samples: &[CursorSample],
    events: &[CursorEvent],
    duration_us: u64,
    existing: &[ZoomRegion],
) -> Vec<ZoomRegion> {
    generate_with_report(samples, events, duration_us, existing).0
}

pub(super) fn generate_with_report(
    samples: &[CursorSample],
    events: &[CursorEvent],
    duration_us: u64,
    existing: &[ZoomRegion],
) -> (Vec<ZoomRegion>, GenerationReport) {
    if duration_us == 0 {
        return (Vec::new(), GenerationReport::default());
    }

    let clicks = activities::extract(samples, events, duration_us);
    let click_count = clicks.len();
    let clusters = activities::cluster(clicks);
    let cluster_count = clusters.len();
    let candidates: Vec<_> = clusters
        .into_iter()
        .filter_map(|activity| regions::candidate(activity, duration_us))
        .collect();
    let candidate_count = candidates.len();
    let generated = regions::filter(candidates, existing);
    let report = GenerationReport {
        clicks: click_count,
        clusters: cluster_count,
        candidates: candidate_count,
        generated: generated.len(),
    };
    (generated, report)
}

#[cfg(test)]
#[path = "auto_zoom_tests.rs"]
mod tests;
