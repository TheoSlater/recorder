use super::{CursorSizeEasing, CursorSizeRegion, MAX_CURSOR_SCALE, cursor_scale_at};

#[test]
fn cursor_size_region_normalizes_invalid_regions() {
    assert!(
        CursorSizeRegion {
            start_us: 2,
            end_us: 2,
            ..CursorSizeRegion::new_at(1_500_000, 2_000_000).unwrap()
        }
        .normalized()
        .is_none()
    );
    let region = CursorSizeRegion::new_at(1_000_000, 2_000_000).unwrap();
    assert_eq!(
        CursorSizeRegion {
            start_scale: 99.0,
            ..region
        }
        .normalized()
        .unwrap()
        .start_scale,
        MAX_CURSOR_SCALE
    );
}

#[test]
fn cursor_size_region_creates_at_end_without_zero_duration() {
    let region = CursorSizeRegion::new_at(7_899_983, 7_899_983).unwrap();

    assert_eq!(region.start_us, 6_899_983);
    assert_eq!(region.end_us, 7_899_983);
    assert!(region.duration_us() > 0);
}

#[test]
fn cursor_size_region_clamps_to_the_recording_duration() {
    let oversized = CursorSizeRegion {
        start_us: 4_000_000,
        end_us: 9_000_000,
        ..CursorSizeRegion::new_at(1_500_000, 5_000_000).unwrap()
    };

    assert_eq!(
        oversized.normalized_for_duration(5_000_000).unwrap(),
        CursorSizeRegion {
            start_us: 4_000_000,
            end_us: 5_000_000,
            ease_in_end_us: Some(4_200_000),
            ease_out_start_us: Some(4_800_000),
            ..CursorSizeRegion::new_at(1_500_000, 5_000_000).unwrap()
        }
    );
    assert!(oversized.normalized_for_duration(3_000_000).is_none());
}

#[test]
fn cursor_size_region_normalizes_unknown_easing_values() {
    let normalized = CursorSizeRegion {
        easing: CursorSizeEasing::Invalid,
        ..CursorSizeRegion::new_at(1_500_000, 2_000_000).unwrap()
    }
    .normalized()
    .unwrap();

    assert_eq!(normalized.easing, CursorSizeEasing::EaseInOut);
}

#[test]
fn cursor_size_region_fills_legacy_transition_defaults() {
    let normalized = CursorSizeRegion {
        ease_in_end_us: None,
        ease_out_start_us: None,
        ..CursorSizeRegion::new_at(1_000_000, 2_000_000).unwrap()
    }
    .normalized()
    .unwrap();

    assert_eq!(normalized.ease_in_end_us, Some(1_200_000));
    assert_eq!(normalized.ease_out_start_us, Some(1_800_000));
}

#[test]
fn cursor_size_region_eases_to_and_from_identity() {
    let region = CursorSizeRegion {
        start_us: 1_000_000,
        end_us: 2_000_000,
        start_scale: 1.0,
        end_scale: 2.0,
        easing: CursorSizeEasing::EaseInOut,
        ease_in_end_us: Some(1_200_000),
        ease_out_start_us: Some(1_800_000),
    };
    assert_eq!(region.scale_at(1_000_000).unwrap(), 1.0);
    assert_eq!(region.scale_at(1_500_000).unwrap(), 2.0);
    assert_eq!(region.scale_at(2_000_000).unwrap(), 1.0);
    assert!(region.scale_at(2_000_001).is_none());
}

#[test]
fn cursor_size_region_uses_dragged_transition_points() {
    let region = CursorSizeRegion {
        start_us: 1_000_000,
        end_us: 2_000_000,
        start_scale: 1.0,
        end_scale: 2.0,
        easing: CursorSizeEasing::EaseInOut,
        ease_in_end_us: Some(1_500_000),
        ease_out_start_us: Some(1_700_000),
    };

    assert_eq!(region.scale_at(1_500_000).unwrap(), 2.0);
    assert_eq!(region.scale_at(1_600_000).unwrap(), 2.0);
    assert!((region.scale_at(1_850_000).unwrap() - 1.5).abs() < 0.01);
}

#[test]
fn cursor_scale_at_newest_overlapping_region_wins() {
    let first = CursorSizeRegion {
        start_us: 1_000_000,
        end_us: 2_000_000,
        start_scale: 1.0,
        end_scale: 2.0,
        easing: CursorSizeEasing::EaseInOut,
        ease_in_end_us: Some(1_200_000),
        ease_out_start_us: Some(1_800_000),
    };
    let second = CursorSizeRegion {
        start_us: 1_000_000,
        end_us: 2_000_000,
        start_scale: 1.0,
        end_scale: 3.0,
        easing: CursorSizeEasing::EaseInOut,
        ease_in_end_us: Some(1_200_000),
        ease_out_start_us: Some(1_800_000),
    };
    assert_eq!(cursor_scale_at(&[first, second], 1_500_000, 1.0), 3.0);
}
