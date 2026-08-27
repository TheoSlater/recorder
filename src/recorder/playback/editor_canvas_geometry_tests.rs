use super::{CanvasHit, canvas_placement, hit_test, needs_recenter, preview_geometry};
use crate::recorder::project_settings::{AspectRatioPreset, CanvasComposition, CanvasView};
use crate::recorder::rendering::{CanvasPlacement, PhysicalSize};
use gpui::{Bounds, Pixels, point, px, size};

#[test]
fn composition_geometry_stays_centered_and_respects_padding() {
    let geometry = preview_geometry(
        Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(600.0))),
        CanvasView::default(),
        &CanvasComposition::default(),
        1920,
        1080,
        None,
        None,
    );

    assert!((geometry.canvas.size.width.as_f32() - 1000.0).abs() < 0.01);
    assert!((geometry.recording_layer.center().x.as_f32() - 500.0).abs() < 0.01);
    assert!((geometry.recording_layer.center().y.as_f32() - 300.0).abs() < 0.01);
    assert!(geometry.recording_layer.size.width < geometry.canvas.size.width);
    assert_eq!(
        hit_test(geometry, geometry.recording_layer.center()),
        Some(CanvasHit::Recording)
    );
    assert_eq!(
        hit_test(geometry, geometry.resize_handle.center()),
        Some(CanvasHit::Resize)
    );
}

#[test]
fn composition_geometry_follows_aspect_position_and_scale() {
    let composition = CanvasComposition {
        aspect_ratio: AspectRatioPreset::Square,
        position_x: 0.2,
        position_y: -0.1,
        scale: 0.5,
        ..CanvasComposition::default()
    };

    let geometry = preview_geometry(
        Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(600.0))),
        CanvasView::default(),
        &composition,
        1920,
        1080,
        None,
        None,
    );

    assert!((geometry.canvas.size.width.as_f32() - 600.0).abs() < 0.01);
    assert!((geometry.canvas.size.height.as_f32() - 600.0).abs() < 0.01);
    assert!((geometry.recording_layer.center().x.as_f32() - 620.0).abs() < 0.01);
    assert!((geometry.recording_layer.center().y.as_f32() - 240.0).abs() < 0.01);
    assert!((geometry.recording_layer.size.width.as_f32() - 252.0).abs() < 0.01);
}

#[test]
fn hit_testing_stays_inside_export_canvas() {
    let composition = CanvasComposition {
        scale: 2.0,
        ..CanvasComposition::default()
    };
    let geometry = preview_geometry(
        Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(600.0))),
        CanvasView::default(),
        &composition,
        1920,
        1080,
        None,
        None,
    );

    assert!(geometry.recording_layer.origin.x < geometry.canvas.origin.x);
    assert_eq!(
        hit_test(
            geometry,
            point(
                px(geometry.canvas.origin.x.as_f32() - 1.0),
                geometry.canvas.center().y,
            ),
        ),
        None
    );
    assert_eq!(
        hit_test(geometry, geometry.canvas.center()),
        Some(CanvasHit::Recording)
    );
}

#[test]
fn zoom_transforms_the_recording_layer() {
    let base = preview_geometry(
        Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(600.0))),
        CanvasView::default(),
        &CanvasComposition::default(),
        1920,
        1080,
        None,
        None,
    );
    let zoomed = preview_geometry(
        Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(600.0))),
        CanvasView::default(),
        &CanvasComposition::default(),
        1920,
        1080,
        Some(crate::recorder::zoom::ZoomEffect {
            scale: 2.0,
            target: crate::recorder::zoom::ZoomTarget::CanvasCenter,
        }),
        None,
    );

    assert_eq!(
        zoomed.composition_layer.center(),
        base.recording_layer.center()
    );
    assert!(
        (zoomed.composition_layer.size.width.as_f32()
            - base.recording_layer.size.width.as_f32() * 2.0)
            .abs()
            < 0.01
    );
    assert!(
        (zoomed.composition_layer.size.height.as_f32()
            - base.recording_layer.size.height.as_f32() * 2.0)
            .abs()
            < 0.01
    );
}

#[test]
fn recording_transform_ignores_editor_camera() {
    let stage = Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(600.0)));
    let composition = CanvasComposition {
        position_x: 0.1,
        position_y: -0.05,
        scale: 0.8,
        ..CanvasComposition::default()
    };
    let framed = |view| preview_geometry(stage, view, &composition, 1920, 1080, None, None);

    let resting = framed(CanvasView::default())
        .recording_transform
        .expect("canvas has area");
    let navigated = framed(CanvasView {
        zoom: 2.5,
        pan_x: 120.0,
        pan_y: -80.0,
    })
    .recording_transform
    .expect("canvas has area");

    assert!((resting.center.x - navigated.center.x).abs() < 1e-4);
    assert!((resting.center.y - navigated.center.y).abs() < 1e-4);
    assert!((resting.size.x - navigated.size.x).abs() < 1e-4);
    assert!((resting.size.y - navigated.size.y).abs() < 1e-4);
}

#[test]
fn recording_aspect_survives_canvas_resize_and_presets() {
    let stages = [
        Bounds::new(point(px(1000.0), px(80.0)), size(px(1000.0), px(600.0))),
        Bounds::new(point(px(40.0), px(20.0)), size(px(1600.0), px(1000.0))),
    ];
    let presets = [
        AspectRatioPreset::Widescreen,
        AspectRatioPreset::Standard,
        AspectRatioPreset::Square,
        AspectRatioPreset::Portrait,
        AspectRatioPreset::Vertical,
    ];

    for stage in stages {
        for preset in presets {
            let composition = CanvasComposition {
                aspect_ratio: preset,
                ..CanvasComposition::default()
            };
            let geometry = preview_geometry(
                stage,
                CanvasView::default(),
                &composition,
                1920,
                1080,
                None,
                None,
            );
            let recording = geometry.recording_layer.size;
            let actual = recording.width.as_f32() / recording.height.as_f32();

            assert!((actual - 1920.0 / 1080.0).abs() < 0.0001);
        }
    }
}

#[test]
fn editor_camera_produces_no_display_motion_blur() {
    use crate::recorder::motion_blur::{
        MotionBlurMode, MotionBlurSettings, compute_display_motion_blur,
    };

    let stage = Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(600.0)));
    let composition = CanvasComposition::default();
    let framed = |view| preview_geometry(stage, view, &composition, 1920, 1080, None, None);

    let resting = framed(CanvasView::default())
        .recording_transform
        .expect("canvas has area");
    let navigated = framed(CanvasView {
        zoom: 3.0,
        pan_x: -200.0,
        pan_y: 90.0,
    })
    .recording_transform
    .expect("canvas has area");

    // Panning and zooming the workspace between two frames is navigation, not
    // composition movement, so it must classify as no motion at all.
    let blur = compute_display_motion_blur(
        resting,
        navigated,
        0.0,
        1.0 / 60.0,
        MotionBlurMode::None,
        crate::recorder::motion_blur::Vec2::new(0.5, 0.5),
        MotionBlurSettings { amount: 1.0 },
    );

    assert_eq!(blur.mode, MotionBlurMode::None);
}

#[test]
fn recenter_is_needed_for_panning_or_clipping() {
    assert!(!needs_recenter(CanvasView::default()));
    assert!(!needs_recenter(CanvasView {
        zoom: 0.75,
        ..CanvasView::default()
    }));
    assert!(needs_recenter(CanvasView {
        zoom: 1.1,
        ..CanvasView::default()
    }));
    assert!(needs_recenter(CanvasView {
        pan_x: 1.0,
        ..CanvasView::default()
    }));
    assert!(needs_recenter(CanvasView {
        pan_y: -1.0,
        ..CanvasView::default()
    }));
}

const SURROUND: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

fn stage() -> Bounds<Pixels> {
    Bounds::new(point(px(40.0), px(20.0)), size(px(1000.0), px(600.0)))
}

fn placement(view: CanvasView, scale_factor: f32) -> CanvasPlacement {
    let geometry = preview_geometry(
        stage(),
        view,
        &CanvasComposition::default(),
        1920,
        1080,
        None,
        None,
    );
    canvas_placement(stage(), geometry.canvas, SURROUND, scale_factor)
        .expect("the stage is a usable rectangle")
}

/// The surface covers the stage, so a fitted canvas is a sub-rectangle of it —
/// and drawing the composition across the whole surface is exactly the stretch
/// this conversion exists to prevent.
#[test]
fn places_the_fitted_canvas_inside_the_stage() {
    let placement = placement(CanvasView::default(), 1.0);

    // 16:9 fitted into 1000x600 is 1000x562.5, centred vertically.
    assert!((placement.rect.width - 1.0).abs() < 1e-6, "{placement:?}");
    assert!((placement.rect.x).abs() < 1e-6, "{placement:?}");
    assert!(placement.rect.height < 1.0);
    assert!(
        (placement.rect.y - (1.0 - placement.rect.height) / 2.0).abs() < 1e-6,
        "{placement:?}"
    );
    assert_eq!(placement.size, PhysicalSize::new(1000, 563));
    assert_eq!(placement.surround, SURROUND);
}

#[test]
fn scales_the_canvas_and_its_radius_with_dpi() {
    let single = placement(CanvasView::default(), 1.0);
    let double = placement(CanvasView::default(), 2.0);

    // The rectangle is normalized, so DPI changes only the device-pixel values.
    assert_eq!(single.rect, double.rect);
    assert_eq!(double.size.width, single.size.width * 2);
    assert!((double.corner_radius - single.corner_radius * 2.0).abs() < 1e-4);
}

/// Viewport zoom and pan may move and scale where the canvas appears, and
/// nothing else. The composition inside it is evaluated separately and never
/// sees these values.
#[test]
fn the_editor_camera_reaches_the_renderer_only_as_placement() {
    let resting = placement(CanvasView::default(), 1.0);
    let navigated = placement(
        CanvasView {
            zoom: 2.0,
            pan_x: 30.0,
            pan_y: -10.0,
        },
        1.0,
    );

    assert!(navigated.rect.width > resting.rect.width * 1.9);
    assert!(navigated.rect.x < resting.rect.x);
    assert_eq!(navigated.size.width, resting.size.width * 2);
}

#[test]
fn rejects_a_collapsed_stage() {
    let empty = Bounds::new(point(px(0.0), px(0.0)), size(px(0.0), px(600.0)));

    assert!(canvas_placement(empty, stage(), SURROUND, 1.0).is_none());
}
