//! Shared D3D11 drawing state for the preview.
//!
//! The shaders, constant-buffer layout, and shader compilation are the exporter's
//! — reused rather than reimplemented, so preview and export sample and
//! transform pixels through identical code. Only the render target differs: the
//! exporter draws into an offscreen texture, the preview into a swapchain back
//! buffer.
//!
//! The exporter still owns its own draw loop. Folding the two together is the
//! consolidation step once the preview composes a real frame; until then this
//! holds only the state a draw needs, not a second copy of the composition
//! logic.

use windows::Win32::Graphics::{
    Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
    Direct3D11::{
        D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_MAP_WRITE_DISCARD, D3D11_MAPPED_SUBRESOURCE,
        D3D11_SAMPLER_DESC, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_VIEWPORT, ID3D11Buffer,
        ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader, ID3D11RenderTargetView,
        ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11VertexShader,
    },
};

use super::super::super::super::export::{renderer::Constants, resources, shaders};
use super::super::super::{PhysicalSize, RenderError};

pub(super) struct Pipeline {
    context: ID3D11DeviceContext,
    constants: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    vertex_shader: ID3D11VertexShader,
    texture_shader: ID3D11PixelShader,
}

impl Pipeline {
    pub(super) fn new(
        device: &ID3D11Device,
        context: ID3D11DeviceContext,
    ) -> Result<Self, RenderError> {
        let constants = resources::create_constants(device)
            .map_err(|error| RenderError::Device(format!("no constant buffer: {error}")))?;
        let mut sampler = None;
        unsafe {
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    MinLOD: 0.0,
                    MaxLOD: f32::MAX,
                    ..Default::default()
                },
                Some(&mut sampler),
            )
        }
        .map_err(|error| RenderError::Device(format!("no sampler: {error}")))?;
        let sampler = sampler.ok_or_else(|| RenderError::Device("sampler was null".into()))?;

        let vertex_shader = resources::create_vertex_shader(device)
            .map_err(|error| RenderError::Device(format!("no vertex shader: {error}")))?;
        let texture_shader = resources::create_pixel_shader(device, &shaders::texture())
            .map_err(|error| RenderError::Device(format!("no texture shader: {error}")))?;

        Ok(Self {
            context,
            constants,
            sampler,
            vertex_shader,
            texture_shader,
        })
    }

    /// Points the pipeline at a render target and clears it.
    pub(super) fn begin(
        &self,
        target: &ID3D11RenderTargetView,
        size: PhysicalSize,
        clear: [f32; 4],
    ) {
        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: size.width as f32,
                Height: size.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            self.context.ClearRenderTargetView(target, &clear);
        }
    }

    /// Draws one textured quad using the exporter's texture shader.
    pub(super) fn draw_texture(
        &self,
        constants: Constants,
        view: &ID3D11ShaderResourceView,
    ) -> Result<(), RenderError> {
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(
                    &self.constants,
                    0,
                    D3D11_MAP_WRITE_DISCARD,
                    0,
                    Some(&mut mapped),
                )
                .map_err(|error| RenderError::Frame(format!("could not map constants: {error}")))?;
            std::ptr::copy_nonoverlapping(
                (&constants as *const Constants).cast::<u8>(),
                mapped.pData.cast::<u8>(),
                std::mem::size_of::<Constants>(),
            );
            self.context.Unmap(&self.constants, 0);

            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.texture_shader, None);
            self.context
                .VSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context
                .PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context
                .PSSetShaderResources(0, Some(&[Some(view.clone())]));
            self.context
                .IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            self.context.Draw(4, 0);
        }
        Ok(())
    }
}
