use super::{activities, generate};
use crate::recorder::cursor::{CursorEvent, CursorSample};
use crate::recorder::input::{ButtonState, MouseEventKind};
use crate::recorder::zoom::{DEFAULT_ZOOM_REGION_SCALE, ZoomEasing, ZoomRegion, ZoomTarget};

const DURATION_US: u64 = 8_000_000;

fn sample(timestamp_us: u64, x: f32, y: f32) -> CursorSample {
    CursorSample {
        timestamp_us,
        normalized_x: x,
        normalized_y: y,
        visible: true,
        buttons: ButtonState::default(),
    }
}

fn event(timestamp_us: u64, x: f32, y: f32, kind: MouseEventKind) -> CursorEvent {
    CursorEvent {
        timestamp_us,
        normalized_x: x,
        normalized_y: y,
        kind,
    }
}

fn click_events(points: &[(u64, f32, f32)]) -> Vec<CursorEvent> {
    points
        .iter()
        .flat_map(|&(timestamp_us, x, y)| {
            [
                event(timestamp_us, x, y, MouseEventKind::LeftDown),
                event(timestamp_us + 40_000, x, y, MouseEventKind::LeftUp),
            ]
        })
        .collect()
}

fn generate_regions(samples: &[CursorSample], events: &[CursorEvent]) -> Vec<ZoomRegion> {
    generate(samples, events, DURATION_US, &[])
}

#[test]
fn creates_a_depth_two_region_for_one_click() {
    let regions = generate_regions(&[], &click_events(&[(1_000_000, 0.4, 0.4)]));

    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].start_us, 0);
    assert_eq!(regions[0].end_us, 2_515_050);
    assert_eq!(regions[0].transition_points(), (500_000, 1_500_000));
    assert_eq!(regions[0].scale, DEFAULT_ZOOM_REGION_SCALE);
    assert_eq!(regions[0].target, ZoomTarget::Cursor);
    assert_eq!(regions[0].easing, ZoomEasing::EaseInOut);
}

#[test]
fn merges_clicks_within_two_and_a_half_seconds() {
    let regions = generate_regions(
        &[],
        &click_events(&[(1_000_000, 0.4, 0.4), (3_499_999, 0.42, 0.4)]),
    );

    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].start_us, 0);
    assert_eq!(regions[0].end_us, 5_015_049);
    assert_eq!(regions[0].transition_points(), (500_000, 3_999_999));
}

#[test]
fn separates_clicks_after_the_cluster_gap() {
    let events = click_events(&[(1_000_000, 0.2, 0.4), (3_500_001, 0.8, 0.4)]);
    let clicks = activities::extract(&[], &events, DURATION_US);
    let clusters = activities::cluster(clicks);
    let regions = generate_regions(&[], &events);

    assert_eq!(clusters.len(), 2);
    assert_eq!(regions.len(), 2);
    assert!(regions[0].end_us <= regions[1].start_us);
}

#[test]
fn keeps_connected_clusters_as_separate_regions() {
    let regions = generate_regions(
        &[],
        &click_events(&[(1_000_000, 0.2, 0.4), (5_000_000, 0.8, 0.4)]),
    );

    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].end_us, regions[1].start_us);
    assert_eq!(regions[0].transition_points(), (500_000, 1_500_000));
    assert_eq!(regions[1].transition_points(), (4_500_000, 5_500_000));
}

#[test]
fn reserves_slow_transition_windows() {
    let regions = generate_regions(&[], &click_events(&[(4_000_000, 0.4, 0.4)]));

    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].start_us, 1_977_425);
    assert_eq!(regions[0].end_us, 5_515_050);
    assert_eq!(regions[0].transition_points(), (3_500_000, 4_500_000));
}

#[test]
fn keeps_end_transition_slow_when_outro_is_clipped() {
    let regions = generate_regions(&[], &click_events(&[(7_800_000, 0.4, 0.4)]));

    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].start_us, 5_777_425);
    assert_eq!(regions[0].end_us, DURATION_US);
    assert_eq!(regions[0].transition_points(), (7_300_000, DURATION_US));
}

#[test]
fn chooses_the_strongest_click_as_cluster_focus() {
    let events = click_events(&[(1_000_000, 0.4, 0.4), (1_300_000, 0.03, 0.04)]);
    let clicks = activities::extract(&[], &events, DURATION_US);
    let clusters = activities::cluster(clicks);

    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].focus.strength, 1_500);
    assert_eq!(clusters[0].focus.timestamp_us, 1_300_000);
    assert_eq!(clusters[0].focus.x, 0.03);
    assert_eq!(clusters[0].focus.y, 0.04);

    let regions = generate_regions(&[], &events);
    assert_eq!(regions[0].target, ZoomTarget::CanvasCenter);
}

#[test]
fn recognizes_context_clicks() {
    let events = vec![
        event(1_000_000, 0.4, 0.4, MouseEventKind::RightDown),
        event(1_040_000, 0.4, 0.4, MouseEventKind::RightUp),
        event(1_500_000, 0.42, 0.4, MouseEventKind::MiddleDown),
        event(1_540_000, 0.42, 0.4, MouseEventKind::MiddleUp),
    ];
    let clicks = activities::extract(&[], &events, DURATION_US);

    assert_eq!(clicks.len(), 2);
    assert!(clicks.iter().all(|click| click.strength == 1_200));
    assert_eq!(generate_regions(&[], &events).len(), 1);
}

#[test]
fn ignores_cursor_motion_and_dwell_without_clicks() {
    let samples = vec![
        sample(1_000_000, 0.4, 0.4),
        sample(1_500_000, 0.4, 0.4),
        sample(2_000_000, 0.9, 0.4),
    ];

    assert!(generate_regions(&samples, &[]).is_empty());
}

#[test]
fn ignores_drag_motion_instead_of_treating_it_as_a_click() {
    let events = vec![
        event(1_000_000, 0.2, 0.4, MouseEventKind::LeftDown),
        event(2_000_000, 0.8, 0.4, MouseEventKind::LeftUp),
    ];
    let samples = vec![
        sample(1_000_000, 0.2, 0.4),
        sample(1_500_000, 0.5, 0.4),
        sample(2_000_000, 0.8, 0.4),
    ];

    assert!(generate_regions(&samples, &events).is_empty());
}

#[test]
fn normalizes_invalid_coordinates_and_clamps_time() {
    let events = vec![
        event(1_000_000, f32::NAN, 0.4, MouseEventKind::LeftDown),
        event(1_040_000, f32::NAN, 0.4, MouseEventKind::LeftUp),
        event(9_000_000, -0.2, 1.2, MouseEventKind::LeftDown),
        event(9_000_000, -0.2, 1.2, MouseEventKind::LeftUp),
    ];

    let regions = generate_regions(&[sample(1_000_000, f32::NAN, 0.5)], &events);

    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].start_us, 5_977_425);
    assert_eq!(regions[0].end_us, DURATION_US);
    assert_eq!(regions[0].target, ZoomTarget::CanvasCenter);
}

#[test]
fn keeps_distant_click_clusters_separate() {
    let regions = generate_regions(
        &[],
        &click_events(&[(1_000_000, 0.2, 0.4), (7_000_000, 0.8, 0.4)]),
    );

    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].start_us, 0);
    assert_eq!(regions[1].start_us, 4_977_425);
}

#[test]
fn protects_overlapping_manual_regions() {
    let manual = ZoomRegion {
        start_us: 900_000,
        end_us: 2_000_000,
        scale: 1.8,
        target: ZoomTarget::CanvasCenter,
        easing: ZoomEasing::EaseInOut,
        zoom_in_end_us: None,
        zoom_out_start_us: None,
    };
    let events = click_events(&[(1_200_000, 0.4, 0.4)]);

    assert!(generate(&[], &events, DURATION_US, &[manual]).is_empty());
}

#[test]
fn clamps_padding_at_recording_edges() {
    let regions = generate_regions(&[], &click_events(&[(100_000, 0.4, 0.4)]));

    assert_eq!(regions[0].start_us, 0);
    assert_eq!(regions[0].end_us, 1_615_050);
}
