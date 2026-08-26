use super::{CanvasHit, hit_test, preview_geometry};
use crate::recorder::project_settings::{AspectRatioPreset, CanvasComposition, CanvasView};
use gpui::{Bounds, point, px, size};

#[test]
fn composition_geometry_stays_centered_and_respects_padding() {
    let geometry = preview_geometry(
        Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(600.0))),
        CanvasView::default(),
        &CanvasComposition::default(),
        16.0 / 9.0,
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
        16.0 / 9.0,
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
        16.0 / 9.0,
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
        16.0 / 9.0,
        None,
        None,
    );
    let zoomed = preview_geometry(
        Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(600.0))),
        CanvasView::default(),
        &CanvasComposition::default(),
        16.0 / 9.0,
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
