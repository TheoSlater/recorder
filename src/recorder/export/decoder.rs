use std::{os::windows::ffi::OsStrExt, path::Path};

use anyhow::{Context, Result, anyhow, bail};
use windows::{
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP},
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                ID3D11Texture2D,
            },
        },
        Media::MediaFoundation::{
            IMFAttributes, IMFDXGIBuffer, IMFDXGIDeviceManager, IMFMediaType, IMFSample,
            IMFSourceReader, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
            MF_PD_DURATION, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READER_D3D_MANAGER,
            MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
            MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR,
            MFCreateAttributes, MFCreateDXGIDeviceManager, MFCreateMediaType,
            MFCreateSourceReaderFromURL, MFMediaType_Video, MFVideoFormat_ARGB32,
        },
        System::{
            Com::StructuredStorage::{PropVariantClear, PropVariantToInt64},
            Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        },
    },
    core::{Interface, PCWSTR},
};

use super::super::composition::SourceSize;

const VIDEO_STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const MEDIA_TIME_PER_SECOND: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameRate {
    pub(crate) numerator: u32,
    pub(crate) denominator: u32,
}

impl FrameRate {
    pub(crate) fn frame_count(self, duration_100ns: u64) -> u64 {
        let numerator = u128::from(duration_100ns) * u128::from(self.numerator);
        let denominator = u128::from(MEDIA_TIME_PER_SECOND) * u128::from(self.denominator);
        numerator.div_ceil(denominator).max(1) as u64
    }

    pub(crate) fn timestamp(self, index: u64) -> u64 {
        (u128::from(index) * u128::from(MEDIA_TIME_PER_SECOND) * u128::from(self.denominator)
            / u128::from(self.numerator)) as u64
    }

    pub(crate) fn frame_duration(self, index: u64) -> u64 {
        self.timestamp(index.saturating_add(1))
            .saturating_sub(self.timestamp(index))
            .max(1)
    }
}

pub(crate) struct DeviceContext {
    pub(crate) device: ID3D11Device,
    pub(crate) context: ID3D11DeviceContext,
    pub(crate) manager: IMFDXGIDeviceManager,
}

pub(crate) struct SourceFrame {
    pub(crate) texture: ID3D11Texture2D,
    pub(crate) timestamp_100ns: u64,
    _sample: IMFSample,
}

pub(crate) struct Decoder {
    reader: IMFSourceReader,
    _device: DeviceContext,
    pub(crate) source: SourceSize,
    pub(crate) frame_rate: FrameRate,
    pub(crate) duration_100ns: u64,
    pending: Option<SourceFrame>,
    current: Option<SourceFrame>,
}

pub(crate) fn initialize_media() -> Result<MediaGuards> {
    let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if result.is_err() {
        return Err(anyhow!("could not initialize COM: {result:?}"));
    }
    if let Err(error) = unsafe {
        windows::Win32::Media::MediaFoundation::MFStartup(
            windows::Win32::Media::MediaFoundation::MF_VERSION,
            windows::Win32::Media::MediaFoundation::MFSTARTUP_FULL,
        )
    } {
        unsafe { CoUninitialize() };
        return Err(anyhow!("could not initialize Media Foundation: {error}"));
    }
    Ok(MediaGuards)
}

pub(crate) struct MediaGuards;

impl Drop for MediaGuards {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Media::MediaFoundation::MFShutdown();
            CoUninitialize();
        }
    }
}

impl Decoder {
    pub(crate) fn device_context(&self) -> &DeviceContext {
        &self._device
    }

    pub(crate) fn open(path: &Path) -> Result<Self> {
        let device = create_device().context("could not create the export D3D11 device")?;
        let reader = create_reader(path, &device.manager)?;
        let native_type = unsafe { reader.GetNativeMediaType(VIDEO_STREAM, 0) }
            .context("could not read recording media type")?;
        let source = read_source_size(&native_type)?;
        let frame_rate = read_frame_rate(&native_type).unwrap_or(FrameRate {
            numerator: 60,
            denominator: 1,
        });
        let duration_100ns = read_duration(&reader).unwrap_or(0);
        set_output_type(&reader)?;
        let output_type = unsafe { reader.GetCurrentMediaType(VIDEO_STREAM) }
            .context("could not read GPU decoder output type")?;
        let output_source = read_source_size(&output_type)?;
        if output_source != source {
            bail!("decoder changed the recording frame dimensions");
        }

        Ok(Self {
            reader,
            _device: device,
            source,
            frame_rate,
            duration_100ns,
            pending: None,
            current: None,
        })
    }

    /// Returns the source frame that is current at `timestamp_100ns`.
    /// Source Reader remains sequential; a single future frame is retained.
    pub(crate) fn frame_at(&mut self, timestamp_100ns: u64) -> Result<&SourceFrame> {
        if self.current.is_none() {
            self.current = self.read_frame()?;
        }
        loop {
            let next = self.pending.take().or(self.read_frame()?);
            let Some(next) = next else {
                break;
            };
            if next.timestamp_100ns <= timestamp_100ns {
                self.current = Some(next);
            } else {
                self.pending = Some(next);
                break;
            }
        }
        self.current
            .as_ref()
            .ok_or_else(|| anyhow!("recording contains no video frames"))
    }

    fn read_frame(&self) -> Result<Option<SourceFrame>> {
        let mut flags = 0u32;
        let mut timestamp = 0i64;
        let mut sample = None;
        unsafe {
            self.reader
                .ReadSample(
                    VIDEO_STREAM,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )
                .context("could not decode recording frame")?;
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Ok(None);
        }
        if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            bail!("Media Foundation reported a decoder error");
        }
        let Some(sample) = sample else {
            return Ok(None);
        };
        let buffer =
            unsafe { sample.GetBufferByIndex(0) }.context("decoded frame has no media buffer")?;
        let dxgi: IMFDXGIBuffer = buffer
            .cast()
            .context("decoded frame is not a D3D11 buffer")?;
        let texture = unsafe {
            let mut texture = None;
            dxgi.GetResource(&ID3D11Texture2D::IID, &mut texture as *mut _ as *mut _)
                .context("could not obtain decoded D3D11 texture")?;
            texture.ok_or_else(|| anyhow!("decoded D3D11 texture is null"))?
        };
        Ok(Some(SourceFrame {
            texture,
            timestamp_100ns: timestamp.max(0) as u64,
            _sample: sample,
        }))
    }
}

fn create_device() -> Result<DeviceContext> {
    let flags = D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    let mut device = None;
    let mut context = None;
    let result = unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            flags,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    };
    if result.is_err() {
        device = None;
        context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
    } else {
        result?;
    }
    let device = device.ok_or_else(|| anyhow!("D3D11 device creation returned null"))?;
    let context = context.ok_or_else(|| anyhow!("D3D11 context creation returned null"))?;
    let mut token = 0;
    let mut manager = None;
    unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) }
        .context("could not create Media Foundation D3D manager")?;
    let manager = manager.ok_or_else(|| anyhow!("Media Foundation D3D manager is null"))?;
    unsafe { manager.ResetDevice(&device, token) }.context("could not bind D3D11 device")?;
    Ok(DeviceContext {
        device,
        context,
        manager,
    })
}

fn create_reader(path: &Path, manager: &IMFDXGIDeviceManager) -> Result<IMFSourceReader> {
    let mut attributes = None;
    unsafe { MFCreateAttributes(&mut attributes, 4) }
        .context("could not create reader attributes")?;
    let attributes: IMFAttributes =
        attributes.ok_or_else(|| anyhow!("reader attributes are null"))?;
    unsafe {
        attributes.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, manager)?;
        attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
    }
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { MFCreateSourceReaderFromURL(PCWSTR(path.as_ptr()), &attributes) }
        .context("could not open recording with Media Foundation")
}

fn set_output_type(reader: &IMFSourceReader) -> Result<()> {
    let media_type: IMFMediaType = unsafe { MFCreateMediaType() }?;
    unsafe {
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_ARGB32)?;
        reader.SetCurrentMediaType(VIDEO_STREAM, None, &media_type)?;
    }
    Ok(())
}

fn read_source_size(media_type: &IMFMediaType) -> Result<SourceSize> {
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }
        .context("recording media type has no frame size")?;
    let source = SourceSize {
        width: (packed >> 32) as u32,
        height: packed as u32,
    };
    if source.valid() {
        Ok(source)
    } else {
        bail!("recording frame dimensions are invalid")
    }
}

fn read_frame_rate(media_type: &IMFMediaType) -> Option<FrameRate> {
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_RATE) }.ok()?;
    let numerator = (packed >> 32) as u32;
    let denominator = packed as u32;
    (numerator > 0 && denominator > 0).then_some(FrameRate {
        numerator,
        denominator,
    })
}

fn read_duration(reader: &IMFSourceReader) -> Option<u64> {
    let mut value = unsafe {
        reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
    }
    .ok()?;
    let duration = unsafe { PropVariantToInt64(&value) }.ok()?.max(0) as u64;
    let _ = unsafe { PropVariantClear(&mut value) };
    Some(duration)
}

#[cfg(test)]
#[path = "decoder_tests.rs"]
mod tests;
