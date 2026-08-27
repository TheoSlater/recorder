use gpui::*;

use super::super::thumbnails::{self, ThumbnailStrip};
use super::editor_timeline::{TRACK_HEIGHT, TimelineState, content_width, track_y};

const LANE_INSET: f32 = 3.;
const CELL_OVERLAP: f32 = 0.5;
const CORNER_RADIUS: f32 = 3.;

pub(super) fn paint(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    state: TimelineState,
    strip: &ThumbnailStrip,
    border: Hsla,
    tint: Hsla,
) {
    if strip.slots.is_empty() || content_width(bounds) <= 0. {
        return;
    }

    let lane = Bounds::new(
        point(
            bounds.origin.x,
            px(bounds.origin.y.as_f32() + track_y(0) + LANE_INSET),
        ),
        size(
            bounds.size.width,
            px((TRACK_HEIGHT - LANE_INSET * 2.).max(0.)),
        ),
    );
    for slot in &strip.slots {
        let start_x = state.time_to_x(slot.start_us, bounds);
        let end_x = state.time_to_x(slot.end_us, bounds);
        let width = end_x - start_x;
        if width <= 0. {
            continue;
        }
        let cell = Bounds::new(
            point(px(start_x - CELL_OVERLAP), lane.origin.y),
            size(px(width + CELL_OVERLAP * 2.), lane.size.height),
        );
        let Some(clipped) = thumbnails::clip_to_viewport(cell, lane) else {
            continue;
        };
        let Some(image) = &slot.image else {
            continue;
        };
        let image_size = image.size(0);
        let image_aspect = if image_size.height.0 > 0 {
            image_size.width.0 as f32 / image_size.height.0 as f32
        } else {
            16. / 9.
        };
        let image_bounds = thumbnails::aspect_fill_bounds(cell, image_aspect);
        let _ = window.paint_image(
            clipped,
            image_bounds,
            Corners::all(px(CORNER_RADIUS)),
            image.clone(),
            0,
            false,
        );
        window.paint_quad(quad(
            clipped,
            px(CORNER_RADIUS),
            tint.opacity(0.08),
            px(1.),
            border.opacity(0.18),
            BorderStyle::Solid,
        ));
    }
}
