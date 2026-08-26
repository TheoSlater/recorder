use std::{mem::ManuallyDrop, os::windows::ffi::OsStrExt, sync::Arc, time::Instant};

use anyhow::{Result, anyhow, bail};
use gpui::RenderImage;
use image::{Frame, ImageBuffer, Rgba};
use windows::{
    Win32::Media::MediaFoundation::{
        IMFSample, IMFSourceReader, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
        MF_MT_SUBTYPE, MF_PD_DURATION, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
        MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_ENDOFSTREAM, MFCreateMediaType,
        MFCreateSourceReaderFromURL, MFMediaType_Video, MFVideoFormat_NV12,
    },
    Win32::System::{
        Com::StructuredStorage::{
            PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0, PropVariantClear,
            PropVariantToInt64,
        },
        Variant::VT_I8,
    },
    core::{GUID, PCWSTR},
};

#[path = "native_decoder/conversion.rs"]
mod conversion;
use self::conversion::copy_nv12_buffer;

const VIDEO_STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const MEDIA_TIME_PER_SECOND: f64 = 10_000_000.0;

pub(super) struct Decoder {
    reader: IMFSourceReader,
    pub(super) width: u32,
    pub(super) height: u32,
    stride: i32,
    pub(super) duration: f64,
    pending_sample: Option<RawSample>,
    last_frame: Option<DecodedFrame>,
}

impl Decoder {
    pub(super) fn open(path: &std::path::Path) -> Result<Self> {
        let url: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let reader = unsafe { MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), None) }
            .map_err(|error| anyhow!("could not open recording: {error}"))?;
        let media_type = unsafe { MFCreateMediaType() }
            .map_err(|error| anyhow!("could not create video media type: {error}"))?;
        unsafe {
            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|error| anyhow!("could not set video media type major type: {error}"))?;
            media_type
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)
                .map_err(|error| anyhow!("could not set NV12 output subtype: {error}"))?;
            reader
                .SetCurrentMediaType(VIDEO_STREAM, None, &media_type)
                .map_err(|error| anyhow!("could not select NV12 output type: {error}"))?;
        }

        let current_type = unsafe { reader.GetCurrentMediaType(VIDEO_STREAM) }
            .map_err(|error| anyhow!("could not read video media type: {error}"))?;
        let packed_size = unsafe { current_type.GetUINT64(&MF_MT_FRAME_SIZE) }
            .map_err(|error| anyhow!("recording has no frame size: {error}"))?;
        let width = (packed_size >> 32) as u32;
        let height = packed_size as u32;
        if width == 0 || height == 0 {
            bail!("recording has an invalid frame size");
        }
        let row_bytes = width
            .checked_mul(4)
            .ok_or_else(|| anyhow!("recording frame is too wide"))?;
        let stride = unsafe { current_type.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
            .ok()
            .map(|value| i32::from_ne_bytes(value.to_ne_bytes()))
            .filter(|value| *value != 0)
            .unwrap_or(row_bytes as i32);
        let duration = read_duration(&reader)?;

        Ok(Self {
            reader,
            width,
            height,
            stride,
            duration,
            pending_sample: None,
            last_frame: None,
        })
    }

    pub(super) fn next_frame(&mut self) -> Result<Option<DecodedFrame>> {
        let Some(sample) = self.take_sample()? else {
            return Ok(None);
        };
        self.decode_sample(sample).map(Some)
    }

    /// Reads sequentially and converts only the newest sample already due on the media clock.
    /// A single future sample is retained so normal playback remains sequential.
    pub(super) fn next_frame_for_clock(
        &mut self,
        clock_seconds: f64,
    ) -> Result<(Option<DecodedFrame>, u64)> {
        let mut newest = None;
        let mut skipped = 0;

        loop {
            let Some(sample) = self.take_sample()? else {
                break;
            };
            if sample.seconds <= clock_seconds {
                if newest.is_some() {
                    skipped += 1;
                }
                newest = Some(sample);
            } else {
                self.pending_sample = Some(sample);
                break;
            }
        }

        let sample = match newest {
            Some(sample) => sample,
            None => {
                let Some(sample) = self.take_sample()? else {
                    return Ok((None, skipped));
                };
                sample
            }
        };
        Ok((Some(self.decode_sample(sample)?), skipped))
    }

    fn take_sample(&mut self) -> Result<Option<RawSample>> {
        if let Some(sample) = self.pending_sample.take() {
            return Ok(Some(sample));
        }

        loop {
            let mut stream_flags = 0;
            let mut timestamp = 0;
            let mut sample = None;
            let decode_started = Instant::now();
            unsafe {
                self.reader.ReadSample(
                    VIDEO_STREAM,
                    0,
                    None,
                    Some(&mut stream_flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )?;
            }
            let sample_ready_at = Instant::now();
            if stream_flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(None);
            }
            let Some(sample) = sample else {
                continue;
            };
            return Ok(Some(RawSample {
                sample,
                seconds: (timestamp.max(0) as f64 / MEDIA_TIME_PER_SECOND).min(self.duration),
                decode_time: decode_started.elapsed(),
                sample_ready_at,
            }));
        }
    }

    fn decode_sample(&mut self, sample: RawSample) -> Result<DecodedFrame> {
        self.decode_sample_if(sample, &|| true)?
            .ok_or_else(|| anyhow!("decoded frame was cancelled"))
    }

    fn decode_sample_if(
        &mut self,
        sample: RawSample,
        should_continue: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<DecodedFrame>> {
        let buffer_started = Instant::now();
        let buffer = unsafe { sample.sample.ConvertToContiguousBuffer() }?;
        let buffer_ready_at = Instant::now();
        let Some(converted) = copy_nv12_buffer(
            &buffer,
            self.width,
            self.height,
            self.stride,
            should_continue,
        )?
        else {
            return Ok(None);
        };
        if !should_continue() {
            return Ok(None);
        }
        let conversion_completed_at = Instant::now();
        let output_bytes = converted.pixels.len() as u64;
        let image_buffer_started = Instant::now();
        let image =
            ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(self.width, self.height, converted.pixels)
                .ok_or_else(|| anyhow!("decoded frame has an invalid buffer size"))?;
        let image_buffer_time = image_buffer_started.elapsed();
        let render_image_started = Instant::now();
        let image = Arc::new(RenderImage::new([Frame::new(image)]));
        let render_image_time = render_image_started.elapsed();
        let ready_at = Instant::now();
        let frame = DecodedFrame {
            seconds: sample.seconds,
            image,
            decode_time: sample.decode_time,
            buffer_copy_time: buffer_ready_at.saturating_duration_since(buffer_started),
            source_bytes: converted.source_bytes,
            output_bytes,
            allocation_time: converted.allocation_time,
            conversion_time: converted.conversion_time,
            image_time: ready_at.saturating_duration_since(conversion_completed_at),
            image_buffer_time,
            render_image_time,
            sample_ready_at: sample.sample_ready_at,
            buffer_ready_at,
            conversion_completed_at,
            ready_at,
        };
        self.last_frame = Some(frame.clone());
        Ok(Some(frame))
    }

    pub(super) fn seek(
        &mut self,
        seconds: f64,
        should_continue: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<DecodedFrame>> {
        self.pending_sample = None;
        let position = (seconds * MEDIA_TIME_PER_SECOND).round() as i64;
        let mut value = media_time(position);
        unsafe {
            self.reader.Flush(VIDEO_STREAM)?;
            // IMFSourceReader uses GUID_NULL to select its native 100-nanosecond
            // media-time format. TIME_FORMAT_MEDIA_TIME is a DirectShow format
            // identifier and is rejected by source readers with MF_E_UNSUPPORTED_BYTESTREAM_TYPE.
            let time_format = GUID::zeroed();
            let result = self.reader.SetCurrentPosition(&time_format, &value);
            let _ = PropVariantClear(&mut value);
            result?;
        }
        if !should_continue() {
            return Ok(None);
        }
        let Some(sample) = self.take_sample()? else {
            return Ok(self.last_frame.clone());
        };
        self.decode_sample_if(sample, should_continue)
    }
}

struct RawSample {
    sample: IMFSample,
    seconds: f64,
    decode_time: std::time::Duration,
    sample_ready_at: Instant,
}

#[derive(Clone)]
pub(super) struct DecodedFrame {
    pub(super) seconds: f64,
    pub(super) image: Arc<RenderImage>,
    pub(super) decode_time: std::time::Duration,
    pub(super) buffer_copy_time: std::time::Duration,
    pub(super) source_bytes: u64,
    pub(super) output_bytes: u64,
    pub(super) allocation_time: std::time::Duration,
    pub(super) conversion_time: std::time::Duration,
    pub(super) image_time: std::time::Duration,
    pub(super) image_buffer_time: std::time::Duration,
    pub(super) render_image_time: std::time::Duration,
    pub(super) sample_ready_at: Instant,
    pub(super) buffer_ready_at: Instant,
    pub(super) conversion_completed_at: Instant,
    pub(super) ready_at: Instant,
}

fn read_duration(reader: &IMFSourceReader) -> Result<f64> {
    let mut value = unsafe {
        reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)?
    };
    let duration = unsafe { PropVariantToInt64(&value) }
        .map_err(|error| anyhow!("recording duration is invalid: {error}"))?;
    unsafe { PropVariantClear(&mut value)? };
    Ok((duration.max(0) as f64 / MEDIA_TIME_PER_SECOND).max(0.0))
}

fn media_time(value: i64) -> PROPVARIANT {
    PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_I8,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 { hVal: value },
            }),
        },
    }
}
