//! Velocity-based motion blur for the editor preview.
//!
//! Motion blur belongs to two spaces only: the composition/recording layer and
//! the reconstructed cursor overlay. The editor camera ([`CanvasView`]) is
//! deliberately excluded — panning or zooming the workspace changes how large
//! the composition appears on screen, never how fast its contents move. Every
//! transform this module consumes is therefore expressed in canvas-normalized
//! units, which cancels the camera exactly.
//!
//! [`CanvasView`]: super::project_settings::CanvasView

use serde::{Deserialize, Serialize};

mod cursor;
mod display;
mod history;

pub(crate) use cursor::{CursorMotion, CursorPoint};
pub(crate) use display::{RecordingTransform, compute_display_motion_blur};
pub(crate) use history::{FrameSample, MotionBlurHistory};

/// Preview baseline. A 60 FPS preview blurs at the authored strength; a slower
/// preview covers more media time per frame, so its longer per-frame motion is
/// scaled back to keep the smear a comparable length on screen.
const BASELINE_FPS: f64 = 60.0;

/// Bounds on the preview-rate correction. A frame delta far from the baseline
/// is usually a dropped or coalesced preview frame rather than a real rate
/// change, and must not be allowed to invent or erase a smear.
const MIN_FPS_SCALE: f32 = 0.25;
const MAX_FPS_SCALE: f32 = 2.0;

/// Per-effect gains. These let cursor and display blur be tuned separately
/// without exposing two controls: a whole moving frame reads as heavier motion
/// than a small sprite, so the display gain is the more conservative one.
const CURSOR_MULTIPLIER: f32 = 1.0;
const DISPLAY_MULTIPLIER: f32 = 0.75;

/// A strength of 1.0 smears across the entire inter-frame distance, which is a
/// 360° shutter. Film sits near 0.5, so the authored maximum stays expressive
/// without turning fast motion into a streak.
const CURSOR_MAX_STRENGTH: f32 = 1.0;
const DISPLAY_MAX_STRENGTH: f32 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Vec2 {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Vec2 {
    pub(crate) const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub(crate) fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub(crate) fn length(self) -> f32 {
        self.x.hypot(self.y)
    }

    pub(crate) fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    pub(crate) fn scaled(self, factor: f32) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }

    /// Shortens the vector to `limit` while preserving its direction.
    pub(crate) fn clamped_length(self, limit: f32) -> Self {
        let length = self.length();
        if length <= limit || length <= 0.0 {
            return self;
        }
        self.scaled(limit / length)
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MotionBlurMode {
    #[default]
    None,
    Movement,
    Zoom,
}

/// The result of classifying one inter-frame recording-layer transform change.
///
/// `movement_uv` and `zoom_amount` are final smear extents: the authored
/// strength is already applied and the result is capped, so a consumer samples
/// along them directly.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MotionBlurDescriptor {
    pub(crate) mode: MotionBlurMode,
    /// Inter-frame movement of the recording layer, in its own UV space.
    pub(crate) movement_uv: Vec2,
    /// Focal point of the zoom in recording-layer UV space.
    pub(crate) zoom_center_uv: Vec2,
    /// Signed radial extent: positive zooms in, negative zooms out.
    pub(crate) zoom_amount: f32,
    /// Effective gain that produced the extents above.
    pub(crate) strength: f32,
}

impl MotionBlurDescriptor {
    pub(crate) fn inactive() -> Self {
        Self {
            mode: MotionBlurMode::None,
            movement_uv: Vec2::ZERO,
            zoom_center_uv: Vec2::new(0.5, 0.5),
            zoom_amount: 0.0,
            strength: 0.0,
        }
    }
}

/// The single authored control. Cursor and display blur derive their own
/// strengths from this one amount.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct MotionBlurSettings {
    #[serde(default = "default_amount")]
    pub(crate) amount: f32,
}

impl Default for MotionBlurSettings {
    fn default() -> Self {
        Self {
            amount: default_amount(),
        }
    }
}

impl MotionBlurSettings {
    pub(crate) fn normalized(mut self) -> Self {
        self.amount = if self.amount.is_finite() {
            self.amount.clamp(0.0, 1.0)
        } else {
            default_amount()
        };
        self
    }

    /// True when the effect is fully bypassed, including its sampling cost.
    pub(crate) fn is_disabled(self) -> bool {
        self.amount <= 0.0
    }

    pub(crate) fn cursor_strength(self, fps_scale: f32) -> f32 {
        (self.amount * CURSOR_MULTIPLIER * fps_scale).clamp(0.0, CURSOR_MAX_STRENGTH)
    }

    pub(crate) fn display_strength(self, fps_scale: f32) -> f32 {
        (self.amount * DISPLAY_MULTIPLIER * fps_scale).clamp(0.0, DISPLAY_MAX_STRENGTH)
    }
}

/// Corrects for the preview rate so a smear looks the same at 24, 30, and
/// 60 FPS. A 30 FPS preview advances twice as much media time per frame, so
/// its doubled motion vector is halved back to the 60 FPS baseline.
pub(crate) fn fps_scale(frame_delta_seconds: f64) -> f32 {
    if !frame_delta_seconds.is_finite() || frame_delta_seconds <= 0.0 {
        return 1.0;
    }
    ((1.0 / (frame_delta_seconds * BASELINE_FPS)) as f32).clamp(MIN_FPS_SCALE, MAX_FPS_SCALE)
}

fn default_amount() -> f32 {
    0.35
}

#[cfg(test)]
#[path = "motion_blur/tests.rs"]
mod tests;
