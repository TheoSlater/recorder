pub(super) const VERTEX: &str = r#"
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

pub(super) const TEXTURE: &str = r#"
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

pub(super) const GRADIENT: &str = r#"
cbuffer Constants : register(b0) { float4 destination; float4 source; float4 color_start; float4 color_end; float4 misc; };
struct Input { float4 position : SV_POSITION; float2 uv : TEXCOORD0; float2 local : TEXCOORD1; };
float4 main(Input input) : SV_TARGET { return lerp(color_start, color_end, input.local.y); }
"#;

pub(super) const CURSOR: &str = r#"
cbuffer Constants : register(b0) { float4 destination; float2 source; float4 color_start; float4 color_end; float4 misc; };
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
