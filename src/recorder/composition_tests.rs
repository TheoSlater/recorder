use super::*;

fn source() -> SourceSize {
    SourceSize {
        width: 1920,
        height: 1080,
    }
}

#[test]
fn output_dimensions_follow_aspect_and_long_edge() {
    assert_eq!(
        OutputSize::for_source(source(), AspectRatioPreset::Widescreen),
        OutputSize {
            width: 1920,
            height: 1080
        }
    );
    assert_eq!(
        OutputSize::for_source(source(), AspectRatioPreset::Square),
        OutputSize {
            width: 1920,
            height: 1920
        }
    );
    assert_eq!(
        OutputSize::for_source(source(), AspectRatioPreset::Vertical),
        OutputSize {
            width: 1080,
            height: 1920
        }
    );
}

#[test]
fn zoom_focus_follows_the_cursor_target() {
    let cursor = CursorFrame {
        x: 0.2,
        y: 0.75,
        visible: true,
        scale: 1.0,
        asset: CursorStyle::Default.asset(),
    };
    let zoom = ZoomEffect {
        scale: 1.5,
        target: ZoomTarget::Cursor,
    };
    let settings = ProjectSettings::default();
    let frame = evaluate_with_aspect(
        &settings.canvas_composition,
        source(),
        OutputSize::for_source(source(), settings.canvas_composition.aspect_ratio),
        Some(zoom),
        Some(cursor),
    );

    let focus = frame.zoom_center();
    assert!((focus.x - 0.2).abs() < 1e-5, "{focus:?}");
    assert!((focus.y - 0.75).abs() < 1e-5, "{focus:?}");
}

#[test]
fn zoom_focus_uses_the_centre_for_centre_targets() {
    let settings = ProjectSettings::default();
    let framed = |zoom| {
        evaluate_with_aspect(
            &settings.canvas_composition,
            source(),
            OutputSize::for_source(source(), settings.canvas_composition.aspect_ratio),
            zoom,
            None,
        )
        .zoom_center()
    };

    for zoom in [
        Some(ZoomEffect {
            scale: 1.5,
            target: ZoomTarget::CanvasCenter,
        }),
        // A cursor-targeted zoom with no cursor to follow, and no zoom at all,
        // both fall back to the centre rather than an unset point.
        Some(ZoomEffect {
            scale: 1.5,
            target: ZoomTarget::Cursor,
        }),
        None,
    ] {
        let focus = framed(zoom);
        assert_eq!((focus.x, focus.y), (0.5, 0.5));
    }
}

#[test]
fn viewport_values_do_not_change_export_frame() {
    let first = ProjectSettings::default();
    let mut second = first.clone();
    second.canvas.zoom = 4.0;
    second.canvas.pan_x = 80.0;
    second.canvas.pan_y = -40.0;
    let frame_a = evaluate_with_aspect(
        &first.canvas_composition,
        source(),
        OutputSize::for_source(source(), first.canvas_composition.aspect_ratio),
        None,
        None,
    );
    let frame_b = evaluate_with_aspect(
        &second.canvas_composition,
        source(),
        OutputSize::for_source(source(), second.canvas_composition.aspect_ratio),
        None,
        None,
    );
    assert_eq!(frame_a, frame_b);
}

#[test]
fn zoom_changes_only_the_recording_transform() {
    let composition = CanvasComposition::default();
    let output = OutputSize::for_source(source(), composition.aspect_ratio);
    let plain = evaluate_with_aspect(&composition, source(), output, None, None);
    let zoom = evaluate_with_aspect(
        &composition,
        source(),
        output,
        Some(ZoomEffect {
            scale: 2.0,
            target: ZoomTarget::CanvasCenter,
        }),
        None,
    );
    assert_eq!(plain.base_recording, zoom.base_recording);
    assert!(zoom.recording.width > plain.recording.width);
}

#[test]
fn normalized_settings_evaluate_deterministically() {
    let settings = ProjectSettings::default().normalized();
    let frame_a = evaluate(&settings, source(), 123_456, None);
    let frame_b = evaluate(&settings, source(), 123_456, None);
    assert_eq!(frame_a, frame_b);
}

#[test]
fn recording_rect_preserves_source_aspect_in_vertical_output() {
    let composition = CanvasComposition::default();
    let output = OutputSize {
        width: 1080,
        height: 1920,
    };
    let frame = evaluate_with_aspect(&composition, source(), output, None, None);
    let pixel_aspect = frame.recording.width * output.aspect() / frame.recording.height;

    assert!((pixel_aspect - source().aspect()).abs() < 0.000_001);
}
