//! Linux preview backend.
//!
//! Not implemented. This module exists so the platform boundary is visible in
//! the tree and so the shared interface is written against more than one
//! target.
//!
//! The intended shape when it is built:
//!
//! ```text
//! VA-API / vaapi-backed decode
//!         -> DMA-BUF import
//!         -> wgpu or Vulkan
//!         -> our compositor
//!         -> native surface hosted in the GPUI window
//! ```
//!
//! Using wgpu here is a choice inside our own compositor. It is explicitly not
//! an attempt to add a render pass to GPUI's renderer: the boundary stays at
//! the native surface, and GPUI remains untouched.
//!
//! Responsibilities it will own:
//!
//! - Creating the GPU device and a surface for the window handle GPUI exposes,
//!   under both X11 and Wayland.
//! - Keeping that surface positioned and scaled with the preview rectangle.
//! - Importing decoded frames without a CPU round trip where the decoder
//!   allows it.
//! - Implementing [`PreviewRenderer`] against the shared [`CompositionState`].
//!
//! No wgpu or Vulkan dependency is declared yet, for the same reason as macOS.
//!
//! [`PreviewRenderer`]: super::PreviewRenderer
//! [`CompositionState`]: super::CompositionState
