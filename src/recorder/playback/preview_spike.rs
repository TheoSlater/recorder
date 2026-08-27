//! Runtime probe for the native preview surface.
//!
//! This exists to answer one question that cannot be settled by reading GPUI's
//! source: does Windows honour a second, non-topmost DirectComposition target
//! on a window that GPUI created with `WS_EX_NOREDIRECTIONBITMAP` and already
//! holds the topmost target for?
//!
//! Enable it with `RECORDER_PREVIEW_SPIKE=1`. When enabled the editor leaves the
//! preview area unpainted and attaches a composition surface cleared to
//! magenta, so the outcome is visible rather than inferred:
//!
//! - magenta fills the preview rectangle: the surface composes correctly
//!   underneath GPUI, and the preview can move onto it while GPUI keeps
//!   painting overlays on top.
//! - the target is refused: the log names the error and the child-HWND route is
//!   the alternative.
//! - the rectangle is empty and no error is logged: the target was accepted but
//!   is not composited, which is the same answer as a refusal.
//!
//! The whole module is deleted once the question is answered.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{Bounds, Hsla, Pixels, Window, transparent_black};

/// Set while a probe surface is attached, so the canvas leaves its area
/// unpainted. A flag rather than a plumbed parameter because this is a
/// diagnostic with a short life, not a rendering mode.
static HOLE_PUNCHED: AtomicBool = AtomicBool::new(false);

pub(super) fn enabled() -> bool {
    std::env::var("RECORDER_PREVIEW_SPIKE").is_ok_and(|value| value == "1")
}

/// True when the editor should leave the preview area transparent so an
/// underlying composition surface can show through.
pub(super) fn hole_punched() -> bool {
    HOLE_PUNCHED.load(Ordering::Relaxed)
}

/// A background that disappears while the probe is attached.
///
/// Every opaque fill stacked over the preview has to go, not just the canvas
/// stage: GPUI paints the shell and the preview container behind it, and either
/// one alone is enough to hide whatever composes underneath.
pub(super) fn background(color: Hsla) -> Hsla {
    if hole_punched() {
        transparent_black()
    } else {
        color
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use gpui::{Bounds, Pixels, Window};
    use raw_window_handle::HasWindowHandle;

    use super::super::super::rendering::{
        CompositionState, FrameId, PreviewBounds, PreviewRenderer, PreviewSurface, probe,
        window_handle,
    };

    #[derive(Default)]
    pub(super) struct Attachment {
        surface: Option<PreviewSurface>,
        attempted: bool,
    }

    impl Attachment {
        /// Follows the playhead and the preview rectangle once attached.
        pub(super) fn update(
            &mut self,
            bounds: Option<PreviewBounds>,
            timestamp_us: u64,
            composition: Option<&CompositionState>,
        ) {
            let Some(surface) = self.surface.as_mut() else {
                return;
            };
            if let Some(bounds) = bounds
                && let Err(error) = surface.set_bounds(bounds)
            {
                tracing::warn!(target: "recorder::rendering", %error, "could not move the preview surface");
            }
            surface.follow(timestamp_us);
            let Some(composition) = composition else {
                return;
            };
            if let Err(error) =
                PreviewRenderer::render(surface, &FrameId::new(0, 0, timestamp_us), composition)
                    .and_then(|_| PreviewRenderer::present(surface))
            {
                tracing::warn!(target: "recorder::rendering", %error, "preview frame failed");
            }
        }

        pub(super) fn attach(
            &mut self,
            window: &Window,
            stage: Bounds<Pixels>,
            composition: Option<CompositionState>,
            video_path: std::path::PathBuf,
        ) -> bool {
            if self.attempted {
                return self.surface.is_some();
            }

            // Disambiguated: GPUI's own `Window::window_handle` returns its
            // internal handle type, not the raw platform one.
            let Some(hwnd) = HasWindowHandle::window_handle(window)
                .ok()
                .and_then(window_handle)
            else {
                tracing::error!(target: "recorder::rendering", "the playback window exposed no Win32 handle");
                return false;
            };
            // The decoder reports its dimensions after the first paints, so a
            // frame that arrives before them is too early to attach on rather
            // than a failure. Retrying costs nothing until one succeeds.
            let Some(composition) = composition else {
                return false;
            };
            let Some(bounds) = PreviewBounds::from_logical(
                stage.origin.x.as_f32(),
                stage.origin.y.as_f32(),
                stage.size.width.as_f32(),
                stage.size.height.as_f32(),
                window.scale_factor(),
            ) else {
                tracing::error!(target: "recorder::rendering", "the preview rectangle is not usable yet");
                return false;
            };

            self.attempted = true;
            match probe(hwnd, bounds, &composition, video_path.clone()) {
                Ok(mut surface) => {
                    if let Err(error) = surface.stream(video_path) {
                        tracing::error!(target: "recorder::rendering", %error, "preview streaming unavailable");
                    }
                    tracing::info!(
                        target: "recorder::rendering",
                        x = bounds.x,
                        y = bounds.y,
                        width = bounds.size.width,
                        height = bounds.size.height,
                        scale_factor = window.scale_factor(),
                        "attached a non-topmost composition surface; the preview area should be magenta"
                    );
                    self.surface = Some(surface);
                    true
                }
                Err(error) => {
                    tracing::error!(
                        target: "recorder::rendering",
                        %error,
                        "the non-topmost composition target is unusable; the child-HWND route is required"
                    );
                    false
                }
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use gpui::{Bounds, Pixels, Window};

    #[derive(Default)]
    pub(super) struct Attachment;

    impl Attachment {
        pub(super) fn attach(
            &mut self,
            _window: &Window,
            _stage: Bounds<Pixels>,
            _composition: Option<super::super::super::rendering::CompositionState>,
            _video_path: std::path::PathBuf,
        ) -> bool {
            false
        }

        pub(super) fn update(
            &mut self,
            _bounds: Option<super::super::super::rendering::PreviewBounds>,
            _timestamp_us: u64,
            _composition: Option<&super::super::super::rendering::CompositionState>,
        ) {
        }
    }
}

#[derive(Default)]
pub(super) struct PreviewSpike {
    attachment: platform::Attachment,
}

/// Attaches on the first paint that has a real preview rectangle.
pub(super) fn attach(view: &mut super::PlaybackView, window: &Window, stage: Bounds<Pixels>) {
    if !enabled() {
        return;
    }
    let composition = super::super::rendering::PhysicalSize::from_logical(
        stage.size.width.as_f32(),
        stage.size.height.as_f32(),
        window.scale_factor(),
    )
    .and_then(|size| view.composition_state(size));
    let video_path = view.video_path.clone();
    if view
        .preview_spike
        .attachment
        .attach(window, stage, composition.clone(), video_path)
    {
        HOLE_PUNCHED.store(true, Ordering::Relaxed);
    }

    let bounds = super::super::rendering::PreviewBounds::from_logical(
        stage.origin.x.as_f32(),
        stage.origin.y.as_f32(),
        stage.size.width.as_f32(),
        stage.size.height.as_f32(),
        window.scale_factor(),
    );
    view.preview_spike
        .attachment
        .update(bounds, view.timeline.playhead_us, composition.as_ref());
}
