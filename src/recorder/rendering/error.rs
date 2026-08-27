use thiserror::Error;

/// Failures a preview backend can report.
///
/// Variants are deliberately platform-neutral: a backend converts its own
/// device errors into these so the editor never has to match on a D3D, Metal,
/// or Vulkan result.
#[derive(Debug, Error)]
pub(crate) enum RenderError {
    /// The platform has no preview backend compiled in. The caller falls back
    /// to the legacy GPUI preview rather than failing the editor.
    #[error("native preview is not supported on this platform yet")]
    Unsupported,

    /// The native surface could not be created, resized, or attached.
    #[error("preview surface failed: {0}")]
    Surface(String),

    /// The GPU device was lost or could not be created.
    #[error("preview device failed: {0}")]
    Device(String),

    /// A frame could not be composed or presented.
    #[error("preview frame failed: {0}")]
    Frame(String),
}
