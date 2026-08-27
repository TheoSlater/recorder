use anyhow::{Context, Result, anyhow};
use windows::Win32::Graphics::{
    Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
    Direct3D11::{
        D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_MAP_WRITE_DISCARD, D3D11_MAPPED_SUBRESOURCE,
        D3D11_SAMPLER_DESC, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_VIEWPORT, ID3D11Buffer,
        ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
        ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
    },
};

use super::super::{
    composition::{self, CompositionFrame, NormalizedRect, SourceSize},
    motion_blur::{MotionBlurDescriptor, MotionBlurMode},
    project_settings::{CanvasBackgroundKind, CanvasComposition},
};
use super::decoder::DeviceContext;
use super::{resources, shaders};

const DEFAULT_BACKGROUND: [f32; 4] = [0.11, 0.13, 0.17, 1.0];
const DEFAULT_GRADIENT_END: [f32; 4] = [0.04, 0.05, 0.07, 1.0];

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Constants {
    destination: [f32; 4],
    source: [f32; 4],
    color_start: [f32; 4],
    color_end: [f32; 4],
    misc: [f32; 4],
    /// Display motion blur. The directional shader reads `xy` as the movement
    /// vector; the radial shader reads `xy` as the zoom focus and `z` as the
    /// signed scale delta. Zero for every other draw.
    motion: [f32; 4],
}

impl Constants {
    /// Defaults that draw `destination` with the whole source texture and no
    /// motion; call sites override only what they need.
    pub(crate) fn for_rect(destination: NormalizedRect) -> Self {
        Self {
            destination: [
                destination.x as f32,
                destination.y as f32,
                destination.width as f32,
                destination.height as f32,
            ],
            source: [0.0, 0.0, 1.0, 1.0],
            color_start: [1.0; 4],
            color_end: [1.0; 4],
            misc: [0.0; 4],
            motion: [0.0; 4],
        }
    }
}

pub(super) struct ImageTexture {
    pub(super) _texture: ID3D11Texture2D,
    pub(super) view: ID3D11ShaderResourceView,
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(crate) struct Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    constants: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    vertex_shader: ID3D11VertexShader,
    texture_shader: ID3D11PixelShader,
    movement_blur_shader: ID3D11PixelShader,
    zoom_blur_shader: ID3D11PixelShader,
    gradient_shader: ID3D11PixelShader,
    cursor_shader: ID3D11PixelShader,
    background: Option<ImageTexture>,
    width: u32,
    height: u32,
}

impl Renderer {
    pub(crate) fn new(
        device: &DeviceContext,
        width: u32,
        height: u32,
        composition: &CanvasComposition,
    ) -> Result<Self> {
        let constants = resources::create_constants(&device.device)?;
        let mut sampler = None;
        unsafe {
            device.device.CreateSamplerState(
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
        .context("could not create export sampler")?;
        let sampler = sampler.ok_or_else(|| anyhow!("export sampler was null"))?;
        let vertex_shader = resources::create_vertex_shader(&device.device)?;
        let texture_shader = resources::create_pixel_shader(&device.device, &shaders::texture())?;
        let movement_blur_shader =
            resources::create_pixel_shader(&device.device, &shaders::movement_blur())?;
        let zoom_blur_shader =
            resources::create_pixel_shader(&device.device, &shaders::zoom_blur())?;
        let gradient_shader = resources::create_pixel_shader(&device.device, &shaders::gradient())?;
        let cursor_shader = resources::create_pixel_shader(&device.device, &shaders::cursor())?;
        let background = match composition.background.kind {
            CanvasBackgroundKind::Image => composition
                .background
                .image_path
                .as_deref()
                .map(|path| resources::load_image(&device.device, path))
                .transpose()?,
            CanvasBackgroundKind::Solid | CanvasBackgroundKind::Gradient => None,
        };
        Ok(Self {
            device: device.device.clone(),
            context: device.context.clone(),
            constants,
            sampler,
            vertex_shader,
            texture_shader,
            movement_blur_shader,
            zoom_blur_shader,
            gradient_shader,
            cursor_shader,
            background,
            width,
            height,
        })
    }

    pub(crate) fn render(
        &self,
        source_texture: &ID3D11Texture2D,
        frame: &CompositionFrame,
        source: SourceSize,
        composition: &CanvasComposition,
        motion: MotionBlurDescriptor,
    ) -> Result<ID3D11Texture2D> {
        // Keep a surface per encoded sample. Sink Writer may retain a DXGI
        // sample after WriteSample returns; reusing one render target would let
        // a later frame overwrite pixels that are still being encoded.
        let output = resources::create_output(&self.device, self.width, self.height)?;
        let mut output_view = None;
        unsafe {
            self.device
                .CreateRenderTargetView(&output, None, Some(&mut output_view))
        }
        .context("could not create export render target")?;
        let output_view = output_view.ok_or_else(|| anyhow!("export render target was null"))?;
        unsafe {
            self.context.OMSetRenderTargets(
                Some(&[Some(output_view.clone())]),
                None::<&ID3D11DepthStencilView>,
            );
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.width as f32,
                Height: self.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
        }
        let solid = resources::color(
            composition.background.solid_color.as_ref(),
            DEFAULT_BACKGROUND,
        );
        let start = resources::color(
            composition.background.gradient_start.as_ref(),
            DEFAULT_BACKGROUND,
        );
        let end = resources::color(
            composition.background.gradient_end.as_ref(),
            DEFAULT_GRADIENT_END,
        );
        unsafe { self.context.ClearRenderTargetView(&output_view, &solid) };
        match composition.background.kind {
            CanvasBackgroundKind::Solid => {}
            CanvasBackgroundKind::Gradient => self.draw(
                Constants {
                    color_start: start,
                    color_end: end,
                    misc: [
                        0.0,
                        self.width as f32,
                        self.height as f32,
                        composition::CANVAS_GRADIENT_ANGLE_DEGREES,
                    ],
                    ..Constants::for_rect(resources::full_rect())
                },
                None,
                &self.gradient_shader,
            )?,
            CanvasBackgroundKind::Image => {
                if let Some(background) = &self.background {
                    let rect = resources::cover_rect(
                        self.width,
                        self.height,
                        background.width,
                        background.height,
                    );
                    self.draw(
                        Constants {
                            color_start: start,
                            color_end: end,
                            ..Constants::for_rect(rect)
                        },
                        Some(&background.view),
                        &self.texture_shader,
                    )?;
                }
            }
        }
        let mut source_view = None;
        unsafe {
            self.device
                .CreateShaderResourceView(source_texture, None, Some(&mut source_view))
        }
        .context("could not create decoded frame shader view")?;
        let source_view =
            source_view.ok_or_else(|| anyhow!("decoded frame shader view was null"))?;
        let radius = (frame.recording.width * self.width as f64)
            .min(frame.recording.height * self.height as f64)
            * frame.corner_radius;
        let (recording_shader, motion) = self.recording_pass(motion);
        self.draw(
            Constants {
                misc: [radius as f32, self.width as f32, self.height as f32, 0.0],
                motion,
                ..Constants::for_rect(frame.recording)
            },
            Some(&source_view),
            recording_shader,
        )?;
        if let Some(cursor) = frame.cursor.filter(|cursor| cursor.visible)
            && let Some(rect) = composition::cursor_rect(frame, source, cursor.style.asset())
        {
            self.draw(
                Constants {
                    color_start: [0.0; 4],
                    color_end: [0.0; 4],
                    misc: [0.0, 0.0, 0.0, resources::style_value(cursor.style)],
                    ..Constants::for_rect(rect)
                },
                None,
                &self.cursor_shader,
            )?;
        }
        unsafe { self.context.Flush() };
        Ok(output)
    }

    /// Chooses the recording shader and the motion values it reads. A still
    /// composition selects the sharp shader, which has no sampling loop at all,
    /// so an inactive effect costs nothing beyond this match.
    fn recording_pass(&self, motion: MotionBlurDescriptor) -> (&ID3D11PixelShader, [f32; 4]) {
        match motion.mode {
            MotionBlurMode::None => (&self.texture_shader, [0.0; 4]),
            MotionBlurMode::Movement => (
                &self.movement_blur_shader,
                [
                    motion.movement_uv.x,
                    motion.movement_uv.y,
                    0.0,
                    motion.strength,
                ],
            ),
            MotionBlurMode::Zoom => (
                &self.zoom_blur_shader,
                [
                    motion.zoom_center_uv.x,
                    motion.zoom_center_uv.y,
                    motion.zoom_amount,
                    motion.strength,
                ],
            ),
        }
    }

    fn draw(
        &self,
        constants: Constants,
        view: Option<&ID3D11ShaderResourceView>,
        pixel_shader: &ID3D11PixelShader,
    ) -> Result<()> {
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
                .context("could not map export constants")?;
            std::ptr::copy_nonoverlapping(
                (&constants as *const Constants).cast::<u8>(),
                mapped.pData.cast::<u8>(),
                std::mem::size_of::<Constants>(),
            );
            self.context.Unmap(&self.constants, 0);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(pixel_shader, None);
            self.context
                .VSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context
                .PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            let resources = view.map(|view| [Some(view.clone())]);
            self.context
                .PSSetShaderResources(0, resources.as_ref().map(|resources| &resources[..]));
            self.context
                .IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            self.context.Draw(4, 0);
        }
        Ok(())
    }
}
