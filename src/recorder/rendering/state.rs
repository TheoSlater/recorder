use super::super::{
    composition::{CompositionFrame, NormalizedRect, SourceSize},
    motion_blur::MotionBlurDescriptor,
    project_settings::CanvasBackground,
};

/// A size in device pixels. Every value the renderer works in is physical: the
/// backend allocates real surfaces, so logical pixels must be converted before
/// they cross this boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PhysicalSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl PhysicalSize {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Converts a logical size at a given DPI scale, rounding to whole device
    /// pixels and never producing a zero dimension a swapchain would reject.
    pub(crate) fn from_logical(width: f32, height: f32, scale_factor: f32) -> Option<Self> {
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            return None;
        };
        let device = |value: f32| {
            let pixels = (value * scale).round();
            (pixels.is_finite() && pixels >= 1.0).then_some(pixels as u32)
        };
        Some(Self::new(device(width)?, device(height)?))
    }

    pub(crate) fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub(crate) fn aspect(self) -> f32 {
        if self.height == 0 {
            0.0
        } else {
            self.width as f32 / self.height as f32
        }
    }
}

/// The preview rectangle GPUI assigned, in device pixels relative to the window
/// client area.
///
/// This is layout, not composition: it says where on screen the preview appears,
/// never what the exported frame contains. The editor camera lives entirely on
/// the GPUI side of this boundary and is not represented here at all.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PreviewBounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) size: PhysicalSize,
}

impl PreviewBounds {
    /// Converts a logical rectangle from GPUI into device pixels.
    ///
    /// The edges are rounded before the size is derived so the surface always
    /// lands on whole pixels and cannot drift by a pixel as the rectangle moves.
    pub(crate) fn from_logical(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        scale_factor: f32,
    ) -> Option<Self> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return None;
        }
        let left = (x * scale_factor).round();
        let top = (y * scale_factor).round();
        let right = ((x + width) * scale_factor).round();
        let bottom = ((y + height) * scale_factor).round();
        if ![left, top, right, bottom]
            .iter()
            .all(|edge| edge.is_finite())
        {
            return None;
        }
        let width = (right - left).max(0.0) as u32;
        let height = (bottom - top).max(0.0) as u32;
        (width > 0 && height > 0).then_some(Self {
            x: left as i32,
            y: top as i32,
            size: PhysicalSize::new(width, height),
        })
    }
}

/// Where the composition canvas sits inside the render target, and how the
/// editor frames it.
///
/// This is the only place the editor camera reaches the renderer, and it
/// reaches it as layout: viewport zoom and pan move and scale *where* the
/// canvas appears on screen, never what the canvas contains. Every value in
/// [`CompositionState::frame`] stays camera-free, so the same frame drawn
/// through a different placement is the same picture in a different rectangle.
///
/// Export places the canvas over the whole target, which is why the exported
/// pixels cannot see the camera at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CanvasPlacement {
    /// The canvas, normalized to the render target.
    pub(crate) rect: NormalizedRect,
    /// The canvas in device pixels, which is the basis for the corner radius
    /// and for gradient direction.
    pub(crate) size: PhysicalSize,
    /// Canvas corner radius in device pixels. Editor presentation only; the
    /// exported frame is a plain rectangle.
    pub(crate) corner_radius: f32,
    /// The editor workspace fill around the canvas. The preview surface covers
    /// the whole preview rectangle, so the compositor paints this rather than
    /// leaving the window transparent where the canvas does not reach.
    pub(crate) surround: [f32; 4],
}

impl CanvasPlacement {
    /// The canvas over the whole target with no surround: what export renders.
    pub(crate) fn filling(size: PhysicalSize) -> Self {
        Self {
            rect: NormalizedRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            size,
            corner_radius: 0.0,
            surround: [0.0; 4],
        }
    }

    /// Maps a canvas-normalized rectangle into render-target-normalized
    /// coordinates.
    ///
    /// Everything the composition module produces — the recording layer, the
    /// cursor sprite, the background — is normalized to the canvas. The
    /// renderer draws into a target the canvas may only occupy part of, so this
    /// is the single conversion between the two spaces.
    pub(crate) fn place(&self, rect: NormalizedRect) -> NormalizedRect {
        NormalizedRect {
            x: self.rect.x + rect.x * self.rect.width,
            y: self.rect.y + rect.y * self.rect.height,
            width: rect.width * self.rect.width,
            height: rect.height * self.rect.height,
        }
    }

    pub(crate) fn aspect(&self) -> f64 {
        f64::from(self.size.aspect())
    }
}

/// Everything the renderer needs to draw one composed frame.
///
/// This is assembled from the values the editor and exporter already agree on,
/// rather than a second description of the same picture. New layers —
/// backgrounds beyond the current one, webcam, captions — are added by
/// extending [`CompositionFrame`], which both consumers already read, so the
/// renderer interface does not change when they arrive.
#[derive(Clone, Debug)]
pub(crate) struct CompositionState {
    /// Size of the render target in device pixels.
    pub(crate) target_size: PhysicalSize,
    /// Where the composition canvas sits inside that target.
    pub(crate) canvas: CanvasPlacement,
    /// Dimensions of the decoded recording, needed to place the cursor.
    pub(crate) source_size: SourceSize,
    /// Normalized, camera-free layout of this frame.
    pub(crate) frame: CompositionFrame,
    pub(crate) background: CanvasBackground,
    pub(crate) motion_blur: MotionBlurDescriptor,
}

impl CompositionState {
    pub(crate) fn new(
        target_size: PhysicalSize,
        canvas: CanvasPlacement,
        source_size: SourceSize,
        frame: CompositionFrame,
        background: CanvasBackground,
        motion_blur: MotionBlurDescriptor,
    ) -> Self {
        Self {
            target_size,
            canvas,
            source_size,
            frame,
            background,
            motion_blur,
        }
    }

    /// True when there is nothing to draw into, which a backend treats as a
    /// skipped frame rather than an error.
    pub(crate) fn is_empty(&self) -> bool {
        self.target_size.is_empty() || self.canvas.size.is_empty()
    }
}
