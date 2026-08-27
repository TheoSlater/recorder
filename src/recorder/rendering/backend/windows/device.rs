//! The GPU device Media Foundation and the compositor share.
//!
//! Media Foundation decodes into textures owned by whichever device it was
//! given. If the compositor used a different device, every frame would need a
//! shared-resource handoff or a copy through system memory — exactly the round
//! trip this migration exists to remove. One device for both ends means a
//! decoded texture is sampled where it already lives.
//!
//! Multithread protection is not optional. The source reader and the sink
//! writer drive this device from their own worker threads, and without it
//! `IMFDXGIDeviceManager::LockDevice` can block indefinitely — a `ReadSample`
//! that never returns while consuming no CPU. The preview needs it for a second
//! reason: decoding runs on its own thread because GPUI's thread is an STA (its
//! Windows platform calls `OleInitialize`, so `CoInitializeEx(COINIT_MULTITHREADED)`
//! fails there), and multithread protection is what makes it legal for that
//! thread to produce textures the render thread samples.

use windows::Win32::Graphics::{
    Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP},
    Direct3D11::{
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
    },
};
use windows::core::Interface;

use super::super::super::RenderError;

/// Creates the device both ends share.
///
/// `BGRA_SUPPORT` is required by DirectComposition, `VIDEO_SUPPORT` by the
/// hardware decoder, and the WARP fallback keeps the preview working on a
/// machine whose hardware device cannot be created.
pub(crate) fn create() -> Result<(ID3D11Device, ID3D11DeviceContext), RenderError> {
    let flags = D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    let mut device = None;
    let mut context = None;
    let hardware = unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            flags,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    };
    if hardware.is_err() {
        device = None;
        context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                Default::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|error| {
            RenderError::Device(format!("no D3D11 device, hardware or WARP: {error}"))
        })?;
    }
    let (device, context) = match (device, context) {
        (Some(device), Some(context)) => (device, context),
        _ => return Err(RenderError::Device("D3D11 returned no device".into())),
    };

    // Without this the decoder thread and the render thread would be racing on
    // the same immediate context.
    let multithread: ID3D11Multithread = context
        .cast()
        .map_err(|error| RenderError::Device(format!("no multithread interface: {error}")))?;
    let _previous = unsafe { multithread.SetMultithreadProtected(true) };

    Ok((device, context))
}
