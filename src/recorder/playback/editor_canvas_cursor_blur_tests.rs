// Imported one by one: `gpui::*` exports its own `test` attribute macro,
// which shadows the standard one behind a glob import.
use gpui::{RenderImage, px, size};
use image::{Frame, ImageBuffer, Rgba};

use super::{CHANNELS, MAX_TAPS, MIN_TAPS, build, resample, tap_count};
use crate::recorder::motion_blur::Vec2;

/// A fully opaque square, the simplest shape whose smear is easy to reason
/// about: a box filter must spread its alpha along the motion and conserve
/// the total.
fn opaque_sprite(extent: u32) -> RenderImage {
    let pixels = vec![255u8; (extent * extent) as usize * CHANNELS];
    let buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(extent, extent, pixels)
        .expect("buffer matches its dimensions");
    RenderImage::new([Frame::new(buffer)])
}

fn alpha_total(image: &RenderImage) -> u32 {
    image
        .as_bytes(0)
        .expect("frame zero exists")
        .chunks_exact(CHANNELS)
        .map(|pixel| u32::from(pixel[3]))
        .sum()
}

#[test]
fn keeps_short_smears_at_the_baseline_tap_count() {
    assert_eq!(tap_count(0.0, 24), MIN_TAPS);
    assert_eq!(tap_count(4.0, 24), MIN_TAPS);
}

#[test]
fn densifies_taps_for_long_smears() {
    // A 24 px sprite spaces taps 3 px apart, so a 150 px smear needs 51.
    assert_eq!(tap_count(150.0, 24), 51);
    assert_eq!(tap_count(4000.0, 24), MAX_TAPS);
}

#[test]
fn scales_tap_spacing_with_sprite_size() {
    // A larger sprite tolerates wider spacing without showing silhouettes.
    assert!(tap_count(150.0, 80) < tap_count(150.0, 24));
}

#[test]
fn pads_the_sprite_along_the_motion() {
    let base = opaque_sprite(4);
    let blurred = build(&base, size(px(4.), px(4.)), Vec2::new(20.0, 0.0), 1.0)
        .expect("sprite is small enough to build");

    assert_eq!(blurred.size.width, px(24.));
    assert_eq!(blurred.size.height, px(4.));
    // Travelling right means the tail extends left, so the sharp cursor
    // sits at the far right of the padded sprite.
    assert_eq!(blurred.offset.x, px(20.));
    assert_eq!(blurred.offset.y, px(0.));
}

#[test]
fn spreads_alpha_instead_of_repeating_the_sprite() {
    let base = opaque_sprite(4);
    let blurred = build(&base, size(px(4.), px(4.)), Vec2::new(20.0, 0.0), 1.0)
        .expect("sprite is small enough to build");
    let smeared = blurred.image.as_bytes(0).expect("frame zero exists");

    // A box filter conserves alpha: the smear is dimmer over a longer run,
    // never 21 opaque copies.
    let source_total = alpha_total(&base);
    let smeared_total = alpha_total(&blurred.image);
    assert!(
        smeared_total.abs_diff(source_total) * 20 < source_total,
        "alpha {smeared_total} drifted from {source_total}"
    );
    assert!(
        smeared.chunks_exact(CHANNELS).all(|pixel| pixel[3] < 255),
        "no pixel should stay fully opaque"
    );
    // The tail reaches the far end of the padded sprite.
    assert!(smeared[3] > 0, "the trailing edge should carry the smear");
}

#[test]
fn smears_backwards_for_negative_motion() {
    let base = opaque_sprite(4);
    let blurred = build(&base, size(px(4.), px(4.)), Vec2::new(0.0, -12.0), 1.0)
        .expect("sprite is small enough to build");

    assert_eq!(blurred.size.height, px(16.));
    // Travelling up leaves the head at the top and the tail below it.
    assert_eq!(blurred.offset.y, px(0.));
}

#[test]
fn declines_pathological_sprites() {
    let base = opaque_sprite(4);
    assert!(build(&base, size(px(0.), px(0.)), Vec2::new(4.0, 0.0), 1.0).is_none());
    assert!(build(&base, size(px(4.), px(4.)), Vec2::new(f32::NAN, 0.0), 1.0).is_none());
}

#[test]
fn resamples_to_the_requested_size() {
    let source = vec![255u8; 4 * 4 * 4];
    let resampled = resample(&source, 4, 4, 8, 8);

    assert_eq!(resampled.len(), 8 * 8 * 4);
    assert!(resampled.iter().all(|channel| *channel == 255));
}
