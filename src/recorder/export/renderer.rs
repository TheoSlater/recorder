use std::path::Path;

use anyhow::{Context, Result, anyhow};
use windows::{
    Win32::Graphics::{
        Direct3D::{D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, Fxc::D3DCompile, ID3DBlob, ID3DInclude},
        Direct3D11::{
            D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
            D3D11_BUFFER_DESC, D3D11_MAP_WRITE_DISCARD, D3D11_MAPPED_SUBRESOURCE,
            D3D11_SAMPLER_DESC, D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_CLAMP,
            D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC,
            D3D11_CPU_ACCESS_WRITE, ID3D11Buffer, ID3D11ClassLinkage,
            ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
            ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView,
            ID3D11Texture2D, ID3D11VertexShader, D3D11_VIEWPORT, D3D11_SUBRESOURCE_DATA,
        },
        Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC},
    },
    core::PCSTR,
};

use super::super::{
    composition::{self, CompositionFrame, NormalizedRect, SourceSize},
    cursor_settings::CursorStyle,
    project_settings::{CanvasBackgroundKind, CanvasComposition},
};
use super::decoder::DeviceContext;
use super::shaders::{CURSOR as CURSOR_SHADER, GRADIENT as GRADIENT_SHADER, TEXTURE as TEXTURE_SHADER, VERTEX as VERTEX_SHADER};

const DEFAULT_BACKGROUND: [f32; 4] = [0.11, 0.13, 0.17, 1.0];
const DEFAULT_GRADIENT_END: [f32; 4] = [0.04, 0.05, 0.07, 1.0];

#[repr(C)]
#[derive(Clone, Copy)]
struct Constants {
    destination: [f32; 4],
    source: [f32; 4],
    color_start: [f32; 4],
    color_end: [f32; 4],
    misc: [f32; 4],
}

struct ImageTexture {
    _texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    width: u32,
    height: u32,
}

pub(crate) struct Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    output: ID3D11Texture2D,
    output_view: ID3D11RenderTargetView,
    constants: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    vertex_shader: ID3D11VertexShader,
    texture_shader: ID3D11PixelShader,
    gradient_shader: ID3D11PixelShader,
    cursor_shader: ID3D11PixelShader,
    background: Option<ImageTexture>,
    width: u32,
    height: u32,
}

impl Renderer {
    pub(crate) fn new(
        device: &DeviceContext,
        width: u32,
        height: u32,
        composition: &CanvasComposition,
    ) -> Result<Self> {
        let output = create_output(&device.device, width, height)?;
        let mut output_view = None;
        unsafe {
            device
                .device
                .CreateRenderTargetView(&output, None, Some(&mut output_view))
        }
        .context("could not create export render target")?;
        let output_view = output_view
            .ok_or_else(|| anyhow!("export render target view was null"))?;
        let constants = create_constants(&device.device)?;
        let mut sampler = None;
        unsafe {
            device.device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MinLOD: 0.0,
                MaxLOD: f32::MAX,
                ..Default::default()
                },
                Some(&mut sampler),
            )
        }
        .context("could not create export sampler")?;
        let sampler = sampler.ok_or_else(|| anyhow!("export sampler was null"))?;
        let vertex_shader = create_vertex_shader(&device.device)?;
        let texture_shader = create_pixel_shader(&device.device, TEXTURE_SHADER)?;
        let gradient_shader = create_pixel_shader(&device.device, GRADIENT_SHADER)?;
        let cursor_shader = create_pixel_shader(&device.device, CURSOR_SHADER)?;
        let background = match composition.background.kind {
            CanvasBackgroundKind::Image => composition
                .background
                .image_path
                .as_deref()
                .map(|path| load_image(&device.device, path))
                .transpose()?,
            CanvasBackgroundKind::Solid | CanvasBackgroundKind::Gradient => None,
        };
        Ok(Self {
            device: device.device.clone(),
            context: device.context.clone(),
            output,
            output_view,
            constants,
            sampler,
            vertex_shader,
            texture_shader,
            gradient_shader,
            cursor_shader,
            background,
            width,
            height,
        })
    }

    pub(crate) fn render(
        &self,
        source_texture: &ID3D11Texture2D,
        frame: &CompositionFrame,
        source: SourceSize,
        composition: &CanvasComposition,
    ) -> Result<&ID3D11Texture2D> {
        unsafe {
            self.context.OMSetRenderTargets(
                Some(&[Some(self.output_view.clone())]),
                None::<&ID3D11DepthStencilView>,
            );
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.width as f32,
                Height: self.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
        }
        let start = color(
            composition.background.solid_color.as_ref(),
            DEFAULT_BACKGROUND,
        );
        let end = color(
            composition.background.gradient_end.as_ref(),
            DEFAULT_GRADIENT_END,
        );
        unsafe { self.context.ClearRenderTargetView(&self.output_view, &start) };
        match composition.background.kind {
            CanvasBackgroundKind::Solid => {}
            CanvasBackgroundKind::Gradient => self.draw(
                full_rect(),
                [0.0, 0.0, 1.0, 1.0],
                start,
                end,
                [0.0; 4],
                None,
                &self.gradient_shader,
            )?,
            CanvasBackgroundKind::Image => {
                if let Some(background) = &self.background {
                    let rect = cover_rect(self.width, self.height, background.width, background.height);
                    self.draw(
                        rect,
                        [0.0, 0.0, 1.0, 1.0],
                        start,
                        end,
                        [0.0; 4],
                        Some(&background.view),
                        &self.texture_shader,
                    )?;
                }
            }
        }

        let mut source_view = None;
        unsafe {
            self.device
                .CreateShaderResourceView(source_texture, None, Some(&mut source_view))
        }
        .context("could not create decoded frame shader view")?;
        let source_view = source_view
            .ok_or_else(|| anyhow!("decoded frame shader view was null"))?;
        let radius = (frame.recording.width * self.width as f64)
            .min(frame.recording.height * self.height as f64)
            * frame.corner_radius;
        self.draw(
            frame.recording,
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
            [1.0; 4],
            [radius as f32, self.width as f32, self.height as f32, 0.0],
            Some(&source_view),
            &self.texture_shader,
        )?;

        if let Some(cursor) = frame.cursor.filter(|cursor| cursor.visible)
            && let Some(rect) = composition::cursor_rect(frame, source, cursor.style.asset())
        {
            self.draw(
                rect,
                [0.0, 0.0, 1.0, 1.0],
                [0.0; 4],
                [0.0; 4],
                [0.0, 0.0, 0.0, style_value(cursor.style)],
                None,
                &self.cursor_shader,
            )?;
        }
        unsafe { self.context.Flush() };
        Ok(&self.output)
    }

    fn draw(
        &self,
        destination: NormalizedRect,
        source: [f32; 4],
        color_start: [f32; 4],
        color_end: [f32; 4],
        misc: [f32; 4],
        view: Option<&ID3D11ShaderResourceView>,
        pixel_shader: &ID3D11PixelShader,
    ) -> Result<()> {
        let constants = Constants {
            destination: [
                destination.x as f32,
                destination.y as f32,
                destination.width as f32,
                destination.height as f32,
            ],
            source,
            color_start,
            color_end,
            misc,
        };
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&self.constants, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .context("could not map export constants")?;
            std::ptr::copy_nonoverlapping(
                (&constants as *const Constants).cast::<u8>(),
                mapped.pData.cast::<u8>(),
                std::mem::size_of::<Constants>(),
            );
            self.context.Unmap(&self.constants, 0);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(pixel_shader, None);
            self.context.VSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context.PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            let resources = view.map(|view| [Some(view.clone())]);
            self.context
                .PSSetShaderResources(0, resources.as_ref().map(|resources| &resources[..]));
            self.context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            self.context.Draw(4, 0);
        }
        Ok(())
    }
}

fn create_output(device: &ID3D11Device, width: u32, height: u32) -> Result<ID3D11Texture2D> {
    let description = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&description, None, Some(&mut texture)) }
        .context("could not create export render texture")?;
    texture.ok_or_else(|| anyhow!("export render texture was null"))
}

fn create_constants(device: &ID3D11Device) -> Result<ID3D11Buffer> {
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

fn create_vertex_shader(device: &ID3D11Device) -> Result<ID3D11VertexShader> {
    let code = compile(VERTEX_SHADER, b"vs_5_0\0")?;
    let mut shader = None;
    unsafe {
        device.CreateVertexShader(&code, None::<&ID3D11ClassLinkage>, Some(&mut shader))
    }
    .context("could not create export vertex shader")?;
    shader.ok_or_else(|| anyhow!("export vertex shader was null"))
}

fn create_pixel_shader(device: &ID3D11Device, source: &str) -> Result<ID3D11PixelShader> {
    let code = compile(source, b"ps_5_0\0")?;
    let mut shader = None;
    unsafe { device.CreatePixelShader(&code, None::<&ID3D11ClassLinkage>, Some(&mut shader)) }
        .context("could not create export pixel shader")?;
    shader.ok_or_else(|| anyhow!("export pixel shader was null"))
}

fn compile(source: &str, target: &[u8]) -> Result<Vec<u8>> {
    let mut blob: Option<ID3DBlob> = None;
    unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
            None,
            None::<&ID3DInclude>,
            PCSTR(b"main\0".as_ptr()),
            PCSTR(target.as_ptr()),
            0,
            0,
            &mut blob,
            None,
        )
    }
    .context("could not compile export shader")?;
    let blob = blob.ok_or_else(|| anyhow!("shader compiler returned no bytecode"))?;
    unsafe {
        Ok(std::slice::from_raw_parts(
            blob.GetBufferPointer().cast(),
            blob.GetBufferSize(),
        )
        .to_vec())
    }
}

fn load_image(device: &ID3D11Device, path: &Path) -> Result<ImageTexture> {
    let bytes = std::fs::read(path).with_context(|| format!("could not read background image {}", path.display()))?;
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
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
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
    Ok(ImageTexture { _texture: texture, view, width, height })
}

fn cover_rect(width: u32, height: u32, image_width: u32, image_height: u32) -> NormalizedRect {
    let canvas_aspect = f64::from(width) / f64::from(height);
    let image_aspect = f64::from(image_width) / f64::from(image_height.max(1));
    if image_aspect >= canvas_aspect {
        NormalizedRect { x: 0.5 - image_aspect / canvas_aspect / 2.0, y: 0.0, width: image_aspect / canvas_aspect, height: 1.0 }
    } else {
        NormalizedRect { x: 0.0, y: 0.5 - canvas_aspect / image_aspect / 2.0, width: 1.0, height: canvas_aspect / image_aspect }
    }
}

fn full_rect() -> NormalizedRect {
    NormalizedRect { x: 0.0, y: 0.0, width: 1.0, height: 1.0 }
}

fn style_value(style: CursorStyle) -> f32 {
    match style {
        CursorStyle::Default => 0.0,
        CursorStyle::Circle => 1.0,
    }
}

fn color(value: Option<&String>, fallback: [f32; 4]) -> [f32; 4] {
    let Some(value) = value else { return fallback };
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    let expanded = match hex.len() {
        3 | 4 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => hex.to_string(),
        _ => return fallback,
    };
    let parse = |start| u8::from_str_radix(&expanded[start..start + 2], 16).ok();
    let (Some(r), Some(g), Some(b)) = (parse(0), parse(2), parse(4)) else { return fallback };
    let a = if expanded.len() == 8 { parse(6).unwrap_or(255) } else { 255 };
    [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, f32::from(a) / 255.0]
}

const VERTEX_SHADER: &str = r#"
cbuffer Constants : register(b0) { float4 destination; float4 source; float4 color_start; float4 color_end; float4 misc; };
struct Output { float4 position : SV_POSITION; float2 uv : TEXCOORD0; float2 local : TEXCOORD1; };
Output main(uint id : SV_VertexID) {
    float2 corners[4] = { float2(0,0), float2(1,0), float2(0,1), float2(1,1) };
    float2 local = corners[id];
    float2 p = destination.xy + local * destination.zw;
    Output output;
    output.position = float4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
    output.uv = lerp(source.xy, source.zw, local);
    output.local = local;
    return output;
}
"#;

const TEXTURE_SHADER: &str = r#"
Texture2D frame_texture : register(t0);
SamplerState frame_sampler : register(s0);
cbuffer Constants : register(b0) { float4 destination; float4 source; float4 color_start; float4 color_end; float4 misc; };
struct Input { float4 position : SV_POSITION; float2 uv : TEXCOORD0; float2 local : TEXCOORD1; };
float rounded_distance(float2 local, float2 size, float radius) {
    float2 p = local * size;
    float2 q = abs(p - size * 0.5) - (size * 0.5 - radius);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}
float4 main(Input input) : SV_TARGET {
    if (misc.x > 0.0 && rounded_distance(input.local, float2(destination.z * misc.y, destination.w * misc.z), misc.x) > 0.0) discard;
    return frame_texture.Sample(frame_sampler, input.uv);
}
"#;

const GRADIENT_SHADER: &str = r#"
cbuffer Constants : register(b0) { float4 destination; float4 source; float4 color_start; float4 color_end; float4 misc; };
struct Input { float4 position : SV_POSITION; float2 uv : TEXCOORD0; float2 local : TEXCOORD1; };
float4 main(Input input) : SV_TARGET { return lerp(color_start, color_end, input.local.y); }
"#;

const CURSOR_SHADER: &str = r#"
cbuffer Constants : register(b0) { float4 destination; float4 source; float4 color_start; float4 color_end; float4 misc; };
struct Input { float4 position : SV_POSITION; float2 uv : TEXCOORD0; float2 local : TEXCOORD1; };
bool triangle(float2 p, float2 a, float2 b, float2 c) {
    float ab = (b.x-a.x)*(p.y-a.y)-(b.y-a.y)*(p.x-a.x);
    float bc = (c.x-b.x)*(p.y-b.y)-(c.y-b.y)*(p.x-b.x);
    float ca = (a.x-c.x)*(p.y-c.y)-(a.y-c.y)*(p.x-c.x);
    return (ab >= 0.0 && bc >= 0.0 && ca >= 0.0) || (ab <= 0.0 && bc <= 0.0 && ca <= 0.0);
}
bool arrow(float2 p) {
    return triangle(p,float2(2,1),float2(2,28),float2(9,21)) ||
           triangle(p,float2(2,1),float2(9,21),float2(14,31)) ||
           triangle(p,float2(2,1),float2(14,31),float2(18,29)) ||
           triangle(p,float2(2,1),float2(18,29),float2(13,19)) ||
           triangle(p,float2(2,1),float2(13,19),float2(23,19));
}
float4 main(Input input) : SV_TARGET {
    if (misc.w > 0.5) {
        float d = distance(input.local, float2(0.5,0.5));
        if (d > 0.5) discard;
        return d < 0.42 ? float4(1,1,1,1) : float4(0,0,0,1);
    }
    float2 p = input.local * float2(24,32);
    if (!arrow(p)) discard;
    float2 inner = (p - float2(2,1)) * 0.86 + float2(2,1);
    return arrow(inner) ? float4(1,1,1,1) : float4(0,0,0,1);
}
"#;
