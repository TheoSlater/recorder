use std::{
    slice,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use rayon::prelude::*;
use windows::Win32::Media::MediaFoundation::IMFMediaBuffer;

#[derive(Clone, Copy)]
struct Nv12Layout {
    width: usize,
    height: usize,
    plane_height: usize,
    stride: i32,
    source_stride: usize,
    row_bytes: usize,
}

const MAX_CONVERSION_WORKERS: usize = 8;
static CONVERSION_WORKERS: OnceLock<usize> = OnceLock::new();
static CONVERSION_POOL: OnceLock<Result<rayon::ThreadPool, String>> = OnceLock::new();

pub(super) struct ConvertedFrame {
    pub(super) pixels: Vec<u8>,
    pub(super) source_bytes: u64,
    pub(super) allocation_time: Duration,
    pub(super) conversion_time: Duration,
}

pub(super) fn copy_nv12_buffer(
    buffer: &IMFMediaBuffer,
    width: u32,
    height: u32,
    stride: i32,
    should_continue: &(dyn Fn() -> bool + Sync),
) -> Result<Option<ConvertedFrame>> {
    let width = width as usize;
    let height = height as usize;
    let source_stride = (stride.unsigned_abs() as usize).max(width);
    let expected_size = width
        .checked_mul(4)
        .and_then(|size| size.checked_mul(height))
        .ok_or_else(|| anyhow::anyhow!("decoded frame is too large"))?;
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
    let result = (|| -> Result<Option<ConvertedFrame>> {
        let source = unsafe { slice::from_raw_parts(pointer, current_length as usize) };
        let plane_height = nv12_plane_height(source.len(), source_stride, height);
        let uv_height = plane_height.div_ceil(2);
        let required_size = source_stride
            .checked_mul(plane_height)
            .and_then(|size| {
                source_stride
                    .checked_mul(uv_height)
                    .and_then(|uv_size| size.checked_add(uv_size))
            })
            .ok_or_else(|| anyhow::anyhow!("decoded frame stride is too large"))?;
        if source.len() < required_size {
            bail!("decoded frame buffer is truncated");
        }
        if !should_continue() {
            return Ok(None);
        }

        let allocation_started = Instant::now();
        let mut pixels = vec![0; expected_size];
        let allocation_time = allocation_started.elapsed();
        let row_bytes = width * 4;
        let worker_count = (*CONVERSION_WORKERS.get_or_init(|| {
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .min(MAX_CONVERSION_WORKERS)
        }))
        .min(height);
        let layout = Nv12Layout {
            width,
            height,
            plane_height,
            stride,
            source_stride,
            row_bytes,
        };
        let cancelled = AtomicBool::new(false);
        let conversion_started = Instant::now();
        if copy_nv12_rows_parallel(
            source,
            layout,
            worker_count,
            &mut pixels,
            should_continue,
            &cancelled,
        )? {
            Ok(None)
        } else {
            Ok(Some(ConvertedFrame {
                pixels,
                source_bytes: current_length as u64,
                allocation_time,
                conversion_time: conversion_started.elapsed(),
            }))
        }
    })();
    unsafe { buffer.Unlock()? };
    result
}

fn copy_nv12_rows_parallel(
    source: &[u8],
    layout: Nv12Layout,
    worker_count: usize,
    pixels: &mut [u8],
    should_continue: &(dyn Fn() -> bool + Sync),
    cancelled: &AtomicBool,
) -> Result<bool> {
    if layout.height == 0 {
        return Ok(false);
    }
    let worker_count = worker_count.max(1);
    let rows_per_worker = layout.height.div_ceil(worker_count);
    let chunk_bytes = rows_per_worker * layout.row_bytes;
    let pool = CONVERSION_POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(*CONVERSION_WORKERS.get_or_init(|| {
                std::thread::available_parallelism()
                    .map(usize::from)
                    .unwrap_or(1)
                    .min(MAX_CONVERSION_WORKERS)
            }))
            .thread_name(|index| format!("recorder-nv12-{index}"))
            .build()
            .map_err(|error| format!("could not start NV12 conversion pool: {error}"))
    });

    let pool = pool
        .as_ref()
        .map_err(|error| anyhow::anyhow!(error.clone()))?;

    pool.install(|| {
        pixels
            .par_chunks_mut(chunk_bytes)
            .enumerate()
            .for_each(|(chunk_index, worker_output)| {
                let row_start = chunk_index * rows_per_worker;
                let row_end = (row_start + rows_per_worker).min(layout.height);
                copy_nv12_rows(
                    source,
                    layout,
                    row_start,
                    row_end,
                    worker_output,
                    should_continue,
                    cancelled,
                );
            });
    });

    Ok(cancelled.load(Ordering::Acquire))
}

fn copy_nv12_rows(
    source: &[u8],
    layout: Nv12Layout,
    row_start: usize,
    row_end: usize,
    pixels: &mut [u8],
    should_continue: &(dyn Fn() -> bool + Sync),
    cancelled: &AtomicBool,
) {
    let Nv12Layout {
        width,
        height,
        plane_height,
        stride,
        source_stride,
        row_bytes,
    } = layout;
    for row in row_start..row_end {
        if !should_continue() {
            cancelled.store(true, Ordering::Release);
            return;
        }
        let source_row = if stride < 0 { height - row - 1 } else { row };
        let y_start = source_row * source_stride;
        let uv_start = source_stride * plane_height + (source_row / 2) * source_stride;
        let target_start = (row - row_start) * row_bytes;
        let mut column = 0;
        while column + 1 < width {
            let y = source[y_start + column];
            let chroma = uv_start + (column / 2) * 2;
            let u = source[chroma];
            let v = source[(chroma + 1).min(uv_start + source_stride - 1)];
            let y_next = source[y_start + column + 1];
            let target = target_start + column * 4;
            write_bgra(pixels, target, y, u, v);
            write_bgra(pixels, target + 4, y_next, u, v);
            column += 2;
        }
        if column < width {
            let chroma = uv_start + (column / 2) * 2;
            let u = source[chroma];
            let v = source[(chroma + 1).min(uv_start + source_stride - 1)];
            write_bgra(
                pixels,
                target_start + column * 4,
                source[y_start + column],
                u,
                v,
            );
        }
    }
}

/// NV12 decoders may align the luma plane to a macroblock height while keeping
/// the visible frame height in the media type. The chroma plane starts after
/// that aligned height, not after the visible rows.
fn nv12_plane_height(buffer_len: usize, source_stride: usize, visible_height: usize) -> usize {
    if source_stride == 0 {
        return visible_height;
    }

    let total_rows = buffer_len / source_stride;
    let candidate = total_rows.saturating_mul(2) / 3;
    if candidate >= visible_height && candidate + candidate.div_ceil(2) == total_rows {
        candidate
    } else {
        visible_height
    }
}

#[inline]
fn write_bgra(pixels: &mut [u8], target: usize, y: u8, u: u8, v: u8) {
    let y = Y_TERMS[usize::from(y)];
    let red = (y + V_RED_TERMS[usize::from(v)] + 128) >> 8;
    let green = (y + U_GREEN_TERMS[usize::from(u)] + V_GREEN_TERMS[usize::from(v)] + 128) >> 8;
    let blue = (y + U_BLUE_TERMS[usize::from(u)] + 128) >> 8;
    pixels[target] = clamp_byte(blue);
    pixels[target + 1] = clamp_byte(green);
    pixels[target + 2] = clamp_byte(red);
    pixels[target + 3] = u8::MAX;
}

fn clamp_byte(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

const fn y_terms() -> [i32; 256] {
    let mut terms = [0; 256];
    let mut value = 0;
    while value < terms.len() {
        let y = if value > 16 { value as i32 - 16 } else { 0 };
        terms[value] = 298 * y;
        value += 1;
    }
    terms
}

const fn u_green_terms() -> [i32; 256] {
    let mut terms = [0; 256];
    let mut value = 0;
    while value < terms.len() {
        terms[value] = -100 * (value as i32 - 128);
        value += 1;
    }
    terms
}

const fn u_blue_terms() -> [i32; 256] {
    let mut terms = [0; 256];
    let mut value = 0;
    while value < terms.len() {
        terms[value] = 516 * (value as i32 - 128);
        value += 1;
    }
    terms
}

const fn v_red_terms() -> [i32; 256] {
    let mut terms = [0; 256];
    let mut value = 0;
    while value < terms.len() {
        terms[value] = 409 * (value as i32 - 128);
        value += 1;
    }
    terms
}

const fn v_green_terms() -> [i32; 256] {
    let mut terms = [0; 256];
    let mut value = 0;
    while value < terms.len() {
        terms[value] = -208 * (value as i32 - 128);
        value += 1;
    }
    terms
}

static Y_TERMS: [i32; 256] = y_terms();
static U_GREEN_TERMS: [i32; 256] = u_green_terms();
static U_BLUE_TERMS: [i32; 256] = u_blue_terms();
static V_RED_TERMS: [i32; 256] = v_red_terms();
static V_GREEN_TERMS: [i32; 256] = v_green_terms();

#[cfg(test)]
#[path = "conversion_tests.rs"]
mod tests;
