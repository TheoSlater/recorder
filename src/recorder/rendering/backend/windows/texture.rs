//! A stand-in picture for renderer bring-up.
//!
//! Step 4 of the preview migration proves that a GPU texture reaches the screen
//! through our own surface — pipeline, sampler, constant buffer, render target,
//! and present — before the Media Foundation decoder is wired in. Generating
//! the pixels here keeps that milestone independent of decoding, so a failure
//! is unambiguous.
//!
//! It is deliberately asymmetric: a checkerboard alone cannot reveal a flipped
//! or transposed sample, so one corner is marked.

use windows::Win32::Graphics::{
    Direct3D11::{
        D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT, ID3D11Device, ID3D11ShaderResourceView, ID3D11Texture2D,
    },
    Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
};

use super::super::super::RenderError;

const WIDTH: u32 = 256;
const HEIGHT: u32 = 144;
const CELL: u32 = 16;

/// A texture and the view the pixel shader samples it through.
pub(super) struct StaticTexture {
    pub(super) _texture: ID3D11Texture2D,
    pub(super) view: ID3D11ShaderResourceView,
}

pub(super) fn placeholder(device: &ID3D11Device) -> Result<StaticTexture, RenderError> {
    let pixels = checkerboard();
    let description = D3D11_TEXTURE2D_DESC {
        Width: WIDTH,
        Height: HEIGHT,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr().cast(),
        SysMemPitch: WIDTH * 4,
        SysMemSlicePitch: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&description, Some(&data), Some(&mut texture)) }
        .map_err(|error| RenderError::Device(format!("could not create a texture: {error}")))?;
    let texture = texture.ok_or_else(|| RenderError::Device("texture was null".into()))?;

    let mut view = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut view)) }
        .map_err(|error| RenderError::Device(format!("could not create a view: {error}")))?;
    let view = view.ok_or_else(|| RenderError::Device("texture view was null".into()))?;

    Ok(StaticTexture {
        _texture: texture,
        view,
    })
}

/// BGRA checkerboard with a red top-left corner, so a flipped or transposed
/// sample is obvious rather than plausible.
fn checkerboard() -> Vec<u8> {
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let marker = x < CELL * 2 && y < CELL;
            let light = ((x / CELL) + (y / CELL)).is_multiple_of(2);
            let (b, g, r) = match (marker, light) {
                (true, _) => (0x30, 0x30, 0xE0),
                (_, true) => (0xC8, 0xC8, 0xC8),
                (_, false) => (0x40, 0x40, 0x40),
            };
            pixels.extend_from_slice(&[b, g, r, 0xFF]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::{CELL, HEIGHT, WIDTH, checkerboard};

    #[test]
    fn builds_a_full_bgra_surface() {
        let pixels = checkerboard();

        assert_eq!(pixels.len(), (WIDTH * HEIGHT * 4) as usize);
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 0xFF));
    }

    #[test]
    fn marks_the_top_left_corner() {
        let pixels = checkerboard();
        let at = |x: u32, y: u32| {
            let index = ((y * WIDTH + x) * 4) as usize;
            (pixels[index], pixels[index + 1], pixels[index + 2])
        };

        // Red in BGRA is a low blue and green with a high red.
        let (b, g, r) = at(0, 0);
        assert!(r > b && r > g, "corner should be red: {b},{g},{r}");
        // The opposite corner is ordinary checkerboard, so orientation shows.
        assert_ne!(at(WIDTH - 1, HEIGHT - 1), at(0, 0));
        assert_ne!(at(CELL * 3, 0), at(0, 0));
    }
}
