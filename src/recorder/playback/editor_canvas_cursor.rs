use std::sync::Arc;

use gpui::*;

use super::super::{
    composition::{self, CompositionFrame, SourceSize},
    cursor::CursorFrame,
    media::PlaybackMetrics,
};
use super::editor_canvas_cursor_blur::BlurredCursor;

#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    window: &mut Window,
    video_composition: Bounds<Pixels>,
    composition_frame: CompositionFrame,
    composition_radius: Pixels,
    cursor: Option<CursorFrame>,
    cursor_images: [Arc<RenderImage>; 2],
    blurred: Option<&BlurredCursor>,
    source: SourceSize,
    metrics: &PlaybackMetrics,
) {
    let Some(cursor) = cursor.filter(|cursor| cursor.visible) else {
        return;
    };
    let Some(sharp_bounds) = cursor_bounds(video_composition, composition_frame, source, cursor.asset) else {
        return;
    };

    // The smeared sprite replaces the sharp one. Painting both would leave a
    // stationary sharp cursor showing through the smear.
    let (image, bounds) = match blurred {
        Some(blurred) => (
            blurred.image.clone(),
            Bounds::new(
                point(
                    sharp_bounds.origin.x - blurred.offset.x,
                    sharp_bounds.origin.y - blurred.offset.y,
                ),
                blurred.size,
            ),
        ),
        None => {
            let index = cursor.asset.style().index().min(cursor_images.len() - 1);
            (cursor_images[index].clone(), sharp_bounds)
        }
    };

    let cursor_started_at = std::time::Instant::now();
    // The cursor follows the transformed recording layer, but is allowed to
    // overflow it. Only the outer Video Composition clips this overlay.
    let _ = window.paint_image(
        video_composition,
        bounds,
        Corners::all(composition_radius),
        image,
        0,
        false,
    );
    metrics.cursor_painted(cursor_started_at.elapsed());
}

/// The sharp cursor's rendered placement. Shared with the motion blur pipeline
/// so the smeared sprite is built at exactly the size it will be drawn.
pub(super) fn cursor_bounds(
    canvas: Bounds<Pixels>,
    composition_frame: CompositionFrame,
    source: SourceSize,
    asset: super::super::cursor_settings::CursorAsset,
) -> Option<Bounds<Pixels>> {
    let rect = composition::cursor_rect(&composition_frame, source, asset)?;
    Some(Bounds::new(
        point(
            px(canvas.origin.x.as_f32() + rect.x as f32 * canvas.size.width.as_f32()),
            px(canvas.origin.y.as_f32() + rect.y as f32 * canvas.size.height.as_f32()),
        ),
        size(
            px(rect.width as f32 * canvas.size.width.as_f32()),
            px(rect.height as f32 * canvas.size.height.as_f32()),
        ),
    ))
}

#[cfg(test)]
#[path = "editor_canvas_cursor_tests.rs"]
mod tests;
