use super::{Constants, RecordingPass, canvas_clip, cover_source};
use crate::recorder::{
    composition::{self, NormalizedRect, SourceSize},
    motion_blur::{MotionBlurDescriptor, MotionBlurMode, Vec2},
    project_settings::ProjectSettings,
    rendering::{CanvasPlacement, CompositionState, PhysicalSize},
};

const SOURCE: SourceSize = SourceSize {
    width: 1920,
    height: 1080,
};

/// A 1600x900 preview surface with the canvas centred in its middle half.
fn preview_state(settings: &ProjectSettings) -> CompositionState {
    let target = PhysicalSize::new(1600, 900);
    CompositionState::new(
        target,
        CanvasPlacement {
            rect: NormalizedRect {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
            size: PhysicalSize::new(800, 450),
            corner_radius: 20.0,
            surround: [0.05, 0.05, 0.05, 1.0],
        },
        SOURCE,
        composition::evaluate(settings, SOURCE, 0, None),
        settings.canvas_composition.background.clone(),
        MotionBlurDescriptor::inactive(),
    )
}

fn export_state(settings: &ProjectSettings) -> CompositionState {
    let target = PhysicalSize::new(1920, 1080);
    CompositionState::new(
        target,
        CanvasPlacement::filling(target),
        SOURCE,
        composition::evaluate(settings, SOURCE, 0, None),
        settings.canvas_composition.background.clone(),
        MotionBlurDescriptor::inactive(),
    )
}

/// The recording keeps the source aspect because the composition already fitted
/// it to the canvas; the placement must not stretch it back out. Comparing the
/// two targets is the check that matters — the same frame drawn through a
/// different placement is the same shape.
#[test]
fn keeps_the_recording_aspect_in_both_targets() {
    let settings = ProjectSettings::default();
    let preview = preview_state(&settings);
    let export = export_state(&settings);

    let aspect = |state: &CompositionState| {
        let (constants, _) = Constants::recording(state);
        let [_, _, width, height] = constants.destination;
        // `destination` is normalized to the target, so it has to be returned
        // to pixels before it describes a shape.
        (width * state.target_size.width as f32) / (height * state.target_size.height as f32)
    };

    let source_aspect = 1920.0 / 1080.0;
    assert!((aspect(&export) - source_aspect).abs() < 0.01, "export");
    assert!((aspect(&preview) - source_aspect).abs() < 0.01, "preview");
}

#[test]
fn draws_the_recording_inside_the_canvas() {
    let settings = ProjectSettings::default();
    let state = preview_state(&settings);
    let (constants, pass) = Constants::recording(&state);
    let [x, y, width, height] = constants.destination;

    assert_eq!(pass, RecordingPass::Sharp);
    assert!(x >= 0.25 && x + width <= 0.75 + 1e-6, "{constants:?}");
    assert!(y >= 0.25 && y + height <= 0.75 + 1e-6, "{constants:?}");
}

#[test]
fn rounds_the_canvas_against_the_target_size() {
    let state = preview_state(&ProjectSettings::default());
    let fill = Constants::canvas_fill(&state);

    // The shader multiplies `destination.zw` by `misc.yz` to recover the quad's
    // own pixel size, so the target size is what belongs in `misc`.
    assert_eq!(fill.misc[0], state.canvas.corner_radius);
    assert_eq!(fill.misc[1], 1600.0);
    assert_eq!(fill.misc[2], 900.0);
    assert_eq!(fill.misc[3], composition::CANVAS_GRADIENT_ANGLE_DEGREES);
    assert!((fill.destination[2] * fill.misc[1] - 800.0).abs() < 1e-3);
}

/// Export draws a plain rectangle; the rounded canvas is editor presentation.
#[test]
fn leaves_the_exported_canvas_square() {
    let fill = Constants::canvas_fill(&export_state(&ProjectSettings::default()));

    assert_eq!(fill.misc[0], 0.0);
    assert_eq!(fill.destination, [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn selects_a_blur_pass_from_the_motion_descriptor() {
    let settings = ProjectSettings::default();
    let mut state = preview_state(&settings);

    state.motion_blur = MotionBlurDescriptor {
        mode: MotionBlurMode::Movement,
        movement_uv: Vec2::new(0.02, -0.01),
        strength: 0.5,
        ..MotionBlurDescriptor::inactive()
    };
    let (constants, pass) = Constants::recording(&state);
    assert_eq!(pass, RecordingPass::Movement);
    assert_eq!(constants.motion, [0.02, -0.01, 0.0, 0.5]);

    state.motion_blur = MotionBlurDescriptor {
        mode: MotionBlurMode::Zoom,
        zoom_center_uv: Vec2::new(0.3, 0.7),
        zoom_amount: -0.04,
        strength: 0.25,
        ..MotionBlurDescriptor::inactive()
    };
    let (constants, pass) = Constants::recording(&state);
    assert_eq!(pass, RecordingPass::Zoom);
    assert_eq!(constants.motion, [0.3, 0.7, -0.04, 0.25]);
}

/// The cover overflow lives in the source UVs, so the quad stays exactly the
/// canvas and cannot spill across the editor workspace.
#[test]
fn covers_the_canvas_through_source_uvs() {
    // A wide image on a 16:9 canvas overflows horizontally only.
    let [u0, v0, u1, v1] = cover_source(16.0 / 9.0, 3440, 1440);

    assert!(u0 > 0.0 && u1 < 1.0, "{u0} {u1}");
    assert!(
        (v0 - 0.0).abs() < 1e-6 && (v1 - 1.0).abs() < 1e-6,
        "{v0} {v1}"
    );
    assert!(((u0 + u1) / 2.0 - 0.5).abs() < 1e-6, "not centred");

    // A tall image overflows vertically instead.
    let [u0, v0, u1, v1] = cover_source(16.0 / 9.0, 1080, 1920);
    assert!(
        (u0 - 0.0).abs() < 1e-6 && (u1 - 1.0).abs() < 1e-6,
        "{u0} {u1}"
    );
    assert!(v0 > 0.0 && v1 < 1.0, "{v0} {v1}");
}

#[test]
fn clips_to_the_canvas_inside_the_target() {
    let state = preview_state(&ProjectSettings::default());

    assert_eq!(canvas_clip(&state), (400, 225, 1200, 675));
    assert_eq!(
        canvas_clip(&export_state(&ProjectSettings::default())),
        (0, 0, 1920, 1080)
    );
}

/// Editor viewport zoom can push the canvas past the surface. The clip has to
/// stay inside the target rather than becoming a negative or oversized rect.
#[test]
fn bounds_a_canvas_the_camera_pushed_off_screen() {
    let settings = ProjectSettings::default();
    let mut state = preview_state(&settings);
    state.canvas.rect = NormalizedRect {
        x: -0.5,
        y: -0.25,
        width: 3.0,
        height: 2.0,
    };

    assert_eq!(canvas_clip(&state), (0, 0, 1600, 900));
}

#[test]
fn places_the_cursor_when_one_is_visible() {
    let settings = ProjectSettings::default();
    let cursor = crate::recorder::cursor::CursorFrame {
        x: 0.5,
        y: 0.5,
        visible: true,
        scale: settings.cursor.scale,
        asset: settings.cursor.style.asset(),
    };
    let mut state = preview_state(&settings);
    state.frame = composition::evaluate(&settings, SOURCE, 0, Some(cursor));

    let constants = Constants::cursor(&state).expect("a visible cursor is placed");
    let [x, y, width, height] = constants.destination;

    assert!(width > 0.0 && height > 0.0, "{constants:?}");
    // Centred on the recording, so it sits inside the canvas rectangle.
    assert!(x > 0.25 && x < 0.75, "{constants:?}");
    assert!(y > 0.25 && y < 0.75, "{constants:?}");
}

#[test]
fn draws_no_cursor_when_it_is_hidden() {
    let state = preview_state(&ProjectSettings::default());

    assert!(Constants::cursor(&state).is_none());
}
