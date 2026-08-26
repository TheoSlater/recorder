use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use windows::{
    Win32::Media::MediaFoundation::{
        IMFAttributes, IMFMediaType, IMFSample, IMFSinkWriter,
        MFCreateAttributes, MFCreateDXGISurfaceBuffer, MFCreateMediaType, MFCreateSample,
        MFCreateSinkWriterFromURL, MFMediaType_Video, MF_MT_ALL_SAMPLES_INDEPENDENT,
        MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
        MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
        MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_D3D_MANAGER,
        MFVideoFormat_ARGB32, MFVideoFormat_H264, MFVideoInterlace_Progressive,
    },
    core::{Interface, PCWSTR},
    Win32::Graphics::Direct3D11::ID3D11Texture2D,
};

use super::decoder::{DeviceContext, FrameRate};

const MEDIA_TIME_PER_SECOND: u64 = 10_000_000;

pub(crate) struct Encoder {
    writer: IMFSinkWriter,
    stream: u32,
    frame_duration_100ns: i64,
}

impl Encoder {
    pub(crate) fn open(
        path: &Path,
        device: &DeviceContext,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
    ) -> Result<Self> {
        let mut attributes = None;
        unsafe { MFCreateAttributes(&mut attributes, 2) }
            .context("could not create writer attributes")?;
        let attributes: IMFAttributes =
            attributes.ok_or_else(|| anyhow!("writer attributes are null"))?;
        unsafe {
            attributes.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &device.manager)?;
            attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        }

        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let writer = unsafe { MFCreateSinkWriterFromURL(PCWSTR(path.as_ptr()), None, &attributes) }
            .context("could not create MP4 sink writer")?;
        let output = media_type(MFVideoFormat_H264, width, height, frame_rate)?;
        unsafe {
            output.SetUINT32(&MF_MT_AVG_BITRATE, bitrate(width, height, frame_rate))?;
        }
        let stream = unsafe { writer.AddStream(&output) }
            .context("could not add H.264 output stream")?;
        let input = media_type(MFVideoFormat_ARGB32, width, height, frame_rate)?;
        unsafe {
            writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>)?;
            writer.BeginWriting()?;
        }
        let frame_duration_100ns = (u128::from(MEDIA_TIME_PER_SECOND)
            * u128::from(frame_rate.denominator)
            / u128::from(frame_rate.numerator)) as i64;
        Ok(Self {
            writer,
            stream,
            frame_duration_100ns,
        })
    }

    pub(crate) fn write(
        &self,
        texture: &ID3D11Texture2D,
        timestamp_100ns: u64,
    ) -> Result<()> {
        let buffer = unsafe {
            MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, texture, 0, false)
        }
        .context("could not wrap rendered D3D11 frame")?;
        let sample: IMFSample = unsafe { MFCreateSample() }
            .context("could not create output sample")?;
        unsafe {
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(timestamp_100ns.min(i64::MAX as u64) as i64)?;
            sample.SetSampleDuration(self.frame_duration_100ns)?;
            self.writer.WriteSample(self.stream, &sample)?;
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<()> {
        unsafe { self.writer.Finalize() }.context("could not finalize exported MP4")
    }
}

fn media_type(
    subtype: windows::core::GUID,
    width: u32,
    height: u32,
    frame_rate: FrameRate,
) -> Result<IMFMediaType> {
    let media_type = unsafe { MFCreateMediaType() }.context("could not create output media type")?;
    unsafe {
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media_type.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
        media_type.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        media_type.SetUINT64(
            &MF_MT_FRAME_RATE,
            pack(frame_rate.numerator, frame_rate.denominator),
        )?;
        media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        media_type.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
    }
    Ok(media_type)
}

fn pack(high: u32, low: u32) -> u64 {
    u64::from(high) << 32 | u64::from(low)
}

fn bitrate(width: u32, height: u32, frame_rate: FrameRate) -> u32 {
    let pixels_per_second = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(u64::from(frame_rate.numerator))
        / u64::from(frame_rate.denominator.max(1));
    pixels_per_second
        .saturating_mul(8)
        .clamp(2_000_000, 40_000_000)
        .min(u64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::{bitrate, pack};
    use crate::recorder::export::decoder::FrameRate;

    #[test]
    fn packs_media_foundation_ratio() {
        assert_eq!(pack(60, 1), 0x0000_003c_0000_0001);
    }

    #[test]
    fn bitrate_stays_in_encoder_range() {
        let rate = bitrate(1920, 1080, FrameRate { numerator: 60, denominator: 1 });
        assert!((2_000_000..=40_000_000).contains(&rate));
    }
}
