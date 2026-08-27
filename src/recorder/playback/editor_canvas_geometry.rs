use gpui::*;

use super::super::{
    composition::{self, CompositionFrame, NormalizedRect, OutputSize, SourceSize},
    cursor::CursorFrame,
    motion_blur::{RecordingTransform, Vec2},
    project_settings::{CanvasComposition, CanvasView},
    zoom::ZoomEffect,
};

const DEFAULT_VIDEO_ASPECT: f32 = 16. / 9.;
const HANDLE_SIZE: f32 = 12.;
const VIEWPORT_ZOOM_TOLERANCE: f64 = 0.001;
const VIEWPORT_PAN_TOLERANCE: f64 = 0.5;

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
    pub(super) composition_frame: CompositionFrame,
    /// The recording layer in output-normalized units, which is what motion
    /// blur measures so editor navigation cannot register as movement.
    pub(super) recording_transform: Option<RecordingTransform>,
    /// Focal point the active zoom is pulling towards, in layer UV space.
    pub(super) zoom_focus: Vec2,
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
    video_width: u32,
    video_height: u32,
    zoom_effect: Option<ZoomEffect>,
    cursor: Option<CursorFrame>,
) -> CanvasGeometry {
    let canvas = camera_bounds(
        fit_canvas(stage, composition.aspect_ratio.ratio()),
        canvas_view,
    );
    let source = SourceSize {
        width: video_width,
        height: video_height,
    };
    let output = OutputSize {
        width: (canvas.size.width.as_f32().max(1.0) * 1_000.0).round() as u32,
        height: (canvas.size.height.as_f32().max(1.0) * 1_000.0).round() as u32,
    };
    let composition_frame =
        composition::evaluate_with_aspect(composition, source, output, zoom_effect, cursor);
    let recording_layer = normalized_bounds(canvas, composition_frame.base_recording);
    let composition_layer = normalized_bounds(canvas, composition_frame.recording);
    let composition_radius = layer_radius(composition_layer, composition.corner_radius);
    let resize_handle = resize_handle(composition_layer);

    CanvasGeometry {
        canvas,
        recording_layer,
        composition_layer,
        composition_radius,
        resize_handle,
        composition_frame,
        recording_transform: composition_frame.recording_transform(),
        zoom_focus: composition_frame.zoom_center(),
    }
}

/// Returns whether the editor camera needs a recenter affordance.
///
/// A fitted canvas reaches at least one stage edge, so zooming above 1x clips
/// it. Panning moves its center even when the whole canvas remains visible.
pub(super) fn needs_recenter(view: CanvasView) -> bool {
    view.zoom > 1.0 + VIEWPORT_ZOOM_TOLERANCE
        || view.pan_x.abs() > VIEWPORT_PAN_TOLERANCE
        || view.pan_y.abs() > VIEWPORT_PAN_TOLERANCE
}

/// Places an output-normalized rect onto the on-screen canvas, which is where
/// the editor camera is applied.
fn normalized_bounds(canvas: Bounds<Pixels>, rect: NormalizedRect) -> Bounds<Pixels> {
    Bounds::new(
        point(
            px(canvas.origin.x.as_f32() + rect.x as f32 * canvas.size.width.as_f32()),
            px(canvas.origin.y.as_f32() + rect.y as f32 * canvas.size.height.as_f32()),
        ),
        size(
            px(rect.width as f32 * canvas.size.width.as_f32()),
            px(rect.height as f32 * canvas.size.height.as_f32()),
        ),
    )
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
    let container_aspect =
        f64::from(bounds.size.width.as_f32()) / f64::from(bounds.size.height.as_f32().max(1.0));
    let rect = composition::cover_rect(container_aspect, f64::from(aspect));
    normalized_bounds(bounds, rect)
}

fn fit_canvas(stage: Bounds<Pixels>, aspect: f32) -> Bounds<Pixels> {
    let aspect = safe_aspect(aspect);
    let stage_width = stage.size.width.as_f32().max(0.0);
    let stage_height = stage.size.height.as_f32().max(0.0);
    let width = stage_width.min(stage_height * aspect);
    let height = width / aspect;
    centered_bounds(stage.center(), width, height)
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
