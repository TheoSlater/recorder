//! Preview compositor.
//!
//! This subsystem owns GPU composition of the video preview. It is deliberately
//! independent of GPUI: GPUI keeps the editor shell, layout, and input, and
//! hands this module a rectangle plus a description of what the frame should
//! look like. Nothing here may depend on GPUI's renderer internals, and nothing
//! platform-specific may reach this module's public surface.
//!
//! The layering is:
//!
//! ```text
//! editor state  ->  CompositionState  ->  PreviewRenderer  ->  platform backend
//! ```
//!
//! [`CompositionState`] is deliberately assembled from the types the editor and
//! the exporter already share ([`CompositionFrame`], [`MotionBlurDescriptor`]),
//! so preview and export describe a frame the same way rather than growing two
//! composition models.
//!
//! [`CompositionFrame`]: super::composition::CompositionFrame
//! [`MotionBlurDescriptor`]: super::motion_blur::MotionBlurDescriptor

mod backend;
mod error;
mod frame;
mod state;

#[cfg(target_os = "windows")]
pub(crate) use backend::windows::{
    CompositionRenderer, DirectCompositionSurface as PreviewSurface, create_device, window_handle,
};
pub(crate) use backend::{Backend, PreviewRenderer, available_backend};
pub(crate) use error::RenderError;
pub(crate) use frame::{FrameId, FrameQueue};
pub(crate) use state::{CanvasPlacement, CompositionState, PhysicalSize, PreviewBounds};

#[cfg(test)]
#[path = "rendering/tests.rs"]
mod tests;
