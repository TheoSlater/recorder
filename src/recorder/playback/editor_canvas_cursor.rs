use std::sync::Arc;

use gpui::*;

use super::super::{cursor::CursorFrame, media::PlaybackMetrics};

#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    window: &mut Window,
    video_composition: Bounds<Pixels>,
    composition_layer: Bounds<Pixels>,
    composition_radius: Pixels,
    cursor: Option<CursorFrame>,
    cursor_images: [Arc<RenderImage>; 2],
    video_width: u32,
    metrics: &PlaybackMetrics,
) {
    let Some(cursor) = cursor.filter(|cursor| cursor.visible) else {
        return;
    };
    let Some(cursor_bounds) = cursor_bounds(composition_layer, cursor, video_width) else {
        return;
    };

    let asset = cursor.asset;
    let image_index = asset.style().index().min(cursor_images.len() - 1);
    let cursor_started_at = std::time::Instant::now();
    // The cursor follows the transformed recording layer, but is allowed to
    // overflow it. Only the outer Video Composition clips this overlay.
    let _ = window.paint_image(
        video_composition,
        cursor_bounds,
        Corners::all(composition_radius),
        cursor_images[image_index].clone(),
        0,
        false,
    );
    metrics.cursor_painted(cursor_started_at.elapsed());
}

fn cursor_bounds(
    composition_layer: Bounds<Pixels>,
    cursor: CursorFrame,
    video_width: u32,
) -> Option<Bounds<Pixels>> {
    let render_scale =
        cursor.scale * composition_layer.size.width.as_f32() / video_width.max(1) as f32;
    if !render_scale.is_finite() || render_scale <= 0.0 {
        return None;
    }

    let asset = cursor.asset;
    Some(Bounds::new(
        point(
            px(composition_layer.origin.x.as_f32()
                + cursor.x * composition_layer.size.width.as_f32()
                - asset.hotspot_x() * render_scale),
            px(composition_layer.origin.y.as_f32()
                + cursor.y * composition_layer.size.height.as_f32()
                - asset.hotspot_y() * render_scale),
        ),
        size(
            px(asset.width() * render_scale),
            px(asset.height() * render_scale),
        ),
    ))
}

#[cfg(test)]
#[path = "editor_canvas_cursor_tests.rs"]
mod tests;
