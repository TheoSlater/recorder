//! The D3D11 composition draw.
//!
//! One implementation serves both consumers. The editor preview points it at a
//! swapchain back buffer and the exporter at an offscreen texture; everything
//! between — background, recording, motion blur, cursor — is the same code
//! reading the same [`CompositionState`], so a preview frame and an exported
//! frame cannot drift apart.
//!
//! The two differ only in the canvas placement: export fills the target with
//! the canvas, while the preview places it inside the editor's preview
//! rectangle and paints the editor workspace around it.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::{
    Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
    Direct3D11::{
        D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD,
        D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_CULL_NONE, D3D11_FILL_SOLID,
        D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_MAP_WRITE_DISCARD, D3D11_MAPPED_SUBRESOURCE,
        D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC, D3D11_SAMPLER_DESC,
        D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_VIEWPORT, ID3D11BlendState, ID3D11Buffer,
        ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
        ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11SamplerState,
        ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
    },
};

use super::super::super::{CompositionState, PhysicalSize, RenderError};
use super::background::Background;
use super::constants::{Constants, RecordingPass, canvas_clip};
use super::{resources, shaders};

type Result<T> = std::result::Result<T, RenderError>;

pub(crate) struct CompositionRenderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    constants: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    clip: ID3D11RasterizerState,
    blend: ID3D11BlendState,
    vertex_shader: ID3D11VertexShader,
    texture_shader: ID3D11PixelShader,
    movement_blur_shader: ID3D11PixelShader,
    zoom_blur_shader: ID3D11PixelShader,
    gradient_shader: ID3D11PixelShader,
    cursor_shader: ID3D11PixelShader,
    background: Background,
}

impl CompositionRenderer {
    pub(crate) fn new(device: &ID3D11Device, context: &ID3D11DeviceContext) -> Result<Self> {
        Ok(Self {
            device: device.clone(),
            context: context.clone(),
            constants: resources::create_constants(device)?,
            sampler: create_sampler(device)?,
            clip: create_clip_state(device)?,
            blend: create_blend_state(device)?,
            vertex_shader: resources::create_vertex_shader(device)?,
            texture_shader: resources::create_pixel_shader(device, &shaders::texture())?,
            movement_blur_shader: resources::create_pixel_shader(
                device,
                &shaders::movement_blur(),
            )?,
            zoom_blur_shader: resources::create_pixel_shader(device, &shaders::zoom_blur())?,
            gradient_shader: resources::create_pixel_shader(device, &shaders::gradient())?,
            cursor_shader: resources::create_pixel_shader(device, &shaders::cursor())?,
            background: Background::default(),
        })
    }

    /// Creates a texture this renderer can draw into. Export encodes it; the
    /// preview draws into its swapchain instead.
    pub(crate) fn create_target(&self, size: PhysicalSize) -> Result<ID3D11Texture2D> {
        resources::create_output(&self.device, size.width, size.height)
    }

    pub(crate) fn target_view(&self, texture: &ID3D11Texture2D) -> Result<ID3D11RenderTargetView> {
        let mut view = None;
        unsafe {
            self.device
                .CreateRenderTargetView(texture, None, Some(&mut view))
        }
        .map_err(|error| {
            RenderError::Frame(format!("could not create a render target: {error}"))
        })?;
        view.ok_or_else(|| RenderError::Frame("render target was null".into()))
    }

    pub(crate) fn shader_view(
        &self,
        texture: &ID3D11Texture2D,
    ) -> Result<ID3D11ShaderResourceView> {
        let mut view = None;
        unsafe {
            self.device
                .CreateShaderResourceView(texture, None, Some(&mut view))
        }
        .map_err(|error| RenderError::Frame(format!("could not create a frame view: {error}")))?;
        view.ok_or_else(|| RenderError::Frame("frame view was null".into()))
    }

    /// Draws one composed frame into `target`.
    ///
    /// `recording` is the decoded picture. Without one the canvas and its
    /// background are still drawn, which is what an editor frame looks like
    /// before the first decode arrives.
    pub(crate) fn draw(
        &mut self,
        target: &ID3D11RenderTargetView,
        state: &CompositionState,
        recording: Option<&ID3D11ShaderResourceView>,
    ) -> Result<()> {
        if state.is_empty() {
            return Ok(());
        }
        self.background
            .ensure(&self.device, Constants::image_path(&state.background))?;

        self.begin(target, state);
        self.draw_background(state)?;
        if let Some(recording) = recording {
            let (constants, pass) = Constants::recording(state);
            self.draw_quad(constants, Some(recording), self.recording_shader(pass))?;
        }
        if let Some(constants) = Constants::cursor(state) {
            // The only blended layer. Everything under it is opaque, and the
            // cursor's antialiased outline is the one place the composition
            // needs a source-over rather than a straight write.
            unsafe { self.context.OMSetBlendState(&self.blend, None, u32::MAX) };
            let drawn = self.draw_quad(constants, None, &self.cursor_shader);
            unsafe { self.context.OMSetBlendState(None, None, u32::MAX) };
            drawn?;
        }
        unsafe { self.context.Flush() };
        Ok(())
    }

    /// Points the pipeline at the target, clips to the canvas, and paints the
    /// editor workspace around it.
    fn begin(&self, target: &ID3D11RenderTargetView, state: &CompositionState) {
        let target_size = state.target_size;
        let (left, top, right, bottom) = canvas_clip(state);
        unsafe {
            self.context.OMSetRenderTargets(
                Some(&[Some(target.clone())]),
                None::<&ID3D11DepthStencilView>,
            );
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: target_size.width as f32,
                Height: target_size.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            self.context.RSSetState(&self.clip);
            self.context.RSSetScissorRects(Some(&[RECT {
                left,
                top,
                right,
                bottom,
            }]));
            // The clear ignores the scissor, which is what paints the editor
            // workspace around the canvas in one step.
            self.context
                .ClearRenderTargetView(target, &state.canvas.surround);
        }
    }

    fn draw_background(&self, state: &CompositionState) -> Result<()> {
        self.draw_quad(Constants::canvas_fill(state), None, &self.gradient_shader)?;
        let Some(image) = self.background.texture() else {
            return Ok(());
        };
        self.draw_quad(
            Constants::canvas_image(state, image.width, image.height),
            Some(&image.view),
            &self.texture_shader,
        )
    }

    fn recording_shader(&self, pass: RecordingPass) -> &ID3D11PixelShader {
        match pass {
            RecordingPass::Sharp => &self.texture_shader,
            RecordingPass::Movement => &self.movement_blur_shader,
            RecordingPass::Zoom => &self.zoom_blur_shader,
        }
    }

    fn draw_quad(
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
                .map_err(|error| {
                    RenderError::Frame(format!("could not map the constants: {error}"))
                })?;
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

fn create_sampler(device: &ID3D11Device) -> Result<ID3D11SamplerState> {
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
    .map_err(|error| RenderError::Device(format!("could not create the sampler: {error}")))?;
    sampler.ok_or_else(|| RenderError::Device("composition sampler was null".into()))
}

/// Source-over blending for premultiplied colour, which is what the cursor
/// shader emits and what the composition swapchain expects.
fn create_blend_state(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let mut target = [D3D11_RENDER_TARGET_BLEND_DESC::default(); 8];
    target[0] = D3D11_RENDER_TARGET_BLEND_DESC {
        BlendEnable: true.into(),
        SrcBlend: D3D11_BLEND_ONE,
        DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOp: D3D11_BLEND_OP_ADD,
        SrcBlendAlpha: D3D11_BLEND_ONE,
        DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOpAlpha: D3D11_BLEND_OP_ADD,
        RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
    };
    let mut state = None;
    unsafe {
        device.CreateBlendState(
            &D3D11_BLEND_DESC {
                AlphaToCoverageEnable: false.into(),
                IndependentBlendEnable: false.into(),
                RenderTarget: target,
            },
            Some(&mut state),
        )
    }
    .map_err(|error| RenderError::Device(format!("could not create the blend state: {error}")))?;
    state.ok_or_else(|| RenderError::Device("composition blend state was null".into()))
}

/// A rasterizer state that clips to the canvas.
///
/// The recording layer may be scaled past the canvas during a zoom, so the
/// canvas owns a real clip rather than relying on the render target edges —
/// which only coincide with the canvas during export. Culling is disabled so
/// the quad's winding cannot matter.
fn create_clip_state(device: &ID3D11Device) -> Result<ID3D11RasterizerState> {
    let mut state = None;
    unsafe {
        device.CreateRasterizerState(
            &D3D11_RASTERIZER_DESC {
                FillMode: D3D11_FILL_SOLID,
                CullMode: D3D11_CULL_NONE,
                DepthClipEnable: true.into(),
                ScissorEnable: true.into(),
                ..Default::default()
            },
            Some(&mut state),
        )
    }
    .map_err(|error| RenderError::Device(format!("could not create the clip state: {error}")))?;
    state.ok_or_else(|| RenderError::Device("composition clip state was null".into()))
}
