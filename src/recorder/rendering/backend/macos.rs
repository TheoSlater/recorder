//! macOS preview backend.
//!
//! Not implemented. This module exists so the platform boundary is visible in
//! the tree and so the shared interface is written against more than one
//! target.
//!
//! The intended shape when it is built:
//!
//! ```text
//! AVFoundation / VideoToolbox
//!         -> CVPixelBuffer / IOSurface
//!         -> Metal
//!         -> our compositor
//!         -> CAMetalLayer hosted in the GPUI window
//! ```
//!
//! Responsibilities it will own:
//!
//! - Creating and owning the Metal device, command queue, and layer.
//! - Attaching that layer to the GPUI window's native view, keeping its bounds
//!   and `contentsScale` in step with the preview rectangle.
//! - Importing decoded frames as Metal textures from `IOSurface` without a CPU
//!   round trip.
//! - Implementing [`PreviewRenderer`] against the shared [`CompositionState`].
//!
//! Nothing above is written yet, and no Metal dependency is declared, because a
//! stub that compiles against an unused SDK is a liability rather than
//! preparation.
//!
//! [`PreviewRenderer`]: super::PreviewRenderer
//! [`CompositionState`]: super::CompositionState
