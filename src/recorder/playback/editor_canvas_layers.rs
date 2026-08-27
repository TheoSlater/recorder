//! The composition layers, painted by GPUI.
//!
//! Everything in this module is what the native compositor takes over: the
//! editor workspace fill, the canvas background, the screen recording, and the
//! reconstructed cursor. Nothing here is editor chrome — the canvas guide, the
//! selection outline, and the resize handle stay in
//! [`editor_canvas_paint`](super::editor_canvas_paint) and keep being painted
//! over the native surface.
//!
//! Keeping the two apart is what makes the migration reversible: while the
//! native path is being validated this is the fallback preview, and when it is
//! retired this module is deleted whole rather than unpicked.

use std::sync::Arc;
use std::time::Duration;

use gpui::*;

use super::super::{
    composition::{self, SourceSize},
    cursor::CursorFrame,
    media::PlaybackMetrics,
    project_settings::{CanvasBackgroundKind, CanvasComposition},
};
use super::{
    editor_canvas_controls::setting_color,
    editor_canvas_cursor,
    editor_canvas_cursor_blur::BlurredCursor,
    editor_canvas_geometry,
    editor_canvas_geometry::{CANVAS_RADIUS, CanvasGeometry},
};

pub(super) struct VideoPaint {
    pub(super) visible: bool,
    pub(super) submitted: bool,
    pub(super) image_submission_duration: Duration,
}

/// Everything the composition layers need that the geometry does not carry.
pub(super) struct Layers {
    pub(super) image: Option<Arc<RenderImage>>,
    pub(super) video_width: u32,
    pub(super) video_height: u32,
    pub(super) cursor: Option<CursorFrame>,
    pub(super) cursor_images: [Arc<RenderImage>; 2],
    pub(super) blurred_cursor: Option<BlurredCursor>,
    pub(super) composition: CanvasComposition,
    pub(super) background_image: Option<Arc<RenderImage>>,
    pub(super) stage_background: Hsla,
    pub(super) canvas_background: Hsla,
    pub(super) border: Hsla,
    pub(super) shadow: Hsla,
}

/// The editor workspace behind the canvas. Never part of the exported video.
pub(super) fn paint_stage(window: &mut Window, stage: Bounds<Pixels>, background: Hsla) {
    window.paint_quad(fill(stage, background));
}

/// Paints the canvas background, the transformed recording, and the cursor.
pub(super) fn paint(
    window: &mut Window,
    stage: Bounds<Pixels>,
    geometry: CanvasGeometry,
    layers: Layers,
    metrics: &PlaybackMetrics,
) -> Option<VideoPaint> {
    paint_background(window, geometry.canvas, &layers, window.scale_factor());

    // Zoom can move the object beyond the composition, so the composition owns
    // the clip and the screen-recording layer owns the transformed paint.
    let canvas_clip = stage.intersect(&geometry.canvas);
    window.with_content_mask(
        Some(ContentMask {
            bounds: canvas_clip,
        }),
        |window| paint_recording(window, geometry, layers, metrics),
    )
}

fn paint_background(
    window: &mut Window,
    canvas: Bounds<Pixels>,
    layers: &Layers,
    scale_factor: f32,
) {
    let composition = &layers.composition;
    // The canvas is the export surface. Its background is painted before the
    // screen recording object so padding remains visible around the layer.
    let background = match composition.background.kind {
        CanvasBackgroundKind::Gradient => {
            let start = setting_color(
                composition.background.gradient_start.as_deref(),
                layers.canvas_background,
            );
            let end = setting_color(
                composition.background.gradient_end.as_deref(),
                layers.stage_background,
            );
            linear_gradient(
                composition::CANVAS_GRADIENT_ANGLE_DEGREES,
                linear_color_stop(start, 0.0),
                linear_color_stop(end, 1.0),
            )
        }
        CanvasBackgroundKind::Solid | CanvasBackgroundKind::Image => setting_color(
            composition.background.solid_color.as_deref(),
            layers.canvas_background,
        )
        .into(),
    };
    window.paint_quad(quad(
        canvas,
        CANVAS_RADIUS,
        background,
        px(0.),
        layers.border,
        BorderStyle::Solid,
    ));

    if composition.background.kind != CanvasBackgroundKind::Image {
        return;
    }
    let Some(background_image) = layers.background_image.clone() else {
        return;
    };
    let image_size = background_image.size(0).to_pixels(scale_factor);
    let image_aspect = if image_size.height > Pixels::ZERO {
        image_size.width.as_f32() / image_size.height.as_f32()
    } else {
        16. / 9.
    };
    let image_bounds = editor_canvas_geometry::cover_bounds(canvas, image_aspect);
    let _ = window.paint_image(
        canvas,
        image_bounds,
        Corners::all(CANVAS_RADIUS),
        background_image,
        0,
        false,
    );
}

fn paint_recording(
    window: &mut Window,
    geometry: CanvasGeometry,
    layers: Layers,
    metrics: &PlaybackMetrics,
) -> Option<VideoPaint> {
    let canvas = geometry.canvas;
    let composition_layer = geometry.composition_layer;
    let radius = geometry.composition_radius;
    if layers.composition.shadow {
        let box_shadow = BoxShadow::new(px(0.), px(12.), layers.shadow)
            .blur_radius(px(24.))
            .spread_radius(px(0.));
        window.paint_drop_shadows(composition_layer, Corners::all(radius), &[box_shadow]);
    }

    let image = layers.image?;
    let visible = canvas.intersect(&composition_layer);
    let image_is_visible = visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO;
    let paint_started_at = std::time::Instant::now();
    let paint_result = window.paint_image(
        composition_layer,
        composition_layer,
        Corners::all(radius),
        image,
        0,
        false,
    );
    let image_submission_duration = paint_started_at.elapsed();

    editor_canvas_cursor::paint(
        window,
        canvas,
        geometry.composition_frame,
        CANVAS_RADIUS,
        layers.cursor,
        layers.cursor_images,
        layers.blurred_cursor.as_ref(),
        SourceSize {
            width: layers.video_width,
            height: layers.video_height,
        },
        metrics,
    );

    Some(VideoPaint {
        visible: image_is_visible,
        submitted: paint_result.is_ok(),
        image_submission_duration,
    })
}
