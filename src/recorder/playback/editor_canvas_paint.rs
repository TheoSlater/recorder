use std::sync::Arc;
use std::time::Duration;

use gpui::*;

use super::super::{
    cursor::CursorFrame,
    media::{FrameTiming, PlaybackMetrics},
    project_settings::{CanvasBackgroundKind, CanvasComposition, CanvasView},
    zoom::ZoomEffect,
};
use super::{editor_canvas_controls::setting_color, editor_canvas_cursor, editor_canvas_geometry};

const CANVAS_RADIUS: Pixels = px(20.);

struct VideoPaintResult {
    visible: bool,
    submitted: bool,
    image_submission_duration: Duration,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_preview(
    window: &mut Window,
    stage: Bounds<Pixels>,
    image: Option<Arc<RenderImage>>,
    video_width: u32,
    video_height: u32,
    cursor: Option<CursorFrame>,
    cursor_images: [Arc<RenderImage>; 2],
    canvas_view: CanvasView,
    composition: CanvasComposition,
    background_image: Option<Arc<RenderImage>>,
    zoom_effect: Option<ZoomEffect>,
    stage_background: Hsla,
    canvas_background: Hsla,
    border: Hsla,
    selection: Hsla,
    shadow: Hsla,
    selected_recording: bool,
    metrics: PlaybackMetrics,
    frame_timing: Option<FrameTiming>,
    frame_invalidated_at: Option<std::time::Instant>,
    playing: bool,
) {
    let canvas_paint_started_at = std::time::Instant::now();
    // The stage is editor chrome and is never part of the exported video.
    window.paint_quad(fill(stage, stage_background));

    let video_aspect = if video_width > 0 && video_height > 0 {
        video_width as f32 / video_height as f32
    } else {
        16. / 9.
    };
    let geometry = editor_canvas_geometry::preview_geometry(
        stage,
        canvas_view,
        &composition,
        video_aspect,
        zoom_effect,
        cursor,
    );
    let canvas = geometry.canvas;
    // The canvas is the export surface. Its background is painted before the
    // screen recording object so padding remains visible around the layer.
    let background = match composition.background.kind {
        CanvasBackgroundKind::Gradient => {
            let start = setting_color(
                composition.background.gradient_start.as_deref(),
                canvas_background,
            );
            let end = setting_color(
                composition.background.gradient_end.as_deref(),
                stage_background,
            );
            linear_gradient(
                135.0,
                linear_color_stop(start, 0.0),
                linear_color_stop(end, 1.0),
            )
        }
        CanvasBackgroundKind::Solid | CanvasBackgroundKind::Image => setting_color(
            composition.background.solid_color.as_deref(),
            canvas_background,
        )
        .into(),
    };
    window.paint_quad(quad(
        canvas,
        CANVAS_RADIUS,
        background,
        px(0.),
        border,
        BorderStyle::Solid,
    ));

    if composition.background.kind == CanvasBackgroundKind::Image
        && let Some(background_image) = background_image
    {
        let image_size = background_image.size(0).to_pixels(window.scale_factor());
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
    let composition_layer = geometry.composition_layer;
    let canvas_clip = stage.intersect(&canvas);
    // Zoom can move the object beyond the composition, so the composition
    // owns the clip and the screen-recording layer owns the transformed paint.
    let video_paint = window.with_content_mask(
        Some(ContentMask {
            bounds: canvas_clip,
        }),
        |window| {
            paint_screen_recording_layer(
                window,
                canvas,
                composition_layer,
                geometry.composition_radius,
                image,
                video_width,
                cursor,
                cursor_images,
                shadow,
                composition.shadow,
                selection,
                selected_recording,
                geometry.resize_handle,
                metrics.clone(),
            )
        },
    );

    // Paint the guide last so a recording scaled into the canvas cannot hide
    // the export boundary.
    window.paint_quad(quad(
        canvas,
        CANVAS_RADIUS,
        transparent_black(),
        px(1.),
        border,
        BorderStyle::Solid,
    ));

    let canvas_paint_duration = canvas_paint_started_at.elapsed();
    if let Some(timing) = frame_timing.as_ref()
        && let Some(invalidated_at) = frame_invalidated_at
        && playing
    {
        if let Some(video_paint) = video_paint
            && video_paint.submitted
            && video_paint.visible
        {
            metrics.presented(
                timing.sequence,
                timing.ready_at,
                invalidated_at,
                timing.scheduled_at,
                canvas_paint_started_at,
                canvas_paint_duration,
                canvas_paint_duration.saturating_sub(video_paint.image_submission_duration),
                video_paint.image_submission_duration,
                std::time::Instant::now(),
            );
        } else {
            metrics.paint_failed();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_screen_recording_layer(
    window: &mut Window,
    canvas: Bounds<Pixels>,
    composition_layer: Bounds<Pixels>,
    radius: Pixels,
    image: Option<Arc<RenderImage>>,
    video_width: u32,
    cursor: Option<CursorFrame>,
    cursor_images: [Arc<RenderImage>; 2],
    shadow: Hsla,
    has_shadow: bool,
    selection: Hsla,
    selected: bool,
    resize_handle: Bounds<Pixels>,
    metrics: PlaybackMetrics,
) -> Option<VideoPaintResult> {
    if has_shadow {
        let box_shadow = BoxShadow::new(px(0.), px(12.), shadow)
            .blur_radius(px(24.))
            .spread_radius(px(0.));
        window.paint_drop_shadows(composition_layer, Corners::all(radius), &[box_shadow]);
    }

    let result = image.map(|image| {
        let visible = canvas.intersect(&composition_layer);
        let image_is_visible =
            visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO;
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
            composition_layer,
            CANVAS_RADIUS,
            cursor,
            cursor_images,
            video_width,
            &metrics,
        );

        VideoPaintResult {
            visible: image_is_visible,
            submitted: paint_result.is_ok(),
            image_submission_duration,
        }
    });

    if selected {
        window.paint_quad(quad(
            composition_layer,
            Corners::all(radius),
            transparent_black(),
            px(1.),
            selection,
            BorderStyle::Solid,
        ));
        window.paint_quad(quad(
            resize_handle,
            px(3.),
            selection,
            px(0.),
            selection,
            BorderStyle::Solid,
        ));
    }

    result
}
