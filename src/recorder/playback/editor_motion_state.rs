//! Per-frame motion state for the editor preview.
//!
//! This owns everything the blur needs to remember between frames: the
//! presented-frame history, how the recording layer was classified for the
//! frame on screen, and the smeared cursor sprite built for it. Keeping it here
//! rather than in [`PlaybackView`] means no GPUI event handler has to carry
//! motion bookkeeping of its own.
//!
//! [`PlaybackView`]: super::PlaybackView

use std::sync::Arc;

use gpui::RenderImage;

use super::super::{
    composition::SourceSize,
    cursor::CursorFrame,
    motion_blur::{
        CursorMotion, CursorPoint, FrameSample, MotionBlurDescriptor, MotionBlurHistory,
        MotionBlurSettings, Vec2,
    },
};
use super::{editor_canvas_cursor, editor_canvas_cursor_blur, editor_canvas_geometry};

/// Everything about one frame that reached the preview.
pub(super) struct PresentedFrame<'a> {
    pub(super) seconds: f64,
    pub(super) seek_generation: u64,
    pub(super) cursor: Option<CursorFrame>,
    pub(super) geometry: Option<editor_canvas_geometry::CanvasGeometry>,
    pub(super) video_width: u32,
    pub(super) video_height: u32,
    pub(super) cursor_images: &'a [Arc<RenderImage>; 2],
    pub(super) settings: MotionBlurSettings,
}

#[derive(Default)]
pub(super) struct MotionBlurState {
    history: MotionBlurHistory,
    display: MotionBlurDescriptor,
    sprite: Option<editor_canvas_cursor_blur::BlurredCursor>,
    scale_factor: f32,
}

impl MotionBlurState {
    /// The smeared sprite for the frame on screen, or `None` when the cursor
    /// should be drawn sharp.
    pub(super) fn sprite(&self) -> Option<editor_canvas_cursor_blur::BlurredCursor> {
        self.sprite.clone()
    }

    pub(super) fn display(&self) -> MotionBlurDescriptor {
        self.display
    }

    /// The sprite is generated in device pixels, so it needs whatever scale the
    /// playback window is presenting at.
    pub(super) fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = scale_factor;
    }

    /// Measures motion against the last presented frame and prepares this
    /// frame's smear, returning any sprite whose atlas entry the caller should
    /// release.
    ///
    /// Only frames that reach the preview get here. Decoded frames that were
    /// dropped, coalesced, or cancelled never enter the history, so velocity
    /// always describes what the viewer actually saw.
    pub(super) fn present(&mut self, frame: PresentedFrame<'_>) -> Option<Arc<RenderImage>> {
        let visible_cursor = frame.cursor.filter(|cursor| cursor.visible);
        let measured = self.history.presented(FrameSample {
            seconds: frame.seconds,
            seek_generation: frame.seek_generation,
            cursor: visible_cursor.and_then(|cursor| CursorPoint::new(cursor.x, cursor.y)),
            transform: frame
                .geometry
                .and_then(|geometry| geometry.recording_transform),
            zoom_center_uv: frame
                .geometry
                .map(|geometry| geometry.zoom_focus)
                .unwrap_or(Vec2::new(0.5, 0.5)),
            settings: frame.settings,
        });
        self.display = measured.display;

        let sprite = measured
            .cursor
            .zip(visible_cursor)
            .and_then(|(motion, cursor)| self.build_sprite(motion, cursor, &frame));
        self.replace_sprite(sprite)
    }

    /// Forgets the history so the next presented frame renders sharp.
    pub(super) fn reset(&mut self) -> Option<Arc<RenderImage>> {
        self.history.reset();
        self.display = MotionBlurDescriptor::inactive();
        self.replace_sprite(None)
    }

    fn build_sprite(
        &self,
        motion: CursorMotion,
        cursor: CursorFrame,
        frame: &PresentedFrame<'_>,
    ) -> Option<editor_canvas_cursor_blur::BlurredCursor> {
        let geometry = frame.geometry?;
        let layer = geometry.composition_layer;
        let motion = motion.to_sprite(layer.size.width.as_f32(), layer.size.height.as_f32())?;
        let bounds = editor_canvas_cursor::cursor_bounds(
            geometry.canvas,
            geometry.composition_frame,
            SourceSize {
                width: frame.video_width,
                height: frame.video_height,
            },
            cursor.asset,
        )?;
        let base = frame
            .cursor_images
            .get(cursor.asset.style().index())
            .or_else(|| frame.cursor_images.first())?;
        editor_canvas_cursor_blur::build(base, bounds.size, motion.motion(), self.scale_factor)
    }

    fn replace_sprite(
        &mut self,
        sprite: Option<editor_canvas_cursor_blur::BlurredCursor>,
    ) -> Option<Arc<RenderImage>> {
        std::mem::replace(&mut self.sprite, sprite).map(|previous| previous.image)
    }
}
