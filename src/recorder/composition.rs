//! Renderer-independent evaluation of the exported composition.
//!
//! Values in this module are normalized to the output canvas.  The editor
//! turns them into GPUI pixels after applying its camera; export uses them
//! directly in output pixels.  Keeping the camera out of this module is the
//! important part of the preview/export contract.

use super::{
    cursor::CursorFrame,
    cursor_settings::{CursorAsset, CursorStyle},
    project_settings::{AspectRatioPreset, CanvasComposition, ProjectSettings},
    zoom::{ZoomEffect, ZoomTarget, effect_at},
};

const DEFAULT_OUTPUT_LONG_EDGE: u32 = 1920;
const DEFAULT_VIDEO_ASPECT: f64 = 16.0 / 9.0;

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
        (
            available_width,
            available_pixel_width / source_aspect / output_aspect,
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::project_settings::ProjectSettings;

    fn source() -> SourceSize {
        SourceSize {
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn output_dimensions_follow_aspect_and_long_edge() {
        assert_eq!(
            OutputSize::for_source(source(), AspectRatioPreset::Widescreen),
            OutputSize {
                width: 1920,
                height: 1080
            }
        );
        assert_eq!(
            OutputSize::for_source(source(), AspectRatioPreset::Square),
            OutputSize {
                width: 1920,
                height: 1920
            }
        );
        assert_eq!(
            OutputSize::for_source(source(), AspectRatioPreset::Vertical),
            OutputSize {
                width: 1080,
                height: 1920
            }
        );
    }

    #[test]
    fn viewport_values_do_not_change_export_frame() {
        let first = ProjectSettings::default();
        let mut second = first.clone();
        second.canvas.zoom = 4.0;
        second.canvas.pan_x = 80.0;
        second.canvas.pan_y = -40.0;
        let frame_a = evaluate_with_aspect(
            &first.canvas_composition,
            source(),
            OutputSize::for_source(source(), first.canvas_composition.aspect_ratio),
            None,
            None,
        );
        let frame_b = evaluate_with_aspect(
            &second.canvas_composition,
            source(),
            OutputSize::for_source(source(), second.canvas_composition.aspect_ratio),
            None,
            None,
        );
        assert_eq!(frame_a, frame_b);
    }

    #[test]
    fn zoom_changes_only_the_recording_transform() {
        let composition = CanvasComposition::default();
        let output = OutputSize::for_source(source(), composition.aspect_ratio);
        let plain = evaluate_with_aspect(&composition, source(), output, None, None);
        let zoom = evaluate_with_aspect(
            &composition,
            source(),
            output,
            Some(ZoomEffect {
                scale: 2.0,
                target: ZoomTarget::CanvasCenter,
            }),
            None,
        );
        assert_eq!(plain.base_recording, zoom.base_recording);
        assert!(zoom.recording.width > plain.recording.width);
    }

    #[test]
    fn normalized_settings_evaluate_deterministically() {
        let settings = ProjectSettings::default().normalized();
        let frame_a = evaluate(&settings, source(), 123_456, None);
        let frame_b = evaluate(&settings, source(), 123_456, None);
        assert_eq!(frame_a, frame_b);
    }
}
