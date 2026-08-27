//! The constant buffer every composition shader reads.
//!
//! This is where canvas-normalized composition values become the
//! render-target-normalized values the vertex shader turns into clip space, so
//! it is the whole coordinate conversion between the two spaces — and, being
//! plain arithmetic over plain data, it is checkable without a GPU.
//!
//! `misc` carries the layer's rounded-corner radius in target pixels plus the
//! target size, because the shaders derive a quad's own pixel size from
//! `destination.zw * misc.yz`. One layout serves the rounded canvas, the
//! rounded recording, and the gradient's aspect correction.

use crate::recorder::{
    composition::{self, NormalizedRect},
    cursor_settings::CursorStyle,
    motion_blur::{MotionBlurDescriptor, MotionBlurMode},
    project_settings::{CanvasBackground, CanvasBackgroundKind},
};

use super::super::super::CompositionState;
use super::resources::color;

const DEFAULT_BACKGROUND: [f32; 4] = [0.11, 0.13, 0.17, 1.0];
const DEFAULT_GRADIENT_END: [f32; 4] = [0.04, 0.05, 0.07, 1.0];

/// Which recording shader a frame needs. A still composition selects the sharp
/// pass, which has no sampling loop at all, so an inactive effect costs nothing
/// beyond this choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecordingPass {
    Sharp,
    Movement,
    Zoom,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Constants {
    pub(super) destination: [f32; 4],
    pub(super) source: [f32; 4],
    pub(super) color_start: [f32; 4],
    pub(super) color_end: [f32; 4],
    pub(super) misc: [f32; 4],
    /// Display motion blur. The directional shader reads `xy` as the movement
    /// vector; the radial shader reads `xy` as the zoom focus and `z` as the
    /// signed scale delta. Zero for every other draw.
    pub(super) motion: [f32; 4],
}

impl Constants {
    /// Defaults that draw `destination` with the whole source texture and no
    /// motion; the builders below override only what they need.
    fn for_rect(destination: NormalizedRect) -> Self {
        Self {
            destination: [
                destination.x as f32,
                destination.y as f32,
                destination.width as f32,
                destination.height as f32,
            ],
            source: [0.0, 0.0, 1.0, 1.0],
            color_start: [1.0; 4],
            color_end: [1.0; 4],
            misc: [0.0; 4],
            motion: [0.0; 4],
        }
    }

    /// The canvas fill: a gradient, or a solid drawn as a gradient with both
    /// stops equal so one rounded-rectangle path serves either choice.
    pub(super) fn canvas_fill(state: &CompositionState) -> Self {
        let background = &state.background;
        let solid = color(background.solid_color.as_ref(), DEFAULT_BACKGROUND);
        let (color_start, color_end) = match background.kind {
            CanvasBackgroundKind::Gradient => (
                color(background.gradient_start.as_ref(), DEFAULT_BACKGROUND),
                color(background.gradient_end.as_ref(), DEFAULT_GRADIENT_END),
            ),
            CanvasBackgroundKind::Solid | CanvasBackgroundKind::Image => (solid, solid),
        };
        Self {
            color_start,
            color_end,
            misc: canvas_misc(state, composition::CANVAS_GRADIENT_ANGLE_DEGREES),
            ..Self::for_rect(state.canvas.rect)
        }
    }

    /// A background image covering the canvas.
    ///
    /// The overflow lives in the source UVs rather than an oversized quad,
    /// because in the editor the canvas is a rectangle inside a larger surface
    /// and an oversized quad would spill across the workspace and past the
    /// canvas's rounded corners.
    pub(super) fn canvas_image(state: &CompositionState, width: u32, height: u32) -> Self {
        Self {
            source: cover_source(state.canvas.aspect(), width, height),
            misc: canvas_misc(state, 0.0),
            ..Self::for_rect(state.canvas.rect)
        }
    }

    /// The screen recording, transformed and rounded as the composition asks.
    pub(super) fn recording(state: &CompositionState) -> (Self, RecordingPass) {
        let frame = &state.frame;
        let canvas = state.canvas;
        // The radius is authored against the layer's shorter edge, so it has to
        // be resolved in canvas pixels before it can be expressed in target
        // pixels for the shader.
        let radius = (frame.recording.width * f64::from(canvas.size.width))
            .min(frame.recording.height * f64::from(canvas.size.height))
            * frame.corner_radius;
        let (pass, motion) = recording_pass(state.motion_blur);
        (
            Self {
                misc: target_misc(state, radius as f32),
                motion,
                ..Self::for_rect(canvas.place(frame.recording))
            },
            pass,
        )
    }

    /// The reconstructed cursor sprite, which the cursor shader draws
    /// procedurally rather than sampling.
    pub(super) fn cursor(state: &CompositionState) -> Option<Self> {
        let frame = &state.frame;
        let cursor = frame.cursor.filter(|cursor| cursor.visible)?;
        let rect = composition::cursor_rect(frame, state.source_size, cursor.style.asset())?;
        Some(Self {
            color_start: [0.0; 4],
            color_end: [0.0; 4],
            misc: [0.0, 0.0, 0.0, style_value(cursor.style)],
            ..Self::for_rect(state.canvas.place(rect))
        })
    }

    /// The image the canvas background needs loaded, if any.
    pub(super) fn image_path(background: &CanvasBackground) -> Option<&std::path::Path> {
        match background.kind {
            CanvasBackgroundKind::Image => background.image_path.as_deref(),
            CanvasBackgroundKind::Solid | CanvasBackgroundKind::Gradient => None,
        }
    }
}

/// `misc` for a quad rounded by the canvas radius. `angle` is read only by the
/// gradient shader and ignored elsewhere.
fn canvas_misc(state: &CompositionState, angle: f32) -> [f32; 4] {
    let [radius, width, height, _] = target_misc(state, state.canvas.corner_radius);
    [radius, width, height, angle]
}

fn target_misc(state: &CompositionState, radius: f32) -> [f32; 4] {
    let target = state.target_size;
    [radius, target.width as f32, target.height as f32, 0.0]
}

fn recording_pass(motion: MotionBlurDescriptor) -> (RecordingPass, [f32; 4]) {
    match motion.mode {
        MotionBlurMode::None => (RecordingPass::Sharp, [0.0; 4]),
        MotionBlurMode::Movement => (
            RecordingPass::Movement,
            [
                motion.movement_uv.x,
                motion.movement_uv.y,
                0.0,
                motion.strength,
            ],
        ),
        MotionBlurMode::Zoom => (
            RecordingPass::Zoom,
            [
                motion.zoom_center_uv.x,
                motion.zoom_center_uv.y,
                motion.zoom_amount,
                motion.strength,
            ],
        ),
    }
}

/// Source UVs that make an image cover a canvas of `canvas_aspect`.
fn cover_source(canvas_aspect: f64, image_width: u32, image_height: u32) -> [f32; 4] {
    let rect = composition::cover_rect(
        canvas_aspect,
        f64::from(image_width) / f64::from(image_height.max(1)),
    );
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return [0.0, 0.0, 1.0, 1.0];
    }
    let u = |value: f64, origin: f64, extent: f64| ((value - origin) / extent) as f32;
    [
        u(0.0, rect.x, rect.width),
        u(0.0, rect.y, rect.height),
        u(1.0, rect.x, rect.width),
        u(1.0, rect.y, rect.height),
    ]
}

fn style_value(style: CursorStyle) -> f32 {
    match style {
        CursorStyle::Default => 0.0,
        CursorStyle::Circle => 1.0,
    }
}

/// The canvas as a device-pixel clip rectangle, bounded by the target so a
/// canvas pushed off-screen by the editor camera cannot produce a negative or
/// out-of-range scissor.
pub(super) fn canvas_clip(state: &CompositionState) -> (i32, i32, i32, i32) {
    let target = state.target_size;
    let rect = state.canvas.rect;
    let edge = |value: f64, extent: u32| {
        (value * f64::from(extent))
            .round()
            .clamp(0.0, f64::from(extent)) as i32
    };
    let left = edge(rect.x, target.width);
    let top = edge(rect.y, target.height);
    (
        left,
        top,
        edge(rect.x + rect.width, target.width).max(left),
        edge(rect.y + rect.height, target.height).max(top),
    )
}

#[cfg(test)]
#[path = "constants_tests.rs"]
mod tests;
