use super::{
    CursorMotion, CursorPoint, MotionBlurDescriptor, MotionBlurMode, MotionBlurSettings,
    RecordingTransform, Vec2, compute_display_motion_blur, fps_scale,
};

/// Largest media-time step still treated as continuous playback. Anything
/// longer is a jump — a seek that reused the current generation, a stall, a
/// replay from the end — and its first frame is rendered sharp.
const MAX_FRAME_GAP_SECONDS: f64 = 0.25;

/// One frame that was actually presented to the preview.
///
/// Velocity is measured between these, never between decoded frames: the
/// playback pipeline drops, coalesces, and cancels decoded frames, and blurring
/// against one that never reached the screen would smear towards an image the
/// viewer never saw.
pub(crate) struct FrameSample {
    pub(crate) seconds: f64,
    pub(crate) seek_generation: u64,
    /// `None` whenever the cursor is hidden, missing, or out of bounds.
    pub(crate) cursor: Option<CursorPoint>,
    pub(crate) transform: Option<RecordingTransform>,
    pub(crate) zoom_center_uv: Vec2,
    pub(crate) settings: MotionBlurSettings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MotionBlurFrame {
    pub(crate) cursor: Option<CursorMotion>,
    pub(crate) display: MotionBlurDescriptor,
}

impl MotionBlurFrame {
    fn sharp(zoom_center_uv: Vec2) -> Self {
        Self {
            cursor: None,
            display: MotionBlurDescriptor {
                zoom_center_uv,
                ..MotionBlurDescriptor::inactive()
            },
        }
    }
}

/// Remembers the last presented frame so the next one can be measured against
/// it, and forgets it whenever continuity is broken.
#[derive(Default)]
pub(crate) struct MotionBlurHistory {
    seek_generation: u64,
    previous: Option<PresentedFrame>,
}

struct PresentedFrame {
    seconds: f64,
    cursor: Option<CursorPoint>,
    transform: Option<RecordingTransform>,
    mode: MotionBlurMode,
}

impl MotionBlurHistory {
    /// Drops the history so the next presented frame renders sharp. Callers use
    /// this for discontinuities that carry no seek generation of their own:
    /// opening a project, changing the preview rate, or replacing zoom state.
    pub(crate) fn reset(&mut self) {
        self.previous = None;
    }

    /// Records a presented frame and returns the blur it earns.
    pub(crate) fn presented(&mut self, sample: FrameSample) -> MotionBlurFrame {
        if sample.seek_generation != self.seek_generation {
            self.seek_generation = sample.seek_generation;
            self.previous = None;
        }

        let previous = self.previous.take();
        let settings = sample.settings.normalized();
        let frame = previous
            .as_ref()
            .filter(|_| !settings.is_disabled())
            .and_then(|previous| measure(previous, &sample, settings))
            .unwrap_or_else(|| MotionBlurFrame::sharp(sample.zoom_center_uv));

        self.previous = Some(PresentedFrame {
            seconds: sample.seconds,
            cursor: sample.cursor,
            transform: sample.transform,
            mode: frame.display.mode,
        });
        frame
    }
}

fn measure(
    previous: &PresentedFrame,
    sample: &FrameSample,
    settings: MotionBlurSettings,
) -> Option<MotionBlurFrame> {
    let delta_seconds = sample.seconds - previous.seconds;
    if !delta_seconds.is_finite() || delta_seconds <= 0.0 || delta_seconds > MAX_FRAME_GAP_SECONDS {
        return None;
    }
    let fps_scale = fps_scale(delta_seconds);

    // A cursor that appeared, disappeared, or left the surface has no velocity
    // to measure, so its first frame back is sharp.
    let cursor = previous
        .cursor
        .zip(sample.cursor)
        .map(|(previous, current)| CursorMotion {
            delta: current.delta_from(previous),
            strength: settings.cursor_strength(fps_scale),
        });

    let display = previous
        .transform
        .zip(sample.transform)
        .map(|(previous_transform, current)| {
            compute_display_motion_blur(
                previous_transform,
                current,
                previous.mode,
                sample.zoom_center_uv,
                settings.display_strength(fps_scale),
            )
        })
        .unwrap_or_else(|| MotionBlurDescriptor {
            zoom_center_uv: sample.zoom_center_uv,
            ..MotionBlurDescriptor::inactive()
        });

    Some(MotionBlurFrame { cursor, display })
}
