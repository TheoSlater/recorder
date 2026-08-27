//! The canvas background image.
//!
//! Reloaded only when the project's choice changes. The editor preview asks for
//! it on every painted frame, so a path that fails to load has to be remembered
//! rather than retried sixty times a second.

use std::path::{Path, PathBuf};

use windows::Win32::Graphics::{
    Direct3D11::{
        D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT, ID3D11Device, ID3D11ShaderResourceView, ID3D11Texture2D,
    },
    Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC},
};

use super::super::super::RenderError;

type Result<T> = std::result::Result<T, RenderError>;

pub(super) struct ImageTexture {
    _texture: ID3D11Texture2D,
    pub(super) view: ID3D11ShaderResourceView,
    pub(super) width: u32,
    pub(super) height: u32,
}

#[derive(Default)]
pub(super) struct Background {
    texture: Option<ImageTexture>,
    loaded: Option<PathBuf>,
}

impl Background {
    /// Makes `path` the loaded image, doing nothing when it already is.
    pub(super) fn ensure(&mut self, device: &ID3D11Device, path: Option<&Path>) -> Result<()> {
        if self.loaded.as_deref() == path {
            return Ok(());
        }
        // Recorded before the load so a failure is not retried every frame.
        self.loaded = path.map(Path::to_path_buf);
        self.texture = None;
        let Some(path) = path else { return Ok(()) };
        self.texture = Some(load(device, path)?);
        Ok(())
    }

    pub(super) fn texture(&self) -> Option<&ImageTexture> {
        self.texture.as_ref()
    }
}

fn load(device: &ID3D11Device, path: &Path) -> Result<ImageTexture> {
    let bytes = std::fs::read(path).map_err(|error| {
        RenderError::Device(format!(
            "could not read background image {}: {error}",
            path.display()
        ))
    })?;
    let image = image::load_from_memory(&bytes)
        .map_err(|error| {
            RenderError::Device(format!(
                "could not decode background image {}: {error}",
                path.display()
            ))
        })?
        .to_rgba8();
    let width = image.width();
    let height = image.height();
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: image.as_ptr().cast(),
        SysMemPitch: width.saturating_mul(4),
        SysMemSlicePitch: 0,
    };
    let description = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&description, Some(&data), Some(&mut texture)) }.map_err(
        |error| RenderError::Device(format!("could not upload the canvas background: {error}")),
    )?;
    let texture =
        texture.ok_or_else(|| RenderError::Device("canvas background texture was null".into()))?;
    let mut view = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut view)) }.map_err(
        |error| RenderError::Device(format!("could not create the background view: {error}")),
    )?;
    let view = view.ok_or_else(|| RenderError::Device("canvas background view was null".into()))?;
    Ok(ImageTexture {
        _texture: texture,
        view,
        width,
        height,
    })
}
