use super::Vec2;

/// Upper bound on the rendered smear. A cursor teleport, a seek that slipped
/// past the history reset, or a corrupt telemetry sample must not be able to
/// stretch the sprite across the whole preview.
const MAX_MOTION_PX: f32 = 480.0;

/// Below roughly a pixel of travel there is nothing to smear, so the sharp
/// sprite is used and the whole blur path is skipped.
const MOTION_DEAD_ZONE_PX: f32 = 0.75;

/// Normalized coordinates address the captured surface, so anything outside
/// the unit square is a sample the cursor pipeline could not place. Treating
/// it as a discontinuity keeps a bad sample from inventing a huge velocity.
const BOUNDS_TOLERANCE: f32 = 0.001;

/// A cursor position that is valid to measure velocity against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CursorPoint {
    position: Vec2,
}

impl CursorPoint {
    pub(crate) fn new(x: f32, y: f32) -> Option<Self> {
        let position = Vec2::new(x, y);
        let in_bounds = |value: f32| (-BOUNDS_TOLERANCE..=1.0 + BOUNDS_TOLERANCE).contains(&value);
        (position.is_finite() && in_bounds(x) && in_bounds(y)).then_some(Self { position })
    }

    pub(crate) fn delta_from(self, previous: Self) -> Vec2 {
        self.position - previous.position
    }
}

/// Cursor velocity in recording-layer normalized coordinates.
///
/// The delta stays normalized here because the conversion to rendered pixels
/// depends on the composition layer's on-screen size, which is only known once
/// the preview geometry is resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CursorMotion {
    pub(crate) delta: Vec2,
    pub(crate) strength: f32,
}

impl CursorMotion {
    /// Converts the normalized delta into the rendered pixel motion produced by
    /// the current video/canvas transform, then applies the authored strength
    /// and the safety clamp.
    ///
    /// Returns `None` when the result is too small to be worth smearing, which
    /// is what keeps a still or barely-moving cursor perfectly sharp.
    pub(crate) fn to_sprite(
        self,
        layer_width: f32,
        layer_height: f32,
    ) -> Option<CursorSpriteMotion> {
        if !layer_width.is_finite() || !layer_height.is_finite() {
            return None;
        }
        let motion = Vec2::new(self.delta.x * layer_width, self.delta.y * layer_height)
            .scaled(self.strength)
            .clamped_length(MAX_MOTION_PX);
        (motion.is_finite() && motion.length() >= MOTION_DEAD_ZONE_PX)
            .then_some(CursorSpriteMotion { motion })
    }
}

/// The rendered smear handed to the cursor sprite builder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CursorSpriteMotion {
    /// Travel over the last presented frame, in rendered pixels, pointing from
    /// the previous cursor position towards the current one. The sprite trails
    /// backwards along it so the cursor head stays at the current position.
    motion: Vec2,
}

impl CursorSpriteMotion {
    pub(crate) fn motion(self) -> Vec2 {
        self.motion
    }
}
