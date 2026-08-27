//! The native composition preview.
//!
//! When this is active the boundary between the two renderers is:
//!
//! ```text
//! GPUI                     toolbar, inspector, timeline, transport, chrome
//! native compositor        every pixel inside the preview rectangle
//! ```
//!
//! GPUI composes through DirectComposition and holds the *topmost* target for
//! its window, so it paints over anything we attach. That is the useful
//! direction — the editor keeps its selection outline and resize handle on top
//! of the video — but it also means the migration cannot be partial: any opaque
//! GPUI fill left over the preview hides the native surface completely, and the
//! surface has to draw the whole composition before that fill can be removed.
//!
//! **If the native preview looks blank while render and present are succeeding,
//! suspect this layering before the decoder, shader, or swapchain.** That
//! mistake cost a session once already.
//!
//! Three opaque layers stand between the window and the surface, and missing
//! any one of them hides it entirely:
//!
//! 1. The window. GPUI's Windows backend never reads
//!    `WindowOptions::window_background`, so the appearance must be set through
//!    `Window::set_background_appearance` before GPUI clears its swapchain to a
//!    transparent `[0.0; 4]` instead of an opaque `[1.0; 4]`.
//! 2. `gpui_component::Root`, which paints its own themed fill.
//! 3. The editor's own shell and preview backgrounds.
//!
//! Clearing the shell's fill has a consequence worth stating, because it is
//! easy to reintroduce: **every editor row outside the preview must paint its
//! own opaque background.** The shell cannot paint one for them — its fill
//! would cover the preview too — so a row that relied on the shell, or a
//! translucent element that relied on an opaque backdrop, becomes a hole
//! through to the desktop. The toolbar, inspector, transport, and timeline each
//! carry their own fill for this reason.
//!
//! The legacy GPUI preview stays selectable behind
//! [`available_backend`](super::super::rendering::available_backend) until the
//! native path has been validated through zoom, seeking, resizing, performance,
//! and motion blur.

use gpui::{Bounds, Hsla, Pixels, Rgba, Window, transparent_black};

use super::super::rendering::{Backend, available_backend};

pub(super) fn enabled() -> bool {
    available_backend() == Backend::Native
}

/// The editor workspace fill, as the compositor needs it.
///
/// The surface covers the whole preview rectangle, so the compositor paints the
/// area around the canvas as well. That area is editor chrome, which is why the
/// colour comes from the theme rather than the renderer.
fn surround(color: Hsla) -> [f32; 4] {
    let rgba = Rgba::from(color);
    [rgba.r, rgba.g, rgba.b, rgba.a]
}

#[cfg(target_os = "windows")]
mod platform {
    use gpui::{Bounds, Pixels, Window};
    use raw_window_handle::HasWindowHandle;

    use super::super::super::rendering::{
        CompositionState, FrameId, PreviewBounds, PreviewRenderer, PreviewSurface, window_handle,
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
        ) -> std::time::Duration {
            let started = std::time::Instant::now();
            let Some(surface) = self.surface.as_mut() else {
                return std::time::Duration::ZERO;
            };
            if let Some(bounds) = bounds
                && let Err(error) = surface.set_bounds(bounds)
            {
                tracing::warn!(target: "recorder::rendering", %error, "could not move the preview surface");
            }
            surface.follow(timestamp_us);
            let Some(composition) = composition else {
                return started.elapsed();
            };
            if let Err(error) =
                PreviewRenderer::render(surface, &FrameId::new(0, 0, timestamp_us), composition)
                    .and_then(|_| PreviewRenderer::present(surface))
            {
                tracing::warn!(target: "recorder::rendering", %error, "preview frame failed");
            }
            started.elapsed()
        }

        pub(super) fn attach(
            &mut self,
            window: &Window,
            stage: Bounds<Pixels>,
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
            let Some(bounds) = PreviewBounds::from_logical(
                stage.origin.x.as_f32(),
                stage.origin.y.as_f32(),
                stage.size.width.as_f32(),
                stage.size.height.as_f32(),
                window.scale_factor(),
            ) else {
                // A rectangle this early is a layout that has not settled yet,
                // not a failure. Retrying costs nothing until one is usable.
                return false;
            };

            self.attempted = true;
            match PreviewSurface::new(hwnd, bounds) {
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
                        "the native compositor owns the preview"
                    );
                    self.surface = Some(surface);
                    true
                }
                Err(error) => {
                    tracing::error!(
                        target: "recorder::rendering",
                        %error,
                        "no native preview surface; the editor stays on the GPUI preview"
                    );
                    false
                }
            }
        }

        /// Presented frames and frames decoded but never shown.
        pub(super) fn counters(&self) -> Option<(u64, u64)> {
            self.surface.as_ref().map(PreviewSurface::counters)
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use gpui::{Bounds, Pixels, Window};

    use super::super::super::rendering::{CompositionState, PreviewBounds};

    #[derive(Default)]
    pub(super) struct Attachment;

    impl Attachment {
        pub(super) fn attach(
            &mut self,
            _window: &Window,
            _stage: Bounds<Pixels>,
            _video_path: std::path::PathBuf,
        ) -> bool {
            false
        }

        pub(super) fn update(
            &mut self,
            _bounds: Option<PreviewBounds>,
            _timestamp_us: u64,
            _composition: Option<&CompositionState>,
        ) -> std::time::Duration {
            std::time::Duration::ZERO
        }

        pub(super) fn counters(&self) -> Option<(u64, u64)> {
            None
        }
    }
}

#[derive(Default)]
pub(super) struct NativePreview {
    attachment: platform::Attachment,
    counters: Counters,
    composing: bool,
}

impl NativePreview {
    /// True once this editor's native surface is attached and drawing the
    /// composition, which is when GPUI must stop painting the same layers.
    ///
    /// Per editor rather than global: a second window that failed to create a
    /// surface has to keep its own legacy preview, not inherit the first
    /// window's success and show nothing.
    pub(super) fn composing(&self) -> bool {
        self.composing
    }

    /// Presented frames per second, for the editor's readout. GPUI's own
    /// presented-frame metric counts image paints, which the native path
    /// deliberately no longer performs.
    pub(super) fn presented_fps(&self) -> f32 {
        self.counters.fps
    }

    /// A background that disappears once the native surface owns the
    /// composition, so nothing opaque is left covering it.
    pub(super) fn background(&self, color: Hsla) -> Hsla {
        if self.composing() {
            transparent_black()
        } else {
            color
        }
    }
}

/// The last reading, so each report is a rate over the interval rather than a
/// running total that has to be differenced by hand.
#[derive(Default)]
struct Counters {
    at: Option<std::time::Instant>,
    presented: u64,
    dropped: u64,
    /// Time spent composing and presenting, which is the compositor's share of
    /// the editor's frame budget.
    spent: std::time::Duration,
    /// Presented frames per second over the last sample, which is what the
    /// editor's readout shows while the compositor owns the preview.
    fps: f32,
    logged_at: Option<std::time::Instant>,
}

/// Composes one preview frame. Called from the editor's render pass, which is
/// what keeps the native surface in step with GPUI's own layout.
pub(super) fn compose(
    view: &mut super::PlaybackView,
    window: &Window,
    stage: Bounds<Pixels>,
    workspace: Hsla,
) {
    if !enabled() {
        return;
    }
    let video_path = view.video_path.clone();
    let pane = pane_bounds(stage, window);
    view.native_preview.composing = view
        .native_preview
        .attachment
        .attach(window, pane, video_path);

    let scale_factor = window.scale_factor();
    let bounds = super::super::rendering::PreviewBounds::from_logical(
        pane.origin.x.as_f32(),
        pane.origin.y.as_f32(),
        pane.size.width.as_f32(),
        pane.size.height.as_f32(),
        scale_factor,
    );
    let composition = bounds.and_then(|bounds| {
        let placement = view.canvas_placement(pane, surround(workspace), scale_factor)?;
        view.composition_state(bounds.size, placement)
    });
    let spent = view.native_preview.attachment.update(
        bounds,
        view.timeline.playhead_us,
        composition.as_ref(),
    );
    view.native_preview.counters.spent += spent;
    sample_counters(&mut view.native_preview);
}

/// The preview pane: the stage plus the inset around it.
///
/// The surface covers the pane rather than the stage because GPUI's fills
/// inside the preview are suppressed while the compositor owns it, and a strip
/// nothing paints would be transparent through to the desktop.
fn pane_bounds(stage: Bounds<Pixels>, window: &Window) -> Bounds<Pixels> {
    let inset = super::editor_preview::PREVIEW_PADDING.to_pixels(window.rem_size());
    Bounds::new(
        gpui::point(stage.origin.x - inset, stage.origin.y - inset),
        gpui::size(
            stage.size.width + inset * 2.0,
            stage.size.height + inset * 2.0,
        ),
    )
}

/// Reports the native path's own throughput periodically, so presented frames
/// per second and dropped decodes are measurable without a separate harness.
///
/// Presented frames are bounded by how often GPUI renders, because composition
/// happens in the editor's render pass. While the legacy pipeline still decodes
/// alongside this one, that is what paces both.
fn sample_counters(preview: &mut NativePreview) {
    /// Short enough that the editor's readout tracks playback instead of
    /// trailing it. At 60 FPS this still averages fifteen frames, which is
    /// steady without being sluggish.
    const SAMPLE: std::time::Duration = std::time::Duration::from_millis(250);
    /// Logging is coarser, because one line a second is noise.
    const REPORT: std::time::Duration = std::time::Duration::from_secs(5);

    let now = std::time::Instant::now();
    let counters = &mut preview.counters;
    if counters
        .at
        .is_some_and(|at| now.duration_since(at) < SAMPLE)
    {
        return;
    }
    let Some((presented, dropped)) = preview.attachment.counters() else {
        return;
    };
    let previous = counters.at.replace(now);
    let since = presented - counters.presented;
    counters.presented = presented;
    let Some(elapsed) = previous.map(|at| now.duration_since(at)) else {
        // The first sample has no interval to divide by; it only establishes
        // the baseline the next one measures against.
        counters.dropped = dropped;
        counters.logged_at = Some(now);
        counters.spent = std::time::Duration::ZERO;
        return;
    };
    let seconds = elapsed.as_secs_f64();
    counters.fps = if seconds > 0.0 {
        (since as f64 / seconds) as f32
    } else {
        0.0
    };

    if counters
        .logged_at
        .is_some_and(|at| now.duration_since(at) < REPORT)
    {
        return;
    }
    let reported = now.duration_since(counters.logged_at.unwrap_or(now));
    counters.logged_at = Some(now);
    let dropped_since = dropped - counters.dropped;
    counters.dropped = dropped;
    let spent = std::mem::take(&mut counters.spent);
    let frames = (reported.as_secs_f64() * f64::from(counters.fps)).round() as u64;
    tracing::info!(
        target: "recorder::rendering",
        fps = counters.fps,
        dropped = dropped_since,
        frame_ms = if frames > 0 { spent.as_secs_f64() * 1_000.0 / frames as f64 } else { 0.0 },
        seconds = reported.as_secs_f64(),
        "native preview"
    );
}
