use super::{
    CursorPoint, FrameSample, MotionBlurDescriptor, MotionBlurHistory, MotionBlurMode,
    MotionBlurSettings, RecordingTransform, Vec2, compute_display_motion_blur, fps_scale,
};

const FRAME_60: f64 = 1.0 / 60.0;
const FRAME_30: f64 = 1.0 / 30.0;
const CENTER: Vec2 = Vec2 { x: 0.5, y: 0.5 };

fn transform(center_x: f32, center_y: f32, size: f32) -> RecordingTransform {
    RecordingTransform::new(Vec2::new(center_x, center_y), Vec2::new(size, size))
        .expect("test transform should be usable")
}

const FULL: MotionBlurSettings = MotionBlurSettings { amount: 1.0 };

/// Classifies one 60 FPS step at full authored strength.
fn classify(
    previous: RecordingTransform,
    current: RecordingTransform,
    previous_mode: MotionBlurMode,
) -> MotionBlurDescriptor {
    compute_display_motion_blur(
        previous,
        current,
        0.0,
        FRAME_60,
        previous_mode,
        CENTER,
        FULL,
    )
}

/// Gains applied to a 60 FPS step, so magnitude assertions stay correct when
/// the multipliers are retuned.
fn movement_gain() -> f32 {
    FULL.movement_strength(1.0)
}

fn zoom_gain() -> f32 {
    FULL.zoom_strength(1.0)
}

#[test]
fn ignores_still_transform() {
    let still = transform(0.5, 0.5, 0.5);
    let blur = classify(still, still, MotionBlurMode::None);

    assert_eq!(blur.mode, MotionBlurMode::None);
    assert_eq!(blur.movement_uv, Vec2::ZERO);
    assert_eq!(blur.zoom_amount, 0.0);
}

#[test]
fn ignores_drift_below_dead_zone() {
    let blur = classify(
        transform(0.5, 0.5, 0.5),
        transform(0.5001, 0.5, 0.500_02),
        MotionBlurMode::None,
    );

    assert_eq!(blur.mode, MotionBlurMode::None);
}

#[test]
fn smears_along_horizontal_movement() {
    let blur = classify(
        transform(0.5, 0.5, 0.5),
        transform(0.55, 0.5, 0.5),
        MotionBlurMode::None,
    );

    assert_eq!(blur.mode, MotionBlurMode::Movement);
    // A tenth of the layer's own width: 0.05 of the canvas over a half-canvas layer.
    assert!(
        (blur.movement_uv.x - 0.1 * movement_gain()).abs() < 1e-5,
        "{blur:?}"
    );
    assert!(blur.movement_uv.y.abs() < 1e-6, "{blur:?}");
}

#[test]
fn smears_along_vertical_movement() {
    let blur = classify(
        transform(0.5, 0.5, 0.5),
        transform(0.5, 0.44, 0.5),
        MotionBlurMode::None,
    );

    assert_eq!(blur.mode, MotionBlurMode::Movement);
    assert!(blur.movement_uv.x.abs() < 1e-6, "{blur:?}");
    // Upwards travel keeps a negative vector: direction survives the gain.
    assert!(blur.movement_uv.y < 0.0, "{blur:?}");
    assert!(
        (blur.movement_uv.y + 0.12 * movement_gain()).abs() < 1e-5,
        "{blur:?}"
    );
}

#[test]
fn clamps_large_movement() {
    let blur = classify(
        transform(0.5, 0.5, 0.5),
        transform(2.5, 0.5, 0.5),
        MotionBlurMode::None,
    );

    assert_eq!(blur.mode, MotionBlurMode::Movement);
    assert!((blur.movement_uv.length() - 0.15).abs() < 1e-5, "{blur:?}");
}

#[test]
fn classifies_scale_change_as_zoom() {
    let blur = classify(
        transform(0.5, 0.5, 0.5),
        transform(0.5, 0.5, 0.52),
        MotionBlurMode::None,
    );

    assert_eq!(blur.mode, MotionBlurMode::Zoom);
    assert!(
        blur.zoom_amount > 0.0,
        "zooming in reads positive: {blur:?}"
    );
    assert!(
        (blur.zoom_amount - 0.04 * zoom_gain()).abs() < 1e-5,
        "{blur:?}"
    );
    assert_eq!(blur.movement_uv, Vec2::ZERO);
}

#[test]
fn clamps_large_zoom() {
    let blur = classify(
        transform(0.5, 0.5, 0.5),
        transform(0.5, 0.5, 1.5),
        MotionBlurMode::None,
    );

    assert_eq!(blur.mode, MotionBlurMode::Zoom);
    assert!((blur.zoom_amount - 0.10).abs() < 1e-5, "{blur:?}");
}

#[test]
fn smears_zoom_out_backwards() {
    let blur = classify(
        transform(0.5, 0.5, 0.5),
        transform(0.5, 0.5, 0.48),
        MotionBlurMode::None,
    );

    assert_eq!(blur.mode, MotionBlurMode::Zoom);
    assert!(blur.zoom_amount < 0.0, "{blur:?}");
}

#[test]
fn classifies_translation_dominant_transform() {
    let blur = classify(
        transform(0.5, 0.5, 0.5),
        transform(0.58, 0.5, 0.502),
        MotionBlurMode::None,
    );

    assert_eq!(blur.mode, MotionBlurMode::Movement);
}

#[test]
fn holds_mode_through_mixed_transform() {
    // Translation and scale contribute comparable motion, so neither dominates
    // and the previous mode is kept rather than flipping every frame.
    // 0.02 of canvas travel against a 4% scale change: both contribute the
    // same equivalent motion, so neither passes the dominance threshold.
    let previous = transform(0.5, 0.5, 0.5);
    let current = transform(0.52, 0.5, 0.52);

    assert_eq!(
        classify(previous, current, MotionBlurMode::Zoom).mode,
        MotionBlurMode::Zoom
    );
    assert_eq!(
        classify(previous, current, MotionBlurMode::Movement).mode,
        MotionBlurMode::Movement
    );
}

#[test]
fn carries_zoom_center() {
    let focus = Vec2::new(0.2, 0.8);
    let blur = compute_display_motion_blur(
        transform(0.5, 0.5, 0.5),
        transform(0.5, 0.5, 0.55),
        0.0,
        FRAME_60,
        MotionBlurMode::None,
        focus,
        FULL,
    );

    assert_eq!(blur.mode, MotionBlurMode::Zoom);
    assert_eq!(blur.zoom_center_uv, focus);
}

#[test]
fn classifies_scale_dominant_transform() {
    // A small nudge against a large scale change: zoom wins outright, so no
    // previous mode is needed to break the tie.
    let blur = classify(
        transform(0.5, 0.5, 0.5),
        transform(0.503, 0.5, 0.56),
        MotionBlurMode::Movement,
    );

    assert_eq!(blur.mode, MotionBlurMode::Zoom);
}

#[test]
fn classification_bypasses_zero_amount() {
    let blur = compute_display_motion_blur(
        transform(0.5, 0.5, 0.5),
        transform(0.9, 0.5, 0.9),
        0.0,
        FRAME_60,
        MotionBlurMode::None,
        CENTER,
        MotionBlurSettings { amount: 0.0 },
    );

    assert_eq!(blur.mode, MotionBlurMode::None);
    assert_eq!(blur.movement_uv, Vec2::ZERO);
    assert_eq!(blur.zoom_amount, 0.0);
    assert_eq!(blur.strength, 0.0);
}

#[test]
fn scales_display_strength_with_frame_interval() {
    let step = |elapsed: f64| {
        compute_display_motion_blur(
            transform(0.5, 0.5, 0.5),
            transform(0.55, 0.5, 0.5),
            0.0,
            elapsed,
            MotionBlurMode::None,
            CENTER,
            FULL,
        )
    };
    // The same displacement over twice the media time is half the velocity, so
    // a 30 FPS preview must not smear twice as far as a 60 FPS one.
    let fast = step(FRAME_60);
    let slow = step(FRAME_30);

    assert!((fast.movement_uv.x - slow.movement_uv.x * 2.0).abs() < 1e-5);
}

#[test]
fn keeps_discontinuous_steps_sharp() {
    let step = |previous_seconds: f64, current_seconds: f64| {
        compute_display_motion_blur(
            transform(0.5, 0.5, 0.5),
            transform(0.7, 0.5, 0.5),
            previous_seconds,
            current_seconds,
            MotionBlurMode::None,
            CENTER,
            FULL,
        )
    };

    // A repeat, a rewind, and a jump are all discontinuities, not velocity.
    assert_eq!(step(1.0, 1.0).mode, MotionBlurMode::None);
    assert_eq!(step(1.0, 0.9).mode, MotionBlurMode::None);
    assert_eq!(step(1.0, 6.0).mode, MotionBlurMode::None);
}

#[test]
fn rejects_out_of_bounds_cursor() {
    assert!(CursorPoint::new(0.5, 0.5).is_some());
    assert!(CursorPoint::new(0.0, 1.0).is_some());
    assert!(CursorPoint::new(-0.2, 0.5).is_none());
    assert!(CursorPoint::new(0.5, f32::NAN).is_none());
}

#[test]
fn converts_cursor_delta_to_rendered_pixels() {
    let motion = super::CursorMotion {
        delta: Vec2::new(0.1, 0.0),
        strength: 0.5,
    };
    let sprite = motion
        .to_sprite(1000.0, 500.0)
        .expect("motion should exceed the dead zone");

    assert!((sprite.motion().x - 50.0).abs() < 1e-4, "{sprite:?}");
    assert_eq!(sprite.motion().y, 0.0);
}

#[test]
fn clamps_cursor_teleport() {
    let motion = super::CursorMotion {
        delta: Vec2::new(1.0, 0.0),
        strength: 1.0,
    };
    let sprite = motion.to_sprite(4000.0, 2000.0).expect("motion is large");

    assert!(
        (sprite.motion().length() - 480.0).abs() < 1e-3,
        "{sprite:?}"
    );
}

#[test]
fn keeps_sub_pixel_cursor_motion_sharp() {
    let motion = super::CursorMotion {
        delta: Vec2::new(0.0001, 0.0),
        strength: 1.0,
    };

    assert!(motion.to_sprite(1000.0, 500.0).is_none());
}

#[test]
fn scales_strength_with_preview_rate() {
    assert!((fps_scale(FRAME_60) - 1.0).abs() < 1e-5);
    assert!((fps_scale(FRAME_30) - 0.5).abs() < 1e-5);
    assert!((fps_scale(1.0 / 24.0) - 0.4).abs() < 1e-5);
    // A missing or reversed timestamp cannot amplify the smear.
    assert_eq!(fps_scale(0.0), 1.0);
    assert_eq!(fps_scale(f64::NAN), 1.0);
}

fn sample(seconds: f64, generation: u64, cursor: Option<(f32, f32)>, size: f32) -> FrameSample {
    FrameSample {
        seconds,
        seek_generation: generation,
        cursor: cursor.and_then(|(x, y)| CursorPoint::new(x, y)),
        transform: Some(transform(0.5, 0.5, size)),
        zoom_center_uv: CENTER,
        settings: MotionBlurSettings { amount: 1.0 },
    }
}

#[test]
fn keeps_first_frame_sharp() {
    let mut history = MotionBlurHistory::default();
    let frame = history.presented(sample(0.0, 0, Some((0.1, 0.1)), 0.5));

    assert!(frame.cursor.is_none());
    assert_eq!(frame.display.mode, MotionBlurMode::None);
}

#[test]
fn measures_between_presented_frames() {
    let mut history = MotionBlurHistory::default();
    history.presented(sample(0.0, 0, Some((0.1, 0.1)), 0.5));
    let frame = history.presented(sample(FRAME_60, 0, Some((0.2, 0.1)), 0.55));

    let cursor = frame.cursor.expect("cursor moved");
    assert!((cursor.delta.x - 0.1).abs() < 1e-5);
    assert!((cursor.strength - 1.0).abs() < 1e-5, "{cursor:?}");
    assert_eq!(frame.display.mode, MotionBlurMode::Zoom);
}

#[test]
fn halves_strength_at_half_rate() {
    let mut history = MotionBlurHistory::default();
    history.presented(sample(0.0, 0, Some((0.1, 0.1)), 0.5));
    let frame = history.presented(sample(FRAME_30, 0, Some((0.2, 0.1)), 0.5));

    let cursor = frame.cursor.expect("cursor moved");
    assert!((cursor.strength - 0.5).abs() < 1e-5, "{cursor:?}");
}

#[test]
fn resets_history_on_seek() {
    let mut history = MotionBlurHistory::default();
    history.presented(sample(0.0, 0, Some((0.1, 0.1)), 0.5));
    let frame = history.presented(sample(FRAME_60, 1, Some((0.9, 0.9)), 0.9));

    assert!(frame.cursor.is_none());
    assert_eq!(frame.display.mode, MotionBlurMode::None);
}

#[test]
fn resets_history_on_request() {
    let mut history = MotionBlurHistory::default();
    history.presented(sample(0.0, 0, Some((0.1, 0.1)), 0.5));
    history.reset();
    let frame = history.presented(sample(FRAME_60, 0, Some((0.4, 0.1)), 0.5));

    assert!(frame.cursor.is_none());
}

#[test]
fn keeps_frame_after_time_jump_sharp() {
    let mut history = MotionBlurHistory::default();
    history.presented(sample(0.0, 0, Some((0.1, 0.1)), 0.5));
    let frame = history.presented(sample(5.0, 0, Some((0.9, 0.9)), 0.5));

    assert!(frame.cursor.is_none());
    assert_eq!(frame.display.mode, MotionBlurMode::None);
}

#[test]
fn keeps_frame_after_backwards_time_sharp() {
    let mut history = MotionBlurHistory::default();
    history.presented(sample(1.0, 0, Some((0.1, 0.1)), 0.5));
    let frame = history.presented(sample(0.9, 0, Some((0.4, 0.1)), 0.5));

    assert!(frame.cursor.is_none());
}

#[test]
fn skips_hidden_cursor() {
    let mut history = MotionBlurHistory::default();
    history.presented(sample(0.0, 0, Some((0.1, 0.1)), 0.5));
    let frame = history.presented(sample(FRAME_60, 0, None, 0.55));

    assert!(frame.cursor.is_none());
    // The recording layer is still moving, so display blur is unaffected.
    assert_eq!(frame.display.mode, MotionBlurMode::Zoom);
}

#[test]
fn keeps_returning_cursor_sharp() {
    let mut history = MotionBlurHistory::default();
    history.presented(sample(0.0, 0, Some((0.1, 0.1)), 0.5));
    history.presented(sample(FRAME_60, 0, None, 0.5));
    let reappeared = history.presented(sample(FRAME_60 * 2.0, 0, Some((0.6, 0.1)), 0.5));
    assert!(reappeared.cursor.is_none());

    let moving = history.presented(sample(FRAME_60 * 3.0, 0, Some((0.65, 0.1)), 0.5));
    assert!(moving.cursor.is_some());
}

#[test]
fn bypasses_zero_amount() {
    let mut history = MotionBlurHistory::default();
    let mut first = sample(0.0, 0, Some((0.1, 0.1)), 0.5);
    first.settings = MotionBlurSettings { amount: 0.0 };
    history.presented(first);

    let mut second = sample(FRAME_60, 0, Some((0.9, 0.9)), 0.9);
    second.settings = MotionBlurSettings { amount: 0.0 };
    let frame = history.presented(second);

    assert!(frame.cursor.is_none());
    assert_eq!(frame.display.mode, MotionBlurMode::None);
}

#[test]
fn normalizes_authored_amount() {
    assert_eq!(MotionBlurSettings { amount: 4.0 }.normalized().amount, 1.0);
    assert_eq!(MotionBlurSettings { amount: -1.0 }.normalized().amount, 0.0);
    assert_eq!(
        MotionBlurSettings {
            amount: f32::INFINITY
        }
        .normalized()
        .amount,
        MotionBlurSettings::default().amount
    );
    assert!(MotionBlurSettings { amount: 0.0 }.is_disabled());
}
