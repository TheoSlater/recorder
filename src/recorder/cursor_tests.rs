use super::{CursorEvaluator, CursorEvent, CursorSample, CursorSettings, CursorTrack};
use crate::recorder::input::{ButtonState, MouseEventKind};

fn track() -> CursorTrack {
    CursorTrack {
        samples: vec![sample(0, 0.0, 0.2), sample(1_000_000, 1.0, 0.8)],
        events: Vec::new(),
        warning: None,
    }
}

fn sample(timestamp_us: u64, x: f32, y: f32) -> CursorSample {
    CursorSample {
        timestamp_us,
        normalized_x: x,
        normalized_y: y,
        visible: true,
        buttons: ButtonState::default(),
    }
}

#[test]
fn interpolates_between_samples() {
    let position = track().position_at(0.5, 0.0);
    assert_eq!(position.x, 0.5);
    assert_eq!(position.y, 0.5);
}

#[test]
fn clamps_before_and_after_track() {
    let track = track();
    assert_eq!(track.position_at(-1.0, 0.0).x, 0.0);
    assert_eq!(track.position_at(2.0, 0.0).x, 1.0);
}

#[test]
fn smooths_position_toward_local_average() {
    let track = CursorTrack {
        samples: vec![
            sample(0, 0.0, 0.5),
            sample(1_000_000, 1.0, 0.5),
            sample(2_000_000, 1.0, 0.5),
        ],
        events: Vec::new(),
        warning: None,
    };

    let raw = track.position_at(1.0, 0.0);
    let gentle = track.position_at(1.0, 0.5).x;
    let full = track.position_at(1.0, 1.0).x;

    assert_eq!(raw.x, 1.0);
    assert!(full < gentle && gentle < raw.x && 0.0 < full);
    assert_eq!(track.position_at(1.0, 1.0).y, raw.y);
    assert!(track.position_at(1.0, 1.0).visible);
}

#[test]
fn smoothing_prioritizes_the_playhead_without_overshoot() {
    let track = CursorTrack {
        samples: vec![
            sample(0, 0.0, 0.5),
            sample(1_000_000, 1.0, 0.5),
            sample(2_000_000, 1.0, 0.5),
        ],
        events: Vec::new(),
        warning: None,
    };

    let position = track.position_at(1.0, 1.0);

    assert!(position.x > 0.965);
    assert!(position.x < 1.0);
    assert_eq!(position.y, 0.5);
}

#[test]
fn keeps_visible_cursor_outside_recording() {
    let evaluator = CursorEvaluator {
        track: Some(CursorTrack {
            samples: vec![sample(0, 1.1, 0.5)],
            events: Vec::new(),
            warning: None,
        }),
    };

    let frame = evaluator
        .frame_at(0.0, CursorSettings::default())
        .expect("cursor frame should exist");

    assert!(frame.visible);
    assert_eq!(frame.x, 1.1);
}

#[test]
fn accepts_samples_without_button_state() {
    let sample: CursorSample = serde_json::from_str(
        r#"{"timestamp_us":0,"normalized_x":0.5,"normalized_y":0.5,"visible":true}"#,
    )
    .expect("legacy cursor sample should deserialize");

    assert!(!sample.buttons.left);
    assert!(!sample.buttons.right);
    assert!(!sample.buttons.middle);
}

#[test]
fn click_bounce_uses_button_down_events() {
    let track = track_with_events(vec![event(1_000_000, MouseEventKind::LeftDown)]);

    assert_eq!(track.click_bounce_at(-1.0), 1.0);
    assert_eq!(track.click_bounce_at(f64::NAN), 1.0);
    assert_eq!(track.click_bounce_at(0.99), 1.0);
    assert!(track.click_bounce_at(1.0) < 1.0);
    assert!(track.click_bounce_at(1.096) > 1.0);
    assert_eq!(track.click_bounce_at(1.24), 1.0);
    assert_eq!(track.click_bounce_at(1.5), 1.0);
}

#[test]
fn click_bounce_ignores_button_up_events() {
    let track = track_with_events(vec![event(1_000_000, MouseEventKind::LeftUp)]);

    assert_eq!(track.click_bounce_at(1.0), 1.0);
}

#[test]
fn evaluator_applies_click_bounce_to_cursor_scale() {
    let evaluator = CursorEvaluator {
        track: Some(track_with_events(vec![event(
            1_000_000,
            MouseEventKind::RightDown,
        )])),
    };

    let frame = evaluator
        .frame_at(1.0, CursorSettings::default())
        .expect("cursor frame should exist");

    assert!(frame.scale < 1.0);
}

fn track_with_events(events: Vec<CursorEvent>) -> CursorTrack {
    CursorTrack {
        samples: vec![sample(0, 0.0, 0.2), sample(1_000_000, 1.0, 0.8)],
        events,
        warning: None,
    }
}

fn event(timestamp_us: u64, kind: MouseEventKind) -> CursorEvent {
    CursorEvent {
        timestamp_us,
        normalized_x: 0.5,
        normalized_y: 0.5,
        kind,
    }
}
