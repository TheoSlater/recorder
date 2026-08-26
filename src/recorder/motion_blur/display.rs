use super::{MotionBlurDescriptor, MotionBlurMode, Vec2};

/// Ignore sub-pixel drift so a still composition stays perfectly sharp.
/// Expressed as a fraction of the canvas, matching [`RecordingTransform`].
const MOVEMENT_DEAD_ZONE: f32 = 0.0005;

/// Ignore scale noise from easing curves settling onto their final value.
const ZOOM_DEAD_ZONE: f32 = 0.0008;

/// A scale ratio change of `d` displaces the layer's edge by about `d / 2` of
/// its own extent. Converting zoom into that equivalent displacement is what
/// makes "translation versus scale" a comparison between like quantities.
const ZOOM_MOTION_EQUIVALENT: f32 = 0.5;

/// How far one kind of motion must exceed the other before the mode switches.
/// Below this ratio the previous mode is held, so a transform that translates
/// and scales at once cannot oscillate between filters every frame.
const MODE_DOMINANCE: f32 = 1.25;

/// Caps on the shader-facing extents. Without them a discontinuous transform —
/// a region boundary, a regenerated zoom, a settings change — would smear the
/// whole frame for one frame.
const MAX_MOVEMENT_UV: f32 = 0.15;
const MAX_ZOOM_AMOUNT: f32 = 0.10;

/// The recording layer's placement inside the export canvas.
///
/// Both fields are normalized against the canvas, which is what makes this
/// value camera-independent: the editor's viewport zoom and pan scale the
/// canvas and the layer by the same factor, so their ratio is unchanged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RecordingTransform {
    /// Layer centre in canvas units, where `(0.5, 0.5)` is the canvas centre.
    pub(crate) center: Vec2,
    /// Layer size as a fraction of the canvas.
    pub(crate) size: Vec2,
}

impl RecordingTransform {
    pub(crate) fn new(center: Vec2, size: Vec2) -> Option<Self> {
        let transform = Self { center, size };
        transform.is_usable().then_some(transform)
    }

    fn is_usable(&self) -> bool {
        self.center.is_finite() && self.size.is_finite() && self.size.x > 0.0 && self.size.y > 0.0
    }

    /// Mean scale ratio against an earlier transform. Zoom is uniform in
    /// practice; averaging the axes keeps a degenerate aspect from skewing it.
    fn scale_ratio_from(&self, previous: &Self) -> f32 {
        (self.size.x / previous.size.x + self.size.y / previous.size.y) / 2.0
    }
}

/// Classifies one inter-frame transform change as movement, zoom, or neither.
///
/// `previous_mode` is the mode chosen for the last presented frame and only
/// breaks ties, so a transform that both translates and scales keeps a stable
/// filter instead of alternating.
pub(crate) fn compute_display_motion_blur(
    previous: RecordingTransform,
    current: RecordingTransform,
    previous_mode: MotionBlurMode,
    zoom_center_uv: Vec2,
    strength: f32,
) -> MotionBlurDescriptor {
    let inactive = MotionBlurDescriptor {
        zoom_center_uv,
        ..MotionBlurDescriptor::inactive()
    };
    if !strength.is_finite() || strength <= 0.0 {
        return inactive;
    }

    let displacement = current.center - previous.center;
    let scale_ratio = current.scale_ratio_from(&previous);
    if !displacement.is_finite() || !scale_ratio.is_finite() {
        return inactive;
    }
    let zoom_delta = scale_ratio - 1.0;

    let movement_extent = displacement.length();
    let zoom_extent = zoom_delta.abs() * ZOOM_MOTION_EQUIVALENT;
    if movement_extent < MOVEMENT_DEAD_ZONE && zoom_delta.abs() < ZOOM_DEAD_ZONE {
        return inactive;
    }

    match dominant_mode(movement_extent, zoom_extent, previous_mode) {
        MotionBlurMode::Movement => {
            // Canvas-space displacement becomes recording-layer UV by dividing
            // through the layer's own extent: moving a small layer by a tenth
            // of the canvas smears far more of its content than moving a
            // full-bleed one by the same distance.
            let movement_uv = Vec2::new(
                displacement.x / current.size.x,
                displacement.y / current.size.y,
            )
            .scaled(strength)
            .clamped_length(MAX_MOVEMENT_UV);
            MotionBlurDescriptor {
                mode: MotionBlurMode::Movement,
                movement_uv,
                zoom_center_uv,
                zoom_amount: 0.0,
                strength,
            }
        }
        MotionBlurMode::Zoom => MotionBlurDescriptor {
            mode: MotionBlurMode::Zoom,
            movement_uv: Vec2::ZERO,
            zoom_center_uv,
            zoom_amount: (zoom_delta * strength).clamp(-MAX_ZOOM_AMOUNT, MAX_ZOOM_AMOUNT),
            strength,
        },
        MotionBlurMode::None => inactive,
    }
}

fn dominant_mode(movement: f32, zoom: f32, previous: MotionBlurMode) -> MotionBlurMode {
    if movement > zoom * MODE_DOMINANCE {
        MotionBlurMode::Movement
    } else if zoom > movement * MODE_DOMINANCE {
        MotionBlurMode::Zoom
    } else if previous == MotionBlurMode::None {
        // No prior mode to hold: fall back to whichever is larger.
        if movement >= zoom {
            MotionBlurMode::Movement
        } else {
            MotionBlurMode::Zoom
        }
    } else {
        previous
    }
}
