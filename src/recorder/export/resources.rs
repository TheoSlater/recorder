use std::path::Path;

use anyhow::{Context, Result, anyhow};
use windows::{
    Win32::Graphics::{
        Direct3D::{Fxc::D3DCompile, ID3DBlob, ID3DInclude},
        Direct3D11::{
            D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_SHADER_RESOURCE, D3D11_BUFFER_DESC,
            D3D11_CPU_ACCESS_WRITE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
            D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC, ID3D11ClassLinkage, ID3D11Device,
            ID3D11PixelShader, ID3D11Texture2D, ID3D11VertexShader,
        },
        Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC},
    },
    core::PCSTR,
};

use super::super::{composition, composition::NormalizedRect, cursor_settings::CursorStyle};
use super::{
    renderer::{Constants, ImageTexture},
    shaders,
};

pub(super) fn create_output(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D> {
    let description = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (windows::Win32::Graphics::Direct3D11::D3D11_BIND_RENDER_TARGET
            | D3D11_BIND_SHADER_RESOURCE)
            .0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&description, None, Some(&mut texture)) }
        .context("could not create export render texture")?;
    texture.ok_or_else(|| anyhow!("export render texture was null"))
}

pub(crate) fn create_constants(
    device: &ID3D11Device,
) -> Result<windows::Win32::Graphics::Direct3D11::ID3D11Buffer> {
    let description = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of::<Constants>() as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buffer = None;
    unsafe { device.CreateBuffer(&description, None, Some(&mut buffer)) }
        .context("could not create export constants")?;
    buffer.ok_or_else(|| anyhow!("export constants buffer was null"))
}

pub(crate) fn create_vertex_shader(device: &ID3D11Device) -> Result<ID3D11VertexShader> {
    let code = compile(&shaders::vertex(), b"vs_5_0\0")?;
    let mut shader = None;
    unsafe { device.CreateVertexShader(&code, None::<&ID3D11ClassLinkage>, Some(&mut shader)) }
        .context("could not create export vertex shader")?;
    shader.ok_or_else(|| anyhow!("export vertex shader was null"))
}

pub(crate) fn create_pixel_shader(
    device: &ID3D11Device,
    source: &str,
) -> Result<ID3D11PixelShader> {
    let code = compile(source, b"ps_5_0\0")?;
    let mut shader = None;
    unsafe { device.CreatePixelShader(&code, None::<&ID3D11ClassLinkage>, Some(&mut shader)) }
        .context("could not create export pixel shader")?;
    shader.ok_or_else(|| anyhow!("export pixel shader was null"))
}

fn compile(source: &str, target: &[u8]) -> Result<Vec<u8>> {
    let mut blob: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
            None,
            None::<&ID3DInclude>,
            PCSTR(c"main".as_ptr().cast()),
            PCSTR(target.as_ptr()),
            0,
            0,
            &mut blob,
            Some(&mut errors),
        )
    }
    .map_err(|error| {
        anyhow!(
            "could not compile export shader: {error}{}",
            blob_text(errors)
        )
    })?;
    let blob = blob.ok_or_else(|| anyhow!("shader compiler returned no bytecode"))?;
    unsafe {
        Ok(
            std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize())
                .to_vec(),
        )
    }
}

/// The compiler's own diagnostics. Without them a shader failure reports only
/// a generic HRESULT, which says nothing about which line is wrong.
fn blob_text(blob: Option<ID3DBlob>) -> String {
    let Some(blob) = blob else {
        return String::new();
    };
    let text = unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
    };
    let text = String::from_utf8_lossy(text);
    let text = text.trim_end_matches('\0').trim();
    if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
    }
}

pub(super) fn load_image(device: &ID3D11Device, path: &Path) -> Result<ImageTexture> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("could not read background image {}", path.display()))?;
    let image = image::load_from_memory(&bytes)
        .with_context(|| format!("could not decode background image {}", path.display()))?
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
    unsafe { device.CreateTexture2D(&description, Some(&data), Some(&mut texture)) }
        .context("could not upload canvas background")?;
    let texture = texture.ok_or_else(|| anyhow!("canvas background texture was null"))?;
    let mut view = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut view)) }
        .context("could not create canvas background view")?;
    let view = view.ok_or_else(|| anyhow!("canvas background view was null"))?;
    Ok(ImageTexture {
        _texture: texture,
        view,
        width,
        height,
    })
}

pub(super) fn cover_rect(
    width: u32,
    height: u32,
    image_width: u32,
    image_height: u32,
) -> NormalizedRect {
    composition::cover_rect(
        f64::from(width) / f64::from(height.max(1)),
        f64::from(image_width) / f64::from(image_height.max(1)),
    )
}

pub(super) fn full_rect() -> NormalizedRect {
    NormalizedRect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    }
}

pub(super) fn style_value(style: CursorStyle) -> f32 {
    match style {
        CursorStyle::Default => 0.0,
        CursorStyle::Circle => 1.0,
    }
}

pub(super) fn color(value: Option<&String>, fallback: [f32; 4]) -> [f32; 4] {
    let Some(value) = value else { return fallback };
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    let expanded = match hex.len() {
        3 | 4 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => hex.to_string(),
        _ => return fallback,
    };
    let parse = |start| u8::from_str_radix(&expanded[start..start + 2], 16).ok();
    let (Some(r), Some(g), Some(b)) = (parse(0), parse(2), parse(4)) else {
        return fallback;
    };
    let a = if expanded.len() == 8 {
        parse(6).unwrap_or(255)
    } else {
        255
    };
    [
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        f32::from(a) / 255.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::{compile, shaders};

    /// The export shaders are assembled from string fragments and compiled at
    /// runtime, so a syntax error would otherwise only surface when a user
    /// starts an export. D3DCompile is a software compiler and needs no device,
    /// which makes this checkable here.
    #[test]
    fn shaders_compile() {
        compile(&shaders::vertex(), b"vs_5_0\0").expect("vertex shader");

        for (name, source) in [
            ("texture", shaders::texture()),
            ("movement blur", shaders::movement_blur()),
            ("zoom blur", shaders::zoom_blur()),
            ("gradient", shaders::gradient()),
            ("cursor", shaders::cursor()),
        ] {
            compile(&source, b"ps_5_0\0")
                .unwrap_or_else(|error| panic!("{name} shader: {error:?}"));
        }
    }
}
