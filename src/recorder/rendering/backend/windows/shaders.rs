//! HLSL for the export compositor.
//!
//! Every shader shares one constant buffer, so each source is assembled from
//! [`CONSTANTS`] plus its own body rather than repeating the declaration.
//!
//! The recording layer has three variants — sharp, directional, and radial.
//! Selecting a whole shader instead of branching inside one means a still frame
//! never reaches a sampling loop at all.

/// Shared declarations. `motion` carries the display motion blur: the movement
/// vector for the directional shader, and the focus point plus signed scale
/// delta for the radial one.
const CONSTANTS: &str = r#"
cbuffer Constants : register(b0) {
    float4 destination;
    float4 source;
    float4 color_start;
    float4 color_end;
    float4 misc;
    float4 motion;
};
struct Input { float4 position : SV_POSITION; float2 uv : TEXCOORD0; float2 local : TEXCOORD1; };
"#;

/// Rounded-corner clipping, shared by every layer that can be rounded: the
/// recording, and — in the editor preview — the canvas background behind it.
/// `misc.x` is the radius in target pixels and `misc.yz` the target size, so a
/// radius of zero leaves the quad square and costs one comparison.
const CORNERS: &str = r#"
float rounded_distance(float2 local, float2 size, float radius) {
    float2 p = local * size;
    float2 q = abs(p - size * 0.5) - (size * 0.5 - radius);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}
void clip_corners(Input input) {
    if (misc.x > 0.0 && rounded_distance(input.local, float2(destination.z * misc.y, destination.w * misc.z), misc.x) > 0.0) discard;
}
"#;

/// Sampling helpers for textured layers. The sampler is clamped, so a tap that
/// leaves the source repeats its edge texel instead of wrapping or reading
/// black.
const RECORDING: &str = r#"
Texture2D frame_texture : register(t0);
SamplerState frame_sampler : register(s0);
float4 tap(float2 uv) { return frame_texture.Sample(frame_sampler, uv); }
"#;

pub(crate) fn vertex() -> String {
    format!("{CONSTANTS}{VERTEX_BODY}")
}

const VERTEX_BODY: &str = r#"
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

pub(crate) fn texture() -> String {
    format!(
        "{CONSTANTS}{CORNERS}{RECORDING}{}",
        r#"
float4 main(Input input) : SV_TARGET {
    clip_corners(input);
    return tap(input.uv);
}
"#
    )
}

/// Directional blur along the recording layer's inter-frame movement.
///
/// The span is centred on the pixel, so the image stretches along its travel
/// without lagging behind it, and every tap carries the same weight — the
/// result is one smear, not a stack of offset copies.
pub(crate) fn movement_blur() -> String {
    format!(
        "{CONSTANTS}{CORNERS}{RECORDING}{}",
        r#"
static const int TAPS = 21;
float4 main(Input input) : SV_TARGET {
    clip_corners(input);
    float2 span = motion.xy;
    float2 step = span / float(TAPS - 1);
    float2 uv = input.uv - span * 0.5;
    float4 total = float4(0, 0, 0, 0);
    [unroll] for (int i = 0; i < TAPS; ++i) {
        total += tap(uv);
        uv += step;
    }
    return total / float(TAPS);
}
"#
    )
}

/// Radial blur along the ray between each pixel and the zoom focus.
///
/// Taps are weighted by `4(t - t²)`, a bell that peaks on the pixel itself and
/// falls to nothing at both ends of the span, which is what keeps the centre
/// crisp while the periphery stretches. The span is symmetric, so zooming in
/// and zooming out smear along the same radial line in opposite directions.
pub(crate) fn zoom_blur() -> String {
    format!(
        "{CONSTANTS}{CORNERS}{RECORDING}{}",
        r#"
static const int TAPS = 13;
static const float MAX_ZOOM_RAY_UV = 0.10;
static const float WEIGHT_FLOOR = 0.02;

// Interleaved gradient noise. It depends only on the pixel's position, so the
// same pixel dithers identically every frame and a held frame cannot shimmer.
float dither(float2 position) {
    return frac(52.9829189 * frac(dot(position, float2(0.06711056, 0.00583715))));
}

float4 main(Input input) : SV_TARGET {
    clip_corners(input);
    float2 ray = (input.uv - motion.xy) * motion.z;
    float extent = length(ray);
    if (extent > MAX_ZOOM_RAY_UV) ray *= MAX_ZOOM_RAY_UV / extent;
    float jitter = dither(input.position.xy) - 0.5;
    float4 total = float4(0, 0, 0, 0);
    float total_weight = 0.0;
    [unroll] for (int i = 0; i < TAPS; ++i) {
        float t = saturate((float(i) + jitter) / float(TAPS - 1));
        float weight = 4.0 * (t - t * t) + WEIGHT_FLOOR;
        total += tap(input.uv + ray * (t - 0.5)) * weight;
        total_weight += weight;
    }
    return total / total_weight;
}
"#
    )
}

/// Solid and gradient canvas backgrounds. A solid fill is this shader with
/// both stops set to the same colour, which keeps one rounded-rectangle path
/// instead of a second shader that differs only in its interpolation.
pub(crate) fn gradient() -> String {
    format!(
        "{CONSTANTS}{CORNERS}{}",
        r#"
float gradient_position(float2 local) {
    float radians = (fmod(misc.w, 360.0) - 90.0) * (3.14159265 / 180.0);
    float2 direction = float2(cos(radians), sin(radians));
    // The quad's own pixel size, derived the same way the corner clip derives
    // it, so one `misc.yz` serves both.
    float2 size = destination.zw * misc.yz;
    if (size.x > size.y) {
        direction.y *= size.y / size.x;
    } else {
        direction.x *= size.x / size.y;
    }
    float2 half_size = size * 0.5;
    float2 center_to_point = (local - 0.5) * size;
    float position = dot(center_to_point, direction) / length(direction);
    if (abs(direction.x) > abs(direction.y)) {
        position = (position + half_size.x) / size.x;
    } else {
        position = (position + half_size.y) / size.y;
    }
    return saturate(position);
}

float4 main(Input input) : SV_TARGET {
    clip_corners(input);
    return lerp(color_start, color_end, gradient_position(input.local));
}
"#
    )
}

/// The reconstructed cursor, drawn from the same geometry as the SVG assets the
/// legacy preview rasterises.
///
/// Signed distance rather than a sampled bitmap: the cursor is scalable and
/// scaled, so a distance field stays crisp at any size where a 24x32 sprite
/// would blur. Both shapes are stroked exactly as the assets are — a 2px stroke
/// centred on the path, black outside white — so the preview, the export, and
/// the legacy preview agree on what the cursor looks like.
pub(crate) fn cursor() -> String {
    format!(
        "{CONSTANTS}{}",
        r#"
// The default cursor's outline, in its own 24x32 space. Taken from the asset's
// path data, so the two cannot drift.
static const int ARROW_POINTS = 7;
static const float2 ARROW[ARROW_POINTS] = {
    float2(2, 1), float2(2, 28), float2(9, 21), float2(14, 31),
    float2(18, 29), float2(13, 19), float2(22, 19)
};
static const float STROKE = 1.0;

float segment_distance(float2 p, float2 a, float2 b) {
    float2 pa = p - a;
    float2 ba = b - a;
    float h = saturate(dot(pa, ba) / max(dot(ba, ba), 1e-6));
    return length(pa - ba * h);
}

// Negative inside the outline. Distance to the nearest edge gives round joins
// for free, which is what the asset's stroke-linejoin asks for.
float arrow_distance(float2 p) {
    float distance = 1e9;
    bool inside = false;
    [unroll] for (int i = 0; i < ARROW_POINTS; ++i) {
        float2 a = ARROW[i];
        float2 b = ARROW[(i + 1) % ARROW_POINTS];
        distance = min(distance, segment_distance(p, a, b));
        if (((a.y > p.y) != (b.y > p.y)) &&
            (p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x)) {
            inside = !inside;
        }
    }
    return inside ? -distance : distance;
}

// White fill, black stroke, premultiplied so the blend over the composition is
// a plain source-over.
float4 stroked(float signed_distance, float width) {
    float edge = max(fwidth(signed_distance), 1e-4);
    float alpha = 1.0 - smoothstep(width - edge, width + edge, signed_distance);
    if (alpha <= 0.0) discard;
    float ink = smoothstep(-width - edge, -width + edge, signed_distance);
    float3 color = lerp(float3(1, 1, 1), float3(0, 0, 0), ink);
    return float4(color * alpha, alpha);
}

float4 main(Input input) : SV_TARGET {
    if (misc.w > 0.5) {
        // Circle: an 11-radius white disc with a 2px stroke and a 3-radius dot.
        float2 p = input.local * 32.0;
        float radius = distance(p, float2(16, 16));
        float edge = max(fwidth(radius), 1e-4);
        float4 ring = stroked(radius - 11.0, STROKE);
        float dot_alpha = 1.0 - smoothstep(3.0 - edge, 3.0 + edge, radius);
        return lerp(ring, float4(0, 0, 0, 1), dot_alpha);
    }
    return stroked(arrow_distance(input.local * float2(24, 32)), STROKE);
}
"#
    )
}
