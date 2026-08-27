use super::{CompositionState, PhysicalSize, RenderError};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

/// A platform GPU compositor for the preview.
///
/// Composition and presentation are separate calls so the same implementation
/// can later target an encoder texture instead of a swapchain: `render` writes
/// the composed frame, `present` is the only step that involves the screen.
///
/// Nothing in this trait names a platform type. A backend keeps its device,
/// surface, and texture handles private, so the editor never links against
/// D3D, Metal, or Vulkan concepts.
pub(crate) trait PreviewRenderer {
    /// Resizes the composition target. Called when the preview rectangle, the
    /// window, or the DPI scale changes.
    fn resize(&mut self, size: PhysicalSize) -> Result<(), RenderError>;

    /// Composes one frame. `frame` identifies the decoded picture the backend
    /// should draw; the backend resolves it against whatever GPU resource its
    /// platform decoder produced.
    fn render(
        &mut self,
        frame: &super::FrameId,
        composition: &CompositionState,
    ) -> Result<(), RenderError>;

    /// Puts the most recently composed frame on screen.
    fn present(&mut self) -> Result<(), RenderError>;
}

/// Which preview implementation the editor should use.
///
/// The legacy GPUI path stays selectable while the native path is brought up,
/// so a platform without a backend — or a machine where device creation fails —
/// keeps a working preview instead of a blank rectangle. This is a migration
/// aid, not a second permanent compositor.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Backend {
    /// Frames are uploaded to GPUI as images and painted by the editor.
    #[default]
    LegacyGpui,
    /// Frames stay on the GPU and are composed by this subsystem.
    Native,
}

/// Opt-in for the native compositor while it is brought up.
const NATIVE_PREVIEW_VARIABLE: &str = "RECORDER_PREVIEW_SPIKE";

/// The backend the editor should use.
///
/// Windows can compose natively, but the legacy GPUI preview stays the default
/// until the native path has been validated through zoom, seeking, resizing,
/// performance, and motion blur on real hardware. Until then this is the single
/// switch between them: a platform with no backend, or a machine where device
/// creation fails, keeps a preview that draws rather than a blank rectangle.
pub(crate) fn available_backend() -> Backend {
    if cfg!(target_os = "windows")
        && std::env::var(NATIVE_PREVIEW_VARIABLE).is_ok_and(|value| value == "1")
    {
        Backend::Native
    } else {
        Backend::LegacyGpui
    }
}
