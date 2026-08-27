use thiserror::Error;

/// Failures a preview backend can report.
///
/// Variants are deliberately platform-neutral: a backend converts its own
/// device errors into these so the editor never has to match on a D3D, Metal,
/// or Vulkan result. A platform with no backend at all is not an error here —
/// [`available_backend`](super::available_backend) simply keeps the editor on
/// the legacy preview.
#[derive(Debug, Error)]
pub(crate) enum RenderError {
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
