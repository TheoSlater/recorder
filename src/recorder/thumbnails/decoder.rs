use std::{
    mem::ManuallyDrop,
    os::windows::ffi::OsStrExt,
    path::Path,
    slice,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use gpui::RenderImage;
use image::{Frame, ImageBuffer, Rgba, imageops::FilterType};
use windows::{
    Win32::Media::MediaFoundation::{
        IMFSample, IMFSourceReader, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
        MF_MT_SUBTYPE, MF_PD_DURATION, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
        MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_ENDOFSTREAM, MFCreateMediaType,
        MFCreateSourceReaderFromURL, MFMediaType_Video, MFVideoFormat_RGB32,
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

const VIDEO_STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const MEDIA_TIME_PER_SECOND: f64 = 10_000_000.;

pub(super) struct Decoder {
    reader: IMFSourceReader,
    pub(super) width: u32,
    pub(super) height: u32,
    stride: i32,
    pub(super) duration: f64,
}

pub(super) struct ExtractedFrame {
    pub(super) image: Arc<RenderImage>,
    pub(super) decode_time: Duration,
    pub(super) resize_time: Duration,
}

impl Decoder {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let url: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let reader = unsafe { MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), None) }
            .map_err(|error| anyhow!("could not open thumbnail source: {error}"))?;
        let media_type = unsafe { MFCreateMediaType() }
            .map_err(|error| anyhow!("could not create thumbnail media type: {error}"))?;
        unsafe {
            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|error| anyhow!("could not set thumbnail major type: {error}"))?;
            media_type
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                .map_err(|error| anyhow!("could not set RGB32 output type: {error}"))?;
            reader
                .SetCurrentMediaType(VIDEO_STREAM, None, &media_type)
                .map_err(|error| anyhow!("could not select RGB32 output type: {error}"))?;
        }
        let current_type = unsafe { reader.GetCurrentMediaType(VIDEO_STREAM) }
            .map_err(|error| anyhow!("could not read thumbnail media type: {error}"))?;
        let packed_size = unsafe { current_type.GetUINT64(&MF_MT_FRAME_SIZE) }
            .map_err(|error| anyhow!("thumbnail source has no frame size: {error}"))?;
        let width = (packed_size >> 32) as u32;
        let height = packed_size as u32;
        if width == 0 || height == 0 {
            bail!("thumbnail source has an invalid frame size");
        }
        let row_bytes = width
            .checked_mul(4)
            .ok_or_else(|| anyhow!("thumbnail source is too wide"))?;
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
        })
    }

    pub(super) fn extract(
        &mut self,
        timestamp_us: u64,
        size: super::layout::ThumbnailSize,
        should_continue: &dyn Fn() -> bool,
    ) -> Result<Option<ExtractedFrame>> {
        if !should_continue() {
            return Ok(None);
        }
        let decode_started = Instant::now();
        self.seek(timestamp_us)?;
        let Some(sample) = self.read_sample(should_continue)? else {
            return Ok(None);
        };
        let buffer = unsafe { sample.ConvertToContiguousBuffer() }?;
        let pixels = copy_rgb32_buffer(&buffer, self.width, self.height, self.stride)?;
        if !should_continue() {
            return Ok(None);
        }
        let source = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(self.width, self.height, pixels)
            .ok_or_else(|| anyhow!("thumbnail frame has an invalid buffer size"))?;
        let decode_time = decode_started.elapsed();
        let resize_started = Instant::now();
        let resized =
            image::imageops::resize(&source, size.width, size.height, FilterType::Triangle);
        let resize_time = resize_started.elapsed();
        if !should_continue() {
            return Ok(None);
        }
        Ok(Some(ExtractedFrame {
            image: Arc::new(RenderImage::new([Frame::new(resized)])),
            decode_time,
            resize_time,
        }))
    }

    fn seek(&mut self, timestamp_us: u64) -> Result<()> {
        let media_time = ((timestamp_us as f64 / 1_000_000.) * MEDIA_TIME_PER_SECOND)
            .round()
            .clamp(0., i64::MAX as f64) as i64;
        let mut value = media_time_value(media_time);
        unsafe {
            self.reader.Flush(VIDEO_STREAM)?;
            let result = self.reader.SetCurrentPosition(&GUID::zeroed(), &value);
            let _ = PropVariantClear(&mut value);
            result?;
        }
        Ok(())
    }

    fn read_sample(&mut self, should_continue: &dyn Fn() -> bool) -> Result<Option<IMFSample>> {
        loop {
            if !should_continue() {
                return Ok(None);
            }
            let mut stream_flags = 0;
            let mut timestamp = 0;
            let mut sample = None;
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
            if stream_flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(None);
            }
            if sample.is_some() {
                return Ok(sample);
            }
        }
    }
}

fn copy_rgb32_buffer(
    buffer: &windows::Win32::Media::MediaFoundation::IMFMediaBuffer,
    width: u32,
    height: u32,
    stride: i32,
) -> Result<Vec<u8>> {
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| anyhow!("thumbnail row is too wide"))? as usize;
    let source_stride = stride.unsigned_abs().max(row_bytes as u32) as usize;
    let required = source_stride
        .checked_mul(height as usize)
        .ok_or_else(|| anyhow!("thumbnail buffer is too large"))?;
    let mut pointer = std::ptr::null_mut();
    let mut max_length = 0;
    let mut current_length = 0;
    unsafe {
        buffer.Lock(
            &mut pointer,
            Some(&mut max_length),
            Some(&mut current_length),
        )?;
    }
    let result = (|| -> Result<Vec<u8>> {
        if (current_length as usize) < required {
            bail!("thumbnail RGB32 buffer is truncated");
        }
        let source = unsafe { slice::from_raw_parts(pointer, current_length as usize) };
        let output_len = row_bytes
            .checked_mul(height as usize)
            .ok_or_else(|| anyhow!("thumbnail output is too large"))?;
        let mut output = vec![0; output_len];
        for row in 0..height as usize {
            let source_row = if stride < 0 {
                height as usize - row - 1
            } else {
                row
            };
            let source_start = source_row * source_stride;
            let target_start = row * row_bytes;
            output[target_start..target_start + row_bytes]
                .copy_from_slice(&source[source_start..source_start + row_bytes]);
            for alpha in output[target_start + 3..target_start + row_bytes]
                .iter_mut()
                .step_by(4)
            {
                *alpha = u8::MAX;
            }
        }
        Ok(output)
    })();
    unsafe { buffer.Unlock()? };
    result
}

fn read_duration(reader: &IMFSourceReader) -> Result<f64> {
    let mut value = unsafe {
        reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)?
    };
    let duration = unsafe { PropVariantToInt64(&value) }
        .map_err(|error| anyhow!("thumbnail duration is invalid: {error}"))?;
    unsafe { PropVariantClear(&mut value)? };
    Ok((duration.max(0) as f64 / MEDIA_TIME_PER_SECOND).max(0.))
}

fn media_time_value(value: i64) -> PROPVARIANT {
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
