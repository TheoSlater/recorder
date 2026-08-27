//! Decodes recording frames onto the compositor's device.
//!
//! The decode runs on its own thread. GPUI's thread is an STA — its Windows
//! platform calls `OleInitialize` — so Media Foundation cannot be started
//! there. Sharing the compositor's device is what makes that split cheap: the
//! hardware decoder writes into textures the renderer can already sample, with
//! no readback and no cross-device transfer.
//!
//! Each decoded frame is copied into a texture this module owns before it
//! crosses threads. The copy stays on the GPU, and it decouples the frame from
//! the Media Foundation sample that produced it, which would otherwise recycle
//! the decoder's texture back into its pool while the renderer was still
//! sampling it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Graphics::{
    Direct3D11::{
        D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
        ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
    },
    Dxgi::Common::DXGI_SAMPLE_DESC,
};

use crate::recorder::export::decoder::{self, DeviceContext};

use super::super::super::{FrameId, FrameQueue, PhysicalSize, RenderError};

/// The single-slot hand-off from the decode thread to the compositor.
type Frames = Arc<Mutex<FrameQueue<Crossing>>>;

/// A decoded frame living on the compositor's device.
pub(super) struct DecodedTexture {
    pub(super) view: ID3D11ShaderResourceView,
    pub(super) timestamp_100ns: u64,
    _texture: ID3D11Texture2D,
}

/// A decoded texture on its way from the decoder thread to the renderer.
///
/// SAFETY: the device both threads use is created multithread-protected (see
/// [`super::device::create`]), which is precisely the guarantee that D3D11
/// resources created on one thread may be used from another. The texture is
/// also a private copy, so no Media Foundation sample lifetime crosses with it.
struct Crossing(DecodedTexture);

unsafe impl Send for Crossing {}

/// A decode thread that follows a requested timestamp.
///
/// Latest request wins: the renderer stores the timestamp it wants and the
/// worker decodes towards it, skipping anything the playhead has already moved
/// past. Only one decoded frame is ever in flight, so a slow decode drops work
/// instead of queueing pictures nobody will see.
pub(super) struct PreviewSource {
    frames: Frames,
    request: Arc<Request>,
    shutdown: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

struct Request {
    timestamp_100ns: AtomicU64,
    generation: AtomicU64,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            // No real timestamp, so the first request always differs and the
            // first frame is decoded rather than treated as already served.
            timestamp_100ns: AtomicU64::new(u64::MAX),
            generation: AtomicU64::new(0),
        }
    }
}

impl PreviewSource {
    pub(super) fn start(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        path: PathBuf,
    ) -> Result<Self, RenderError> {
        let device = device.clone();
        let context = context.clone();
        let request = Arc::new(Request::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        // One slot: the renderer only ever draws the newest frame.
        let frames: Frames = Arc::new(Mutex::new(FrameQueue::new()));
        let worker_request = request.clone();
        let worker_shutdown = shutdown.clone();
        let worker_frames = frames.clone();

        let worker = std::thread::Builder::new()
            .name("recorder-preview-decode".to_string())
            .spawn(move || {
                if let Err(error) = decode_loop(
                    &device,
                    &context,
                    &path,
                    &worker_request,
                    &worker_shutdown,
                    &worker_frames,
                ) {
                    tracing::error!(target: "recorder::rendering", %error, "preview decode stopped");
                }
            })
            .map_err(|error| RenderError::Frame(format!("could not start the decoder: {error}")))?;

        Ok(Self {
            frames,
            request,
            shutdown,
            worker: Some(worker),
        })
    }

    /// Asks for the frame current at `timestamp_100ns`. Replaces any request
    /// the worker has not started yet.
    ///
    /// A repeated timestamp is not a new request. The editor asks on every
    /// painted frame, so without this a paused preview would decode the same
    /// picture continuously.
    pub(super) fn seek(&self, timestamp_100ns: u64) {
        if self
            .request
            .timestamp_100ns
            .swap(timestamp_100ns, Ordering::AcqRel)
            != timestamp_100ns
        {
            self.request.generation.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// Takes the newest decoded frame, if one arrived since the last call.
    pub(super) fn take(&self) -> Option<DecodedTexture> {
        let mut frames = self.frames.lock().ok()?;
        frames.take().map(|(_, crossing)| crossing.0)
    }

    /// Frames decoded but never shown, because a newer one replaced them or the
    /// playhead moved past them first.
    pub(super) fn dropped(&self) -> u64 {
        self.frames
            .lock()
            .map(|frames| frames.dropped())
            .unwrap_or(0)
    }
}

impl Drop for PreviewSource {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn decode_loop(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    path: &std::path::Path,
    request: &Request,
    shutdown: &AtomicBool,
    frames: &Frames,
) -> Result<(), String> {
    let _media = decoder::initialize_media().map_err(|error| error.to_string())?;
    let device_context =
        DeviceContext::adopt(device.clone(), context.clone()).map_err(|error| error.to_string())?;
    let mut source =
        decoder::Decoder::open_on(path, device_context).map_err(|error| error.to_string())?;
    // The media frame size, which a hardware decoder may hand back inside a
    // larger, alignment-padded texture.
    let media = PhysicalSize::new(source.source.width, source.source.height);

    let mut served = u64::MAX;
    let mut sequence = 0;
    while !shutdown.load(Ordering::Acquire) {
        let generation = request.generation.load(Ordering::Acquire);
        if generation == served {
            std::thread::sleep(std::time::Duration::from_millis(2));
            continue;
        }
        let timestamp = request.timestamp_100ns.load(Ordering::Acquire);
        served = generation;
        sequence += 1;

        let frame = match source.frame_at(timestamp) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(target: "recorder::rendering", %error, "preview decode failed");
                continue;
            }
        };
        let decoded = match copy_frame(
            device,
            context,
            &frame.texture,
            media,
            frame.timestamp_100ns,
        ) {
            Ok(decoded) => decoded,
            Err(error) => {
                tracing::warn!(target: "recorder::rendering", %error, "preview copy failed");
                continue;
            }
        };
        // Drop whatever the renderer has not taken: newest frame wins.
        let id = FrameId::new(generation, sequence, decoded.timestamp_100ns / 10);
        if let Ok(mut frames) = frames.lock() {
            frames.offer(id, Crossing(decoded));
        }
    }
    Ok(())
}

/// Copies a decoded frame into a texture this module owns, cropped to the media
/// frame size.
///
/// A hardware decoder may return the picture inside a larger texture padded for
/// alignment. Sampling that texture across its whole extent would letterbox
/// garbage into the recording layer, so the copy takes exactly the media
/// rectangle and the renderer can keep sampling the full 0..1 range.
fn copy_frame(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    source: &ID3D11Texture2D,
    media: PhysicalSize,
    timestamp_100ns: u64,
) -> Result<DecodedTexture, RenderError> {
    let mut description = D3D11_TEXTURE2D_DESC::default();
    unsafe { source.GetDesc(&mut description) };
    let size = PhysicalSize::new(
        media.width.min(description.Width).max(1),
        media.height.min(description.Height).max(1),
    );
    let owned = D3D11_TEXTURE2D_DESC {
        Width: size.width,
        Height: size.height,
        MipLevels: 1,
        ArraySize: 1,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        ..description
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&owned, None, Some(&mut texture)) }.map_err(|error| {
        RenderError::Frame(format!("could not create a frame texture: {error}"))
    })?;
    let texture = texture.ok_or_else(|| RenderError::Frame("frame texture was null".into()))?;
    let region = D3D11_BOX {
        left: 0,
        top: 0,
        front: 0,
        right: size.width,
        bottom: size.height,
        back: 1,
    };
    unsafe { context.CopySubresourceRegion(&texture, 0, 0, 0, 0, source, 0, Some(&region)) };

    let mut view = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut view)) }
        .map_err(|error| RenderError::Frame(format!("could not create a frame view: {error}")))?;
    let view = view.ok_or_else(|| RenderError::Frame("frame view was null".into()))?;

    Ok(DecodedTexture {
        view,
        timestamp_100ns,
        _texture: texture,
    })
}
