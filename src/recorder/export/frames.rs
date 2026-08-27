//! Export's view of the shared composition renderer.
//!
//! Export and the editor preview draw the same picture through the same code;
//! they differ only in where it lands. This wraps the shared renderer with the
//! two things export needs and the preview does not: a fresh output texture per
//! encoded sample, and a canvas that fills that texture.

use anyhow::Result;

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::super::{
    composition::{CompositionFrame, OutputSize, SourceSize},
    motion_blur::MotionBlurDescriptor,
    project_settings::CanvasComposition,
    rendering::{CanvasPlacement, CompositionRenderer, CompositionState, PhysicalSize},
};
use super::decoder::DeviceContext;

pub(crate) struct Renderer {
    renderer: CompositionRenderer,
    /// Rebuilt in place per frame. Only the composition frame and the motion
    /// descriptor change between output timestamps, so the background and the
    /// canvas placement are resolved once.
    state: CompositionState,
}

impl Renderer {
    pub(crate) fn new(
        device: &DeviceContext,
        output: OutputSize,
        source: SourceSize,
        composition: &CanvasComposition,
        frame: CompositionFrame,
    ) -> Result<Self> {
        let size = PhysicalSize::new(output.width, output.height);
        Ok(Self {
            renderer: CompositionRenderer::new(&device.device, &device.context)?,
            state: CompositionState::new(
                size,
                CanvasPlacement::filling(size),
                source,
                frame,
                composition.background.clone(),
                MotionBlurDescriptor::inactive(),
            ),
        })
    }

    /// Composes one output frame and returns the texture to encode.
    ///
    /// A surface per encoded sample is deliberate: Sink Writer may retain a
    /// DXGI sample after `WriteSample` returns, so reusing one render target
    /// would let a later frame overwrite pixels still being encoded.
    pub(crate) fn render(
        &mut self,
        source_texture: &ID3D11Texture2D,
        frame: CompositionFrame,
        motion: MotionBlurDescriptor,
    ) -> Result<ID3D11Texture2D> {
        self.state.frame = frame;
        self.state.motion_blur = motion;
        let output = self.renderer.create_target(self.state.target_size)?;
        let target = self.renderer.target_view(&output)?;
        let recording = self.renderer.shader_view(source_texture)?;
        self.renderer.draw(&target, &self.state, Some(&recording))?;
        Ok(output)
    }
}
