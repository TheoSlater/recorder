use super::{MAX_ZOOM_REGION_SCALE, ZoomEasing, ZoomRegion, ZoomTarget, effect_at};

fn region() -> ZoomRegion {
    ZoomRegion {
        start_us: 1_000_000,
        end_us: 2_000_000,
        scale: 2.0,
        target: ZoomTarget::Cursor,
        easing: ZoomEasing::EaseInOut,
        zoom_in_end_us: Some(1_200_000),
        zoom_out_start_us: Some(1_800_000),
    }
}

#[test]
fn normalizes_invalid_regions() {
    assert!(
        ZoomRegion {
            start_us: 2,
            end_us: 2,
            ..region()
        }
        .normalized()
        .is_none()
    );
    assert_eq!(
        ZoomRegion {
            scale: 99.0,
            ..region()
        }
        .normalized()
        .unwrap()
        .scale,
        MAX_ZOOM_REGION_SCALE
    );
}

#[test]
fn creates_a_region_at_the_end_without_zero_duration() {
    let region = ZoomRegion::new_at(7_899_983, 7_899_983).unwrap();

    assert_eq!(region.start_us, 6_899_983);
    assert_eq!(region.end_us, 7_899_983);
    assert!(region.duration_us() > 0);
}

#[test]
fn clamps_regions_to_the_recording_duration() {
    let oversized = ZoomRegion {
        start_us: 4_000_000,
        end_us: 9_000_000,
        ..region()
    };

    assert_eq!(
        oversized.normalized_for_duration(5_000_000).unwrap(),
        ZoomRegion {
            start_us: 4_000_000,
            end_us: 5_000_000,
            zoom_in_end_us: Some(4_200_000),
            zoom_out_start_us: Some(4_800_000),
            ..region()
        }
    );
    assert!(oversized.normalized_for_duration(3_000_000).is_none());
}

#[test]
fn normalizes_unknown_target_and_easing_values() {
    let normalized = ZoomRegion {
        target: ZoomTarget::Invalid,
        easing: ZoomEasing::Invalid,
        ..region()
    }
    .normalized()
    .unwrap();

    assert_eq!(normalized.target, ZoomTarget::CanvasCenter);
    assert_eq!(normalized.easing, ZoomEasing::EaseInOut);
}

#[test]
fn fills_legacy_transition_defaults() {
    let normalized = ZoomRegion {
        zoom_in_end_us: None,
        zoom_out_start_us: None,
        ..region()
    }
    .normalized()
    .unwrap();

    assert_eq!(normalized.zoom_in_end_us, Some(1_200_000));
    assert_eq!(normalized.zoom_out_start_us, Some(1_800_000));
}

#[test]
fn eases_to_and_from_identity() {
    let region = region();
    assert_eq!(region.effect_at(1_000_000).unwrap().scale, 1.0);
    assert_eq!(region.effect_at(1_500_000).unwrap().scale, 2.0);
    assert_eq!(region.effect_at(2_000_000).unwrap().scale, 1.0);
    assert!(region.effect_at(2_000_001).is_none());
}

#[test]
fn easing_is_stronger_than_a_linear_ramp() {
    let quarter = super::transition_progress(250_000, 1_000_000);
    let halfway = super::transition_progress(500_000, 1_000_000);
    let three_quarters = super::transition_progress(750_000, 1_000_000);

    assert_eq!(super::transition_progress(0, 1_000_000), 0.0);
    assert_eq!(super::transition_progress(1_000_000, 1_000_000), 1.0);
    assert!(quarter < 0.125);
    assert!((halfway - 0.5).abs() < f32::EPSILON);
    assert!(three_quarters > 0.875);
}

#[test]
fn easing_progresses_monotonically_without_overshoot() {
    let mut previous = 0.0;
    for step in 0..=20 {
        let progress = super::transition_progress(step * 50_000, 1_000_000);
        assert!((0.0..=1.0).contains(&progress));
        assert!(progress >= previous);
        previous = progress;
    }
}

#[test]
fn uses_dragged_transition_points() {
    let region = ZoomRegion {
        zoom_in_end_us: Some(1_500_000),
        zoom_out_start_us: Some(1_700_000),
        ..region()
    };

    assert_eq!(region.effect_at(1_500_000).unwrap().scale, 2.0);
    assert_eq!(region.effect_at(1_600_000).unwrap().scale, 2.0);
    assert_eq!(region.effect_at(1_850_000).unwrap().scale, 1.5);
}

#[test]
fn holds_the_final_zoom_when_exit_is_clipped() {
    let region = ZoomRegion {
        zoom_out_start_us: Some(2_000_000),
        ..region()
    };

    assert_eq!(region.effect_at(2_000_000).unwrap().scale, 2.0);
}

#[test]
fn newest_overlapping_region_wins() {
    let first = region();
    let second = ZoomRegion {
        scale: 3.0,
        ..region()
    };
    assert_eq!(effect_at(&[first, second], 1_500_000).unwrap().scale, 3.0);
}
