//! Builds the smeared cursor sprite.
//!
//! The sprite is a genuine directional convolution: the cursor is resampled to
//! its rendered size once, then accumulated along the motion vector and divided
//! by the tap count. Because GPUI images are premultiplied BGRA, averaging the
//! channels directly is the correct compositing operation, so transparency
//! around the cursor survives and no halo appears.
//!
//! The result replaces the sharp sprite rather than being layered over it — a
//! sharp copy under a blurred one would read as a stationary ghost.

use std::sync::Arc;

use gpui::*;
use image::{Frame, ImageBuffer, Rgba};

use super::super::motion_blur::Vec2;

/// The spec's tap count, used whenever the smear is short enough that denser
/// sampling would not change the result.
const MIN_TAPS: u32 = 21;

/// Beyond this the taps overlap so heavily that more of them cost time without
/// changing a pixel.
const MAX_TAPS: u32 = 96;

/// Taps must land closer together than this fraction of the sprite's smaller
/// dimension. Spacing relative to the sprite — not an absolute pixel count — is
/// what prevents visible duplicate cursor silhouettes in a long smear.
const TAP_OVERLAP: f32 = 8.0;

/// Refuse to allocate a pathological sprite. The motion clamp keeps normal
/// playback far below this; blowing it means falling back to a sharp cursor,
/// which is always better than stalling the preview.
const MAX_SPRITE_PIXELS: usize = 1_200_000;

const CHANNELS: usize = 4;

/// A smeared cursor sprite, positioned by the offset of the sharp cursor's
/// top-left corner inside the padded image.
#[derive(Clone)]
pub(super) struct BlurredCursor {
    pub(super) image: Arc<RenderImage>,
    pub(super) size: Size<Pixels>,
    pub(super) offset: Point<Pixels>,
}

/// Smears `base` along `motion`.
///
/// `rendered` is the size the sharp cursor would occupy and `motion` is its
/// travel over the last presented frame, both in logical pixels. Returns `None`
/// when the sprite cannot be built, in which case the caller draws the cursor
/// sharp.
pub(super) fn build(
    base: &RenderImage,
    rendered: Size<Pixels>,
    motion: Vec2,
    scale_factor: f32,
) -> Option<BlurredCursor> {
    let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let source = base.as_bytes(0)?;
    let source_size = base.size(0);
    let source_width = source_size.width.0.max(0) as usize;
    let source_height = source_size.height.0.max(0) as usize;
    if source_width == 0
        || source_height == 0
        || source.len() < source_width * source_height * CHANNELS
    {
        return None;
    }

    let sprite_width = device_pixels(rendered.width.as_f32(), scale_factor)?;
    let sprite_height = device_pixels(rendered.height.as_f32(), scale_factor)?;
    let motion = Vec2::new(motion.x * scale_factor, motion.y * scale_factor);
    if !motion.is_finite() {
        return None;
    }

    let pad_x = motion.x.abs().ceil() as usize;
    let pad_y = motion.y.abs().ceil() as usize;
    let output_width = sprite_width + pad_x;
    let output_height = sprite_height + pad_y;
    if output_width * output_height > MAX_SPRITE_PIXELS {
        return None;
    }

    // The smear trails the cursor: the head sits at the current position and
    // the tail runs back towards where the cursor came from, so padding is
    // added on the side the cursor travelled from.
    let head_x = if motion.x > 0.0 { pad_x } else { 0 };
    let head_y = if motion.y > 0.0 { pad_y } else { 0 };

    let sprite = resample(
        source,
        source_width,
        source_height,
        sprite_width,
        sprite_height,
    );
    let taps = tap_count(motion.length(), sprite_width.min(sprite_height));
    let mut accumulator = vec![0u32; output_width * output_height * CHANNELS];
    for tap in 0..taps {
        let progress = tap as f32 / (taps - 1).max(1) as f32;
        let x = head_x as f32 - motion.x * progress;
        let y = head_y as f32 - motion.y * progress;
        accumulate(
            &mut accumulator,
            output_width,
            output_height,
            &sprite,
            sprite_width,
            sprite_height,
            x.round() as isize,
            y.round() as isize,
        );
    }

    let pixels = accumulator
        .into_iter()
        .map(|total| (total / taps).min(u8::MAX as u32) as u8)
        .collect();
    let buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(
        output_width as u32,
        output_height as u32,
        pixels,
    )?;

    Some(BlurredCursor {
        image: Arc::new(RenderImage::new([Frame::new(buffer)])),
        size: size(
            px(output_width as f32 / scale_factor),
            px(output_height as f32 / scale_factor),
        ),
        offset: point(
            px(head_x as f32 / scale_factor),
            px(head_y as f32 / scale_factor),
        ),
    })
}

/// Picks a tap count dense enough that consecutive copies overlap instead of
/// reading as separate cursors.
fn tap_count(length: f32, sprite_extent: usize) -> u32 {
    let spacing = (sprite_extent as f32 / TAP_OVERLAP).max(1.0);
    let needed = (length / spacing).ceil() as u32 + 1;
    needed.clamp(MIN_TAPS, MAX_TAPS)
}

fn device_pixels(logical: f32, scale_factor: f32) -> Option<usize> {
    let pixels = (logical * scale_factor).round();
    (pixels.is_finite() && pixels >= 1.0).then_some(pixels as usize)
}

/// Bilinear resample into the sprite's rendered size, in premultiplied BGRA.
fn resample(
    source: &[u8],
    source_width: usize,
    source_height: usize,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let mut output = vec![0u8; width * height * CHANNELS];
    let ratio_x = source_width as f32 / width as f32;
    let ratio_y = source_height as f32 / height as f32;
    for y in 0..height {
        let source_y = (((y as f32 + 0.5) * ratio_y) - 0.5).clamp(0.0, (source_height - 1) as f32);
        let y0 = source_y.floor() as usize;
        let y1 = (y0 + 1).min(source_height - 1);
        let weight_y = source_y - y0 as f32;
        for x in 0..width {
            let source_x =
                (((x as f32 + 0.5) * ratio_x) - 0.5).clamp(0.0, (source_width - 1) as f32);
            let x0 = source_x.floor() as usize;
            let x1 = (x0 + 1).min(source_width - 1);
            let weight_x = source_x - x0 as f32;
            let corners = [
                (y0 * source_width + x0) * CHANNELS,
                (y0 * source_width + x1) * CHANNELS,
                (y1 * source_width + x0) * CHANNELS,
                (y1 * source_width + x1) * CHANNELS,
            ];
            let weights = [
                (1.0 - weight_x) * (1.0 - weight_y),
                weight_x * (1.0 - weight_y),
                (1.0 - weight_x) * weight_y,
                weight_x * weight_y,
            ];
            let target = (y * width + x) * CHANNELS;
            for channel in 0..CHANNELS {
                let value: f32 = corners
                    .iter()
                    .zip(weights)
                    .map(|(corner, weight)| source[corner + channel] as f32 * weight)
                    .sum();
                output[target + channel] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    output
}

/// Adds one tap of the sprite into the accumulator at an integer offset.
///
/// Taps are spaced at least a pixel apart and overlap many times over, so
/// rounding each to the pixel grid stays below the visible threshold while
/// keeping the inner loop to integer adds.
#[allow(clippy::too_many_arguments)]
fn accumulate(
    accumulator: &mut [u32],
    output_width: usize,
    output_height: usize,
    sprite: &[u8],
    sprite_width: usize,
    sprite_height: usize,
    offset_x: isize,
    offset_y: isize,
) {
    for y in 0..sprite_height {
        let target_y = offset_y + y as isize;
        if target_y < 0 || target_y >= output_height as isize {
            continue;
        }
        let row = target_y as usize * output_width;
        for x in 0..sprite_width {
            let target_x = offset_x + x as isize;
            if target_x < 0 || target_x >= output_width as isize {
                continue;
            }
            let source = (y * sprite_width + x) * CHANNELS;
            let target = (row + target_x as usize) * CHANNELS;
            for channel in 0..CHANNELS {
                accumulator[target + channel] += sprite[source + channel] as u32;
            }
        }
    }
}

#[cfg(test)]
#[path = "editor_canvas_cursor_blur_tests.rs"]
mod tests;
