use super::super::zoom::{DEFAULT_ZOOM_REGION_SCALE, ZoomEasing, ZoomRegion, ZoomTarget};
use super::activities::ClickCluster;

const REGION_PADDING_US: u64 = 500_000;
// Keep the reference playback timing in microseconds so generated regions do not
// inherit the short duration-ratio ramps used by compact manual regions.
const ZOOM_IN_US: u64 = 1_522_575;
const ZOOM_OUT_US: u64 = 1_015_050;
const CONNECTED_GAP_US: u64 = 1_350_000;
const SAFE_TARGET_MIN: f32 = 0.15;
const SAFE_TARGET_MAX: f32 = 0.85;

pub(super) fn candidate(cluster: ClickCluster, duration_us: u64) -> Option<ZoomRegion> {
    let active_start_us = cluster.start_us.saturating_sub(REGION_PADDING_US);
    let active_end_us = cluster
        .end_us
        .saturating_add(REGION_PADDING_US)
        .min(duration_us);
    let start_us = active_start_us.saturating_sub(ZOOM_IN_US);
    let end_us = active_end_us.saturating_add(ZOOM_OUT_US).min(duration_us);
    if end_us <= start_us {
        return None;
    }

    ZoomRegion {
        start_us,
        end_us,
        scale: DEFAULT_ZOOM_REGION_SCALE,
        target: target_for(cluster.focus),
        easing: ZoomEasing::EaseInOut,
        zoom_in_end_us: Some(active_start_us.clamp(start_us, end_us)),
        zoom_out_start_us: Some(active_end_us.clamp(start_us, end_us)),
    }
    .normalized_for_duration(duration_us)
}

pub(super) fn filter(mut candidates: Vec<ZoomRegion>, existing: &[ZoomRegion]) -> Vec<ZoomRegion> {
    candidates.sort_by_key(|candidate| candidate.start_us);
    let mut accepted = Vec::new();

    for mut candidate in candidates {
        if existing.iter().any(|region| overlaps(candidate, *region)) {
            continue;
        }

        if let Some(previous) = accepted.last_mut() {
            connect_regions(previous, &mut candidate, existing);
        }
        accepted.push(candidate);
    }

    accepted
}

fn connect_regions(previous: &mut ZoomRegion, next: &mut ZoomRegion, existing: &[ZoomRegion]) {
    if next.start_us < previous.end_us {
        trim_overlap(previous, next);
        return;
    }

    let gap_us = next.start_us - previous.end_us;
    if gap_us <= CONNECTED_GAP_US
        && !existing
            .iter()
            .any(|region| overlaps_range(previous.end_us, next.start_us, *region))
    {
        // Keep connected regions separate, but remove a short identity gap so
        // playback does not flash out between two nearby attention areas.
        previous.end_us = next.start_us;
    }
}

fn trim_overlap(previous: &mut ZoomRegion, next: &mut ZoomRegion) {
    let (_, previous_out_us) = previous.transition_points();
    let (next_in_us, _) = next.transition_points();
    let midpoint = previous_out_us.saturating_add(next_in_us.saturating_sub(previous_out_us) / 2);
    let boundary = midpoint.clamp(next.start_us, previous.end_us);

    previous.end_us = boundary;
    previous.zoom_out_start_us = Some(previous_out_us.min(boundary));
    next.start_us = boundary;
    next.zoom_in_end_us = Some(next_in_us.max(boundary).min(next.end_us));
}

fn target_for(focus: super::activities::Click) -> ZoomTarget {
    if (SAFE_TARGET_MIN..=SAFE_TARGET_MAX).contains(&focus.x)
        && (SAFE_TARGET_MIN..=SAFE_TARGET_MAX).contains(&focus.y)
    {
        ZoomTarget::Cursor
    } else {
        ZoomTarget::CanvasCenter
    }
}

fn overlaps(left: ZoomRegion, right: ZoomRegion) -> bool {
    left.start_us < right.end_us && right.start_us < left.end_us
}

fn overlaps_range(start_us: u64, end_us: u64, region: ZoomRegion) -> bool {
    start_us < region.end_us && region.start_us < end_us
}
