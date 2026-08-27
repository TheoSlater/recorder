use raw_window_handle::{RawWindowHandle, WindowHandle};
use windows::Win32::{
    Foundation::HWND,
    Graphics::{
        Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D},
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

use super::super::super::{
    CompositionState, FrameId, PhysicalSize, PreviewBounds, PreviewRenderer, RenderError,
};
use super::composition::CompositionRenderer;
use super::{device, source};

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
    renderer: CompositionRenderer,
    /// The most recently decoded frame, or `None` before one arrives.
    frame: Option<source::DecodedTexture>,
    streaming: Option<source::PreviewSource>,
    presented: u64,
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

        let renderer = CompositionRenderer::new(&device, &context)?;
        let surface = Self {
            device,
            context,
            composition,
            _target: target,
            visual,
            swap_chain,
            bounds,
            renderer,
            frame: None,
            streaming: None,
            presented: 0,
        };
        unsafe {
            surface
                .visual
                .SetContent(&surface.swap_chain)
                .and_then(|_| surface._target.SetRoot(&surface.visual))
                .map_err(|error| {
                    RenderError::Surface(format!("could not attach the visual: {error}"))
                })?;
        }
        surface.place(bounds)?;
        Ok(surface)
    }

    /// Moves and resizes the surface to the rectangle GPUI assigned.
    fn place(&self, bounds: PreviewBounds) -> Result<(), RenderError> {
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

    /// Counters for the native path: frames put on screen, and frames decoded
    /// but never shown because the playhead moved past them first.
    pub(crate) fn counters(&self) -> (u64, u64) {
        (
            self.presented,
            self.streaming
                .as_ref()
                .map(source::PreviewSource::dropped)
                .unwrap_or(0),
        )
    }

    /// A render target view over the current back buffer.
    ///
    /// Taken per frame rather than cached: a flip-model swapchain rotates its
    /// buffers, and `ResizeBuffers` invalidates any view held across it.
    fn back_buffer(&self) -> Result<ID3D11RenderTargetView, RenderError> {
        let back_buffer: ID3D11Texture2D = unsafe { self.swap_chain.GetBuffer(0) }
            .map_err(|error| RenderError::Frame(format!("no back buffer: {error}")))?;
        self.renderer.target_view(&back_buffer)
    }
}

impl PreviewRenderer for DirectCompositionSurface {
    fn resize(&mut self, size: PhysicalSize) -> Result<(), RenderError> {
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

    /// Composes the whole preview: workspace surround, canvas background,
    /// the transformed recording, and the reconstructed cursor.
    ///
    /// Every pixel inside the preview rectangle comes from here. GPUI keeps
    /// painting the editor around and over it, but nothing of the composition
    /// itself is left on that side.
    fn render(
        &mut self,
        _frame: &FrameId,
        composition: &CompositionState,
    ) -> Result<(), RenderError> {
        if composition.is_empty() {
            return Ok(());
        }
        let target = self.back_buffer()?;
        let recording = self.frame.as_ref().map(|frame| &frame.view);
        self.renderer.draw(&target, composition, recording)
    }

    fn present(&mut self) -> Result<(), RenderError> {
        unsafe { self.swap_chain.Present(0, DXGI_PRESENT(0)) }
            .ok()
            .map_err(|error| RenderError::Frame(format!("could not present: {error}")))?;
        self.presented += 1;
        Ok(())
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
