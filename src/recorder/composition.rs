//! Renderer-independent evaluation of the exported composition.
//!
//! Values in this module are normalized to the output canvas.  The editor
//! turns them into GPUI pixels after applying its camera; export uses them
//! directly in output pixels.  Keeping the camera out of this module is the
//! important part of the preview/export contract.

use super::{
    cursor::CursorFrame,
    cursor_settings::{CursorAsset, CursorStyle},
    motion_blur::{RecordingTransform, Vec2},
    project_settings::{AspectRatioPreset, CanvasComposition, ProjectSettings},
    zoom::{ZoomEffect, ZoomTarget, effect_at},
};

const DEFAULT_OUTPUT_LONG_EDGE: u32 = 1920;
const DEFAULT_VIDEO_ASPECT: f64 = 16.0 / 9.0;
pub(crate) const CANVAS_GRADIENT_ANGLE_DEGREES: f32 = 135.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl SourceSize {
    pub(crate) fn valid(self) -> bool {
        self.width > 0 && self.height > 0
    }

    fn aspect(self) -> f64 {
        if self.valid() {
            f64::from(self.width) / f64::from(self.height)
        } else {
            DEFAULT_VIDEO_ASPECT
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutputSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl OutputSize {
    /// Keeps the source's long edge and changes only the composition aspect.
    /// Even dimensions are required by the common H.264 encoders.
    pub(crate) fn for_source(source: SourceSize, preset: AspectRatioPreset) -> Self {
        let long_edge = if source.valid() {
            source.width.max(source.height)
        } else {
            DEFAULT_OUTPUT_LONG_EDGE
        };
        let aspect = f64::from(preset.ratio());
        if aspect >= 1.0 {
            Self {
                width: even_dimension(long_edge),
                height: even_dimension((f64::from(long_edge) / aspect).round() as u32),
            }
        } else {
            Self {
                width: even_dimension((f64::from(long_edge) * aspect).round() as u32),
                height: even_dimension(long_edge),
            }
        }
    }

    pub(crate) fn aspect(self) -> f64 {
        f64::from(self.width) / f64::from(self.height)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NormalizedRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl NormalizedRect {
    fn centered(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x: x - width / 2.0,
            y: y - height / 2.0,
            width: width.max(0.0),
            height: height.max(0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CursorPlacement {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) scale: f32,
    pub(crate) style: CursorStyle,
    pub(crate) visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CompositionFrame {
    pub(crate) output: OutputSize,
    pub(crate) base_recording: NormalizedRect,
    pub(crate) recording: NormalizedRect,
    pub(crate) zoom: Option<ZoomEffect>,
    pub(crate) zoom_focus: (f64, f64),
    pub(crate) cursor: Option<CursorPlacement>,
    pub(crate) corner_radius: f64,
    pub(crate) shadow: bool,
}

impl CompositionFrame {
    /// The recording layer as motion blur measures it. These values are
    /// normalized to the output canvas and the editor camera is never read
    /// here, so viewport pan and zoom cannot register as composition movement.
    pub(crate) fn recording_transform(&self) -> Option<RecordingTransform> {
        RecordingTransform::new(
            Vec2::new(
                (self.recording.x + self.recording.width / 2.0) as f32,
                (self.recording.y + self.recording.height / 2.0) as f32,
            ),
            Vec2::new(self.recording.width as f32, self.recording.height as f32),
        )
    }

    /// Where the active zoom is pulling towards, in recording-layer UV. This is
    /// the focal point a radial smear has to be centred on; it follows the
    /// cursor for cursor-targeted zooms and the layer centre otherwise.
    pub(crate) fn zoom_center(&self) -> Vec2 {
        Vec2::new(self.zoom_focus.0 as f32, self.zoom_focus.1 as f32)
    }
}

/// Evaluates all time-dependent composition values for an output timestamp.
/// `CanvasView` is intentionally not read here.
pub(crate) fn evaluate(
    settings: &ProjectSettings,
    source: SourceSize,
    timestamp_us: u64,
    cursor: Option<CursorFrame>,
) -> CompositionFrame {
    let composition = &settings.canvas_composition;
    let output = OutputSize::for_source(source, composition.aspect_ratio);
    evaluate_with_aspect(
        composition,
        source,
        output,
        effect_at(&settings.zoom_regions, timestamp_us),
        cursor,
    )
}

pub(crate) fn evaluate_with_aspect(
    composition: &CanvasComposition,
    source: SourceSize,
    output: OutputSize,
    zoom: Option<ZoomEffect>,
    cursor: Option<CursorFrame>,
) -> CompositionFrame {
    let base_recording = recording_rect(composition, source.aspect(), output.aspect());
    let zoom_focus = zoom_focus(zoom, cursor);
    let recording = transform(base_recording, zoom, zoom_focus);
    let cursor = cursor
        .filter(|cursor| cursor.visible)
        .map(|cursor| CursorPlacement {
            x: recording.x + f64::from(cursor.x.clamp(0.0, 1.0)) * recording.width,
            y: recording.y + f64::from(cursor.y.clamp(0.0, 1.0)) * recording.height,
            scale: cursor.scale,
            style: cursor.asset.style(),
            visible: true,
        });

    CompositionFrame {
        output,
        base_recording,
        recording,
        zoom,
        zoom_focus,
        cursor,
        corner_radius: composition.corner_radius,
        shadow: composition.shadow,
    }
}

/// Computes the cursor sprite rectangle in output-canvas coordinates.
pub(crate) fn cursor_rect(
    frame: &CompositionFrame,
    source: SourceSize,
    asset: CursorAsset,
) -> Option<NormalizedRect> {
    let cursor = frame.cursor.filter(|cursor| cursor.visible)?;
    if !source.valid() || !cursor.scale.is_finite() || cursor.scale <= 0.0 {
        return None;
    }
    let scale = f64::from(cursor.scale) * frame.recording.width / f64::from(source.width);
    Some(NormalizedRect {
        x: cursor.x - f64::from(asset.hotspot_x()) * scale,
        y: cursor.y - f64::from(asset.hotspot_y()) * scale * frame.output.aspect(),
        width: f64::from(asset.width()) * scale,
        height: f64::from(asset.height()) * scale * frame.output.aspect(),
    })
}

/// Returns a centered rectangle that covers a canvas of `container_aspect`
/// with content of `content_aspect`. Both aspects are width divided by height;
/// the returned coordinates are normalized to the canvas.
pub(crate) fn cover_rect(container_aspect: f64, content_aspect: f64) -> NormalizedRect {
    let container_aspect = valid_aspect(container_aspect);
    let content_aspect = valid_aspect(content_aspect);
    if content_aspect >= container_aspect {
        NormalizedRect {
            x: 0.5 - content_aspect / container_aspect / 2.0,
            y: 0.0,
            width: content_aspect / container_aspect,
            height: 1.0,
        }
    } else {
        NormalizedRect {
            x: 0.0,
            y: 0.5 - container_aspect / content_aspect / 2.0,
            width: 1.0,
            height: container_aspect / content_aspect,
        }
    }
}

fn recording_rect(
    composition: &CanvasComposition,
    source_aspect: f64,
    output_aspect: f64,
) -> NormalizedRect {
    let padding = composition.padding.clamp(0.0, 0.45);
    let available_width = 1.0 - padding * 2.0;
    let available_height = available_width;
    let available_pixel_width = available_width * output_aspect;
    let available_pixel_height = available_height;
    let (width, height) = if available_pixel_width / available_pixel_height > source_aspect {
        (
            available_pixel_height * source_aspect / output_aspect,
            available_height,
        )
    } else {
        // `available_pixel_width` already includes the output aspect. Divide by
        // the source aspect once to convert that pixel width into normalized
        // output height; dividing by `output_aspect` again stretches narrow
        // compositions.
        (available_width, available_pixel_width / source_aspect)
    };
    NormalizedRect::centered(
        0.5 + composition.position_x.clamp(-1.0, 1.0),
        0.5 + composition.position_y.clamp(-1.0, 1.0),
        width * composition.scale,
        height * composition.scale,
    )
}

fn zoom_focus(zoom: Option<ZoomEffect>, cursor: Option<CursorFrame>) -> (f64, f64) {
    match zoom.map(|zoom| zoom.target) {
        Some(ZoomTarget::Cursor) => cursor
            .filter(|cursor| cursor.x.is_finite() && cursor.y.is_finite())
            .map(|cursor| {
                (
                    f64::from(cursor.x.clamp(0.0, 1.0)),
                    f64::from(cursor.y.clamp(0.0, 1.0)),
                )
            })
            .unwrap_or((0.5, 0.5)),
        Some(ZoomTarget::CanvasCenter | ZoomTarget::Invalid) | None => (0.5, 0.5),
    }
}

fn transform(rect: NormalizedRect, zoom: Option<ZoomEffect>, focus: (f64, f64)) -> NormalizedRect {
    let Some(zoom) = zoom else {
        return rect;
    };
    let scale = f64::from(zoom.scale.max(1.0));
    let target = (
        rect.x + focus.0 * rect.width,
        rect.y + focus.1 * rect.height,
    );
    let width = rect.width * scale;
    let height = rect.height * scale;
    NormalizedRect {
        x: target.0 - focus.0 * width,
        y: target.1 - focus.1 * height,
        width,
        height,
    }
}

fn even_dimension(value: u32) -> u32 {
    value.max(2) & !1
}

fn valid_aspect(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        DEFAULT_VIDEO_ASPECT
    }
}

#[cfg(test)]
#[path = "composition_tests.rs"]
mod tests;
