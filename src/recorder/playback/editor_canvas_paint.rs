use std::sync::Arc;

use gpui::*;

use super::super::{
    cursor::CursorFrame,
    media::{FrameTiming, PlaybackMetrics},
    project_settings::{CanvasComposition, CanvasView},
    zoom::ZoomEffect,
};
use super::{
    editor_canvas_cursor_blur::BlurredCursor,
    editor_canvas_geometry,
    editor_canvas_geometry::{CANVAS_RADIUS, CanvasGeometry},
    editor_canvas_layers::{self, Layers, VideoPaint},
};

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_preview(
    window: &mut Window,
    stage: Bounds<Pixels>,
    image: Option<Arc<RenderImage>>,
    video_width: u32,
    video_height: u32,
    cursor: Option<CursorFrame>,
    cursor_images: [Arc<RenderImage>; 2],
    blurred_cursor: Option<BlurredCursor>,
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
    composed_natively: bool,
) {
    let canvas_paint_started_at = std::time::Instant::now();
    let geometry = editor_canvas_geometry::preview_geometry(
        stage,
        canvas_view,
        &composition,
        video_width,
        video_height,
        zoom_effect,
        cursor,
    );

    // Everything the native compositor owns is skipped wholesale rather than
    // layered over: GPUI paints above the native surface, so a single opaque
    // fill left here would hide the composition entirely.
    let video_paint = if composed_natively {
        None
    } else {
        editor_canvas_layers::paint_stage(window, stage, stage_background);
        editor_canvas_layers::paint(
            window,
            stage,
            geometry,
            Layers {
                image,
                video_width,
                video_height,
                cursor,
                cursor_images,
                blurred_cursor,
                composition,
                background_image,
                stage_background,
                canvas_background,
                border,
                shadow,
            },
            &metrics,
        )
    };

    paint_chrome(
        window,
        stage,
        geometry,
        border,
        selection,
        selected_recording,
    );

    if composed_natively {
        // The native path counts its own presented frames; a GPUI paint that
        // deliberately drew no video is not a dropped frame.
        return;
    }
    report_paint(
        &metrics,
        frame_timing.as_ref(),
        frame_invalidated_at,
        playing,
        video_paint,
        canvas_paint_started_at,
    );
}

/// Editor chrome over the composition: the export boundary, the selection
/// outline, and the resize handle. These stay in GPUI whichever renderer draws
/// the video, because they are interaction affordances rather than composition.
fn paint_chrome(
    window: &mut Window,
    stage: Bounds<Pixels>,
    geometry: CanvasGeometry,
    border: Hsla,
    selection: Hsla,
    selected_recording: bool,
) {
    if selected_recording {
        let canvas_clip = stage.intersect(&geometry.canvas);
        window.with_content_mask(
            Some(ContentMask {
                bounds: canvas_clip,
            }),
            |window| {
                window.paint_quad(quad(
                    geometry.composition_layer,
                    Corners::all(geometry.composition_radius),
                    transparent_black(),
                    px(1.),
                    selection,
                    BorderStyle::Solid,
                ));
                window.paint_quad(quad(
                    geometry.resize_handle,
                    px(3.),
                    selection,
                    px(0.),
                    selection,
                    BorderStyle::Solid,
                ));
            },
        );
    }

    // Painted last so a recording scaled into the canvas cannot hide the
    // export boundary.
    window.paint_quad(quad(
        geometry.canvas,
        CANVAS_RADIUS,
        transparent_black(),
        px(1.),
        border,
        BorderStyle::Solid,
    ));
}

fn report_paint(
    metrics: &PlaybackMetrics,
    frame_timing: Option<&FrameTiming>,
    frame_invalidated_at: Option<std::time::Instant>,
    playing: bool,
    video_paint: Option<VideoPaint>,
    started_at: std::time::Instant,
) {
    let duration = started_at.elapsed();
    let (Some(timing), Some(invalidated_at)) = (frame_timing, frame_invalidated_at) else {
        return;
    };
    if !playing {
        return;
    }
    let Some(video_paint) = video_paint.filter(|paint| paint.submitted && paint.visible) else {
        metrics.paint_failed();
        return;
    };
    metrics.presented(
        timing.sequence,
        timing.ready_at,
        invalidated_at,
        timing.scheduled_at,
        started_at,
        duration,
        duration.saturating_sub(video_paint.image_submission_duration),
        video_paint.image_submission_duration,
        std::time::Instant::now(),
    );
}
