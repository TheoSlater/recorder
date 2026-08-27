//! D3D11 resources the composition renderer needs: shader compilation, the
//! shared constant buffer, render textures, and colour parsing.

use windows::{
    Win32::Graphics::{
        Direct3D::{Fxc::D3DCompile, ID3DBlob, ID3DInclude},
        Direct3D11::{
            D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_SHADER_RESOURCE, D3D11_BUFFER_DESC,
            D3D11_CPU_ACCESS_WRITE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC,
            ID3D11ClassLinkage, ID3D11Device, ID3D11PixelShader, ID3D11Texture2D,
            ID3D11VertexShader,
        },
        Dxgi::Common::DXGI_SAMPLE_DESC,
    },
    core::PCSTR,
};

use super::super::super::RenderError;
use super::constants::Constants;
use super::shaders;

type Result<T> = std::result::Result<T, RenderError>;

fn device_error(what: &str, error: impl std::fmt::Display) -> RenderError {
    RenderError::Device(format!("{what}: {error}"))
}

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
        .map_err(|error| device_error("could not create a render texture", error))?;
    texture.ok_or_else(|| RenderError::Device("render texture was null".into()))
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
        .map_err(|error| device_error("could not create the constant buffer", error))?;
    buffer.ok_or_else(|| RenderError::Device("constant buffer was null".into()))
}

pub(crate) fn create_vertex_shader(device: &ID3D11Device) -> Result<ID3D11VertexShader> {
    let code = compile(&shaders::vertex(), b"vs_5_0\0")?;
    let mut shader = None;
    unsafe { device.CreateVertexShader(&code, None::<&ID3D11ClassLinkage>, Some(&mut shader)) }
        .map_err(|error| device_error("could not create the vertex shader", error))?;
    shader.ok_or_else(|| RenderError::Device("vertex shader was null".into()))
}

pub(crate) fn create_pixel_shader(
    device: &ID3D11Device,
    source: &str,
) -> Result<ID3D11PixelShader> {
    let code = compile(source, b"ps_5_0\0")?;
    let mut shader = None;
    unsafe { device.CreatePixelShader(&code, None::<&ID3D11ClassLinkage>, Some(&mut shader)) }
        .map_err(|error| device_error("could not create a pixel shader", error))?;
    shader.ok_or_else(|| RenderError::Device("pixel shader was null".into()))
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
        RenderError::Device(format!(
            "could not compile a shader: {error}{}",
            blob_text(errors)
        ))
    })?;
    let blob =
        blob.ok_or_else(|| RenderError::Device("shader compiler returned no bytecode".into()))?;
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
