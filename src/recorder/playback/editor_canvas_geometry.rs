use gpui::*;

use super::super::{
    cursor::CursorFrame,
    project_settings::{CanvasComposition, CanvasView},
    zoom::{ZoomEffect, ZoomTarget},
};

const DEFAULT_VIDEO_ASPECT: f32 = 16. / 9.;
const HANDLE_SIZE: f32 = 12.;

#[derive(Clone, Copy, Debug)]
pub(super) struct CanvasGeometry {
    pub(super) canvas: Bounds<Pixels>,
    /// The untransformed screen recording object inside the export canvas.
    #[allow(dead_code)]
    pub(super) recording_layer: Bounds<Pixels>,
    /// The screen recording object after the active composition transform.
    pub(super) composition_layer: Bounds<Pixels>,
    pub(super) composition_radius: Pixels,
    pub(super) resize_handle: Bounds<Pixels>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CanvasHit {
    Recording,
    Resize,
}

pub(super) fn preview_geometry(
    stage: Bounds<Pixels>,
    canvas_view: CanvasView,
    composition: &CanvasComposition,
    video_aspect: f32,
    zoom_effect: Option<ZoomEffect>,
    cursor: Option<CursorFrame>,
) -> CanvasGeometry {
    let canvas = camera_bounds(
        fit_canvas(stage, composition.aspect_ratio.ratio()),
        canvas_view,
    );
    let recording_layer = recording_bounds(canvas, video_aspect, composition);
    let composition_layer = transform_layer(recording_layer, zoom_effect, cursor);
    let composition_radius = layer_radius(composition_layer, composition.corner_radius);
    let resize_handle = resize_handle(composition_layer);

    CanvasGeometry {
        canvas,
        recording_layer,
        composition_layer,
        composition_radius,
        resize_handle,
    }
}

pub(super) fn hit_test(geometry: CanvasGeometry, position: Point<Pixels>) -> Option<CanvasHit> {
    // The export canvas is the interaction boundary. A recording may be
    // scaled past that boundary so it can be cropped during a zoom, but
    // pixels outside the canvas are not part of the composition.
    if !geometry.canvas.contains(&position) {
        return None;
    }
    if geometry.resize_handle.contains(&position) {
        Some(CanvasHit::Resize)
    } else if geometry.composition_layer.contains(&position) {
        Some(CanvasHit::Recording)
    } else {
        None
    }
}

pub(super) fn cover_bounds(bounds: Bounds<Pixels>, aspect: f32) -> Bounds<Pixels> {
    let aspect = safe_aspect(aspect);
    let mut width = bounds.size.width.as_f32();
    let mut height = width / aspect;
    if height < bounds.size.height.as_f32() {
        height = bounds.size.height.as_f32();
        width = height * aspect;
    }
    centered_bounds(bounds.center(), width, height)
}

fn fit_canvas(stage: Bounds<Pixels>, aspect: f32) -> Bounds<Pixels> {
    let aspect = safe_aspect(aspect);
    let stage_width = stage.size.width.as_f32().max(0.0);
    let stage_height = stage.size.height.as_f32().max(0.0);
    let width = stage_width.min(stage_height * aspect);
    let height = width / aspect;
    centered_bounds(stage.center(), width, height)
}

fn recording_bounds(
    canvas: Bounds<Pixels>,
    video_aspect: f32,
    composition: &CanvasComposition,
) -> Bounds<Pixels> {
    let padding = composition.padding as f32;
    let available = Bounds::new(
        point(
            px(canvas.origin.x.as_f32() + canvas.size.width.as_f32() * padding),
            px(canvas.origin.y.as_f32() + canvas.size.height.as_f32() * padding),
        ),
        size(
            px(canvas.size.width.as_f32() * (1.0 - padding * 2.0)),
            px(canvas.size.height.as_f32() * (1.0 - padding * 2.0)),
        ),
    );
    let contained = contain_bounds(available, video_aspect);
    let scale = composition.scale as f32;
    let width = contained.size.width.as_f32() * scale;
    let height = contained.size.height.as_f32() * scale;
    let center = point(
        px(canvas.center().x.as_f32() + composition.position_x as f32 * canvas.size.width.as_f32()),
        px(canvas.center().y.as_f32()
            + composition.position_y as f32 * canvas.size.height.as_f32()),
    );
    centered_bounds(center, width, height)
}

fn contain_bounds(bounds: Bounds<Pixels>, aspect: f32) -> Bounds<Pixels> {
    let aspect = safe_aspect(aspect);
    let mut width = bounds.size.width.as_f32().max(0.0);
    let mut height = width / aspect;
    if height > bounds.size.height.as_f32() {
        height = bounds.size.height.as_f32().max(0.0);
        width = height * aspect;
    }
    centered_bounds(bounds.center(), width, height)
}

fn centered_bounds(center: Point<Pixels>, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(
        point(
            px(center.x.as_f32() - width / 2.0),
            px(center.y.as_f32() - height / 2.0),
        ),
        size(px(width.max(0.0)), px(height.max(0.0))),
    )
}

fn camera_bounds(bounds: Bounds<Pixels>, view: CanvasView) -> Bounds<Pixels> {
    let zoom = view.zoom as f32;
    let center = bounds.center();
    let width = bounds.size.width.as_f32() * zoom;
    let height = bounds.size.height.as_f32() * zoom;
    Bounds::new(
        point(
            px(center.x.as_f32()
                + (bounds.origin.x.as_f32() - center.x.as_f32()) * zoom
                + view.pan_x as f32),
            px(center.y.as_f32()
                + (bounds.origin.y.as_f32() - center.y.as_f32()) * zoom
                + view.pan_y as f32),
        ),
        size(px(width), px(height)),
    )
}

fn resize_handle(layer: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        point(
            px(layer.right().as_f32() - HANDLE_SIZE / 2.0),
            px(layer.bottom().as_f32() - HANDLE_SIZE / 2.0),
        ),
        size(px(HANDLE_SIZE), px(HANDLE_SIZE)),
    )
}

fn layer_radius(layer: Bounds<Pixels>, normalized_radius: f64) -> Pixels {
    let shortest = layer.size.width.as_f32().min(layer.size.height.as_f32());
    px((shortest * normalized_radius as f32)
        .min(shortest / 2.0)
        .max(0.0))
}

fn transform_layer(
    layer: Bounds<Pixels>,
    effect: Option<ZoomEffect>,
    cursor: Option<CursorFrame>,
) -> Bounds<Pixels> {
    let Some(effect) = effect else {
        return layer;
    };
    let scale = effect.scale.max(1.0);
    let (target_x, target_y) = match effect.target {
        ZoomTarget::Cursor => cursor
            .filter(|cursor| cursor.x.is_finite() && cursor.y.is_finite())
            .map(|cursor| (cursor.x.clamp(0.0, 1.0), cursor.y.clamp(0.0, 1.0)))
            .unwrap_or((0.5, 0.5)),
        ZoomTarget::CanvasCenter | ZoomTarget::Invalid => (0.5, 0.5),
    };
    let width = layer.size.width.as_f32() * scale;
    let height = layer.size.height.as_f32() * scale;
    let target = point(
        px(layer.origin.x.as_f32() + target_x * layer.size.width.as_f32()),
        px(layer.origin.y.as_f32() + target_y * layer.size.height.as_f32()),
    );
    Bounds::new(
        point(
            px(target.x.as_f32() - target_x * width),
            px(target.y.as_f32() - target_y * height),
        ),
        size(px(width), px(height)),
    )
}

fn safe_aspect(aspect: f32) -> f32 {
    if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        DEFAULT_VIDEO_ASPECT
    }
}

#[cfg(test)]
#[path = "editor_canvas_geometry_tests.rs"]
mod tests;
