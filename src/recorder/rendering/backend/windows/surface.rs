use raw_window_handle::{RawWindowHandle, WindowHandle};
use windows::Win32::{
    Foundation::HWND,
    Graphics::{
        Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0},
        Direct3D11::{
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
            ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D,
        },
        DirectComposition::{
            DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
        },
        Dxgi::{
            Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
            CreateDXGIFactory2, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
            DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice,
            IDXGIFactory2, IDXGISwapChain1,
        },
    },
};
use windows::core::Interface;

use super::super::super::super::export::renderer::Constants;
use super::super::super::{
    CompositionState, FrameId, PhysicalSize, PreviewBounds, PreviewRenderer, RenderError,
};
use super::{device, pipeline::Pipeline, source, texture};

/// Two buffers is enough for a preview that always presents the newest frame;
/// a deeper chain only adds latency between decode and screen.
const BUFFER_COUNT: u32 = 2;

/// A composition surface layered beneath GPUI's own output in the same window.
pub(crate) struct DirectCompositionSurface {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    composition: IDCompositionDevice,
    /// Held for its lifetime: dropping the target detaches the visual tree.
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
    swap_chain: IDXGISwapChain1,
    bounds: PreviewBounds,
    pipeline: Pipeline,
    placeholder: texture::StaticTexture,
    /// The most recently decoded frame, or `None` before one arrives.
    frame: Option<source::DecodedTexture>,
    streaming: Option<source::PreviewSource>,
}

impl DirectCompositionSurface {
    /// Attaches to `hwnd`, which must be the GPUI window's handle.
    ///
    /// Fails rather than falling back when the non-topmost target is refused,
    /// so the caller can choose another strategy instead of silently rendering
    /// nowhere.
    pub(crate) fn new(hwnd: isize, bounds: PreviewBounds) -> Result<Self, RenderError> {
        if bounds.size.is_empty() {
            return Err(RenderError::Surface("preview rectangle is empty".into()));
        }
        let (device, context) = device::create()?;
        let dxgi_device: IDXGIDevice = device
            .cast()
            .map_err(|error| RenderError::Device(format!("no DXGI device: {error}")))?;

        let composition: IDCompositionDevice = unsafe { DCompositionCreateDevice(&dxgi_device) }
            .map_err(|error| {
                RenderError::Surface(format!("could not create a composition device: {error}"))
            })?;

        // The decisive call. GPUI already owns the topmost target for this
        // window, so this asks Windows for the non-topmost one.
        let target = unsafe { composition.CreateTargetForHwnd(HWND(hwnd as *mut _), false) }
            .map_err(|error| {
                RenderError::Surface(format!(
                    "the window refused a second composition target: {error}"
                ))
            })?;

        let swap_chain = create_swap_chain(&device, bounds.size)?;
        let visual = unsafe { composition.CreateVisual() }
            .map_err(|error| RenderError::Surface(format!("could not create a visual: {error}")))?;

        let pipeline = Pipeline::new(&device, context.clone())?;
        let placeholder = texture::placeholder(&device)?;
        let surface = Self {
            device,
            context,
            composition,
            _target: target,
            visual,
            swap_chain,
            bounds,
            pipeline,
            placeholder,
            frame: None,
            streaming: None,
        };
        unsafe {
            surface
                .visual
                .SetContent(&surface.swap_chain)
                .and_then(|_| target_set_root(&surface))
                .map_err(|error| {
                    RenderError::Surface(format!("could not attach the visual: {error}"))
                })?;
        }
        surface.place(bounds)?;
        Ok(surface)
    }

    /// Moves and resizes the surface to the rectangle GPUI assigned.
    pub(crate) fn place(&self, bounds: PreviewBounds) -> Result<(), RenderError> {
        unsafe {
            self.visual
                .SetOffsetX2(bounds.x as f32)
                .and_then(|_| self.visual.SetOffsetY2(bounds.y as f32))
                .and_then(|_| self.composition.Commit())
                .map_err(|error| {
                    RenderError::Surface(format!("could not position the preview: {error}"))
                })
        }
    }

    pub(crate) fn resize(&mut self, size: PhysicalSize) -> Result<(), RenderError> {
        if size.is_empty() {
            return Ok(());
        }
        unsafe {
            self.swap_chain
                .ResizeBuffers(
                    BUFFER_COUNT,
                    size.width,
                    size.height,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    Default::default(),
                )
                .map_err(|error| {
                    RenderError::Surface(format!("could not resize the preview: {error}"))
                })?;
        }
        self.bounds.size = size;
        Ok(())
    }

    /// Decodes the frame current at `timestamp_100ns` and keeps it for
    /// rendering.
    ///
    /// The decoder runs on the compositor's own device, so the texture it
    /// produces is sampled where it was written: no readback, no cross-device
    /// transfer, and no `RenderImage` upload.
    pub(crate) fn decode(
        &mut self,
        path: std::path::PathBuf,
        timestamp_100ns: u64,
    ) -> Result<(), RenderError> {
        let frame = source::decode_frame(&self.device, &self.context, path, timestamp_100ns)?;
        tracing::info!(
            target: "recorder::rendering",
            width = frame.size.width,
            height = frame.size.height,
            timestamp_100ns = frame.timestamp_100ns,
            "decoded a GPU frame onto the compositor device"
        );
        self.frame = Some(frame);
        Ok(())
    }

    /// Starts following the playhead, decoding on this surface's own device.
    pub(crate) fn stream(&mut self, path: std::path::PathBuf) -> Result<(), RenderError> {
        self.streaming = Some(source::PreviewSource::start(
            &self.device,
            &self.context,
            path,
        )?);
        Ok(())
    }

    /// Asks the decoder for the frame at `timestamp_us` and adopts whatever has
    /// finished since the last call.
    ///
    /// Both halves are non-blocking: a decode slower than the preview never
    /// stalls the editor, it only means this frame repeats.
    pub(crate) fn follow(&mut self, timestamp_us: u64) {
        let Some(streaming) = self.streaming.as_ref() else {
            return;
        };
        streaming.seek(timestamp_us.saturating_mul(10));
        if let Some(frame) = streaming.take() {
            self.frame = Some(frame);
        }
    }

    /// Keeps the surface on the rectangle GPUI assigned.
    ///
    /// Resize, DPI change, and monitor moves all arrive here as a new
    /// rectangle; the swapchain is only rebuilt when the size actually differs.
    pub(crate) fn set_bounds(&mut self, bounds: PreviewBounds) -> Result<(), RenderError> {
        if bounds == self.bounds {
            return Ok(());
        }
        let resized = bounds.size != self.bounds.size;
        self.bounds = bounds;
        if resized {
            self.resize(bounds.size)?;
        }
        self.place(bounds)
    }

    /// The picture to compose: a decoded frame once one exists, otherwise the
    /// bring-up stand-in.
    fn source_view(&self) -> &ID3D11ShaderResourceView {
        match self.frame.as_ref() {
            Some(frame) => &frame.view,
            None => &self.placeholder.view,
        }
    }

    /// A render target view over the current back buffer.
    ///
    /// Taken per frame rather than cached: a flip-model swapchain rotates its
    /// buffers, and `ResizeBuffers` invalidates any view held across it.
    fn back_buffer(&self) -> Result<ID3D11RenderTargetView, RenderError> {
        let back_buffer: ID3D11Texture2D = unsafe { self.swap_chain.GetBuffer(0) }
            .map_err(|error| RenderError::Frame(format!("no back buffer: {error}")))?;
        let mut view: Option<ID3D11RenderTargetView> = None;
        unsafe {
            self.device
                .CreateRenderTargetView(&back_buffer, None, Some(&mut view))
                .map_err(|error| RenderError::Frame(format!("no render target: {error}")))?;
        }
        view.ok_or_else(|| RenderError::Frame("render target was null".into()))
    }

    /// Fills the surface with one colour.
    pub(crate) fn clear(&self, color: [f32; 4]) -> Result<(), RenderError> {
        self.pipeline
            .begin(&self.back_buffer()?, self.bounds.size, color);
        Ok(())
    }

    pub(crate) fn present(&self) -> Result<(), RenderError> {
        unsafe { self.swap_chain.Present(0, DXGI_PRESENT(0)) }
            .ok()
            .map_err(|error| RenderError::Frame(format!("could not present: {error}")))
    }
}

impl PreviewRenderer for DirectCompositionSurface {
    fn resize(&mut self, size: PhysicalSize) -> Result<(), RenderError> {
        DirectCompositionSurface::resize(self, size)
    }

    /// Composes one frame.
    ///
    /// The picture is still the bring-up stand-in: this proves the geometry,
    /// sampler, and target are correct before a decoded texture replaces it.
    /// The recording rectangle already comes from the shared
    /// [`CompositionState`], so swapping the source texture is the only change
    /// that milestone needs.
    fn render(
        &mut self,
        _frame: &FrameId,
        composition: &CompositionState,
    ) -> Result<(), RenderError> {
        if composition.is_empty() {
            return Ok(());
        }
        self.pipeline
            .begin(&self.back_buffer()?, self.bounds.size, [0.0; 4]);
        self.pipeline.draw_texture(
            Constants::for_rect(composition.frame.recording),
            self.source_view(),
        )
    }

    fn present(&mut self) -> Result<(), RenderError> {
        DirectCompositionSurface::present(self)
    }
}

unsafe fn target_set_root(surface: &DirectCompositionSurface) -> windows::core::Result<()> {
    unsafe { surface._target.SetRoot(&surface.visual) }
}

fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext), RenderError> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            // BGRA support is required for anything that composes with D2D or
            // DirectComposition.
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|error| RenderError::Device(format!("could not create a D3D11 device: {error}")))?;
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(RenderError::Device("D3D11 returned no device".into())),
    }
}

fn create_swap_chain(
    device: &ID3D11Device,
    size: PhysicalSize,
) -> Result<IDXGISwapChain1, RenderError> {
    let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(Default::default()) }
        .map_err(|error| RenderError::Device(format!("no DXGI factory: {error}")))?;
    let description = DXGI_SWAP_CHAIN_DESC1 {
        Width: size.width,
        Height: size.height,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: BUFFER_COUNT,
        // Composition swapchains only support stretch scaling.
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        Flags: 0,
    };
    unsafe { factory.CreateSwapChainForComposition(device, &description, None) }
        .map_err(|error| RenderError::Surface(format!("could not create a swapchain: {error}")))
}

/// Reads the HWND out of a GPUI window handle.
///
/// This is the only place the renderer touches a windowing handle, and it takes
/// the borrowed handle rather than the GPUI type, so GPUI stays out of this
/// subsystem entirely.
pub(crate) fn window_handle(handle: WindowHandle<'_>) -> Option<isize> {
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
        _ => None,
    }
}

/// Renders the bring-up frame onto a freshly attached surface.
///
/// The clear colour proves the surface is layered correctly even where nothing
/// is drawn, and the texture on top proves the pipeline, sampler, and
/// composition geometry all reach the screen. Replacing that texture with a
/// decoded one is the next milestone.
pub(crate) fn probe(
    hwnd: isize,
    bounds: PreviewBounds,
    composition: &CompositionState,
    video_path: std::path::PathBuf,
) -> Result<DirectCompositionSurface, RenderError> {
    let mut surface = DirectCompositionSurface::new(hwnd, bounds)?;
    // A decode failure must not lose the surface: the stand-in texture keeps
    // the milestone before it legible instead of leaving a blank rectangle.
    if let Err(error) = surface.decode(video_path, 0) {
        tracing::error!(target: "recorder::rendering", %error, "could not decode a preview frame");
    }
    // Magenta: no part of the editor theme uses it, so it cannot be mistaken
    // for GPUI's own painting.
    surface.clear([1.0, 0.0, 1.0, 1.0])?;
    surface.pipeline.draw_texture(
        Constants::for_rect(composition.frame.recording),
        surface.source_view(),
    )?;
    PreviewRenderer::present(&mut surface)?;
    Ok(surface)
}
