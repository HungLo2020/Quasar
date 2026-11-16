// Quasar post-processing: bloom (prefilter, downsample, upsample) and the
// final composite (exposure, bloom, tonemap, grade, vignette, sRGB encode).

struct Params {
    // xy: source texel size, z: bloom threshold, w: soft knee
    a: vec4<f32>,
    // x: exposure, y: bloom intensity, z: tonemap (0 none, 1 reinhard, 2 aces),
    // w: encode sRGB in shader (1) or write linear (0)
    b: vec4<f32>,
    // x: saturation, y: contrast, z: vignette strength, w: vignette radius
    c: vec4<f32>,
    // rgb: colour tint (gain), w: vignette softness
    d: vec4<f32>,
    // rgb: lift, w: bloom enabled
    e: vec4<f32>,
};

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var bloom: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn fullscreen(@builtin(vertex_index) index: u32) -> VertexOutput {
    // One oversized triangle covering the viewport.
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn sample_source(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(source, linear_sampler, uv, 0.0).rgb;
}

// 13-tap downsample (Jimenez, "Next Generation Post Processing in Call of
// Duty"), which suppresses fireflies and aliasing when halving resolution.
fn downsample13(uv: vec2<f32>) -> vec3<f32> {
    let t = params.a.xy;
    let a = sample_source(uv + t * vec2<f32>(-2.0, -2.0));
    let b = sample_source(uv + t * vec2<f32>(0.0, -2.0));
    let c = sample_source(uv + t * vec2<f32>(2.0, -2.0));
    let d = sample_source(uv + t * vec2<f32>(-1.0, -1.0));
    let e = sample_source(uv + t * vec2<f32>(1.0, -1.0));
    let f = sample_source(uv + t * vec2<f32>(-2.0, 0.0));
    let g = sample_source(uv);
    let h = sample_source(uv + t * vec2<f32>(2.0, 0.0));
    let i = sample_source(uv + t * vec2<f32>(-1.0, 1.0));
    let j = sample_source(uv + t * vec2<f32>(1.0, 1.0));
    let k = sample_source(uv + t * vec2<f32>(-2.0, 2.0));
    let l = sample_source(uv + t * vec2<f32>(0.0, 2.0));
    let m = sample_source(uv + t * vec2<f32>(2.0, 2.0));
    var result = (d + e + i + j) * 0.125;
    result += (a + b + g + f) * 0.03125;
    result += (b + c + h + g) * 0.03125;
    result += (f + g + l + k) * 0.03125;
    result += (g + h + m + l) * 0.03125;
    return result;
}

@fragment
fn prefilter(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = downsample13(in.uv) * params.b.x;
    // Soft-knee threshold on the brightest channel.
    let threshold = params.a.z;
    let knee = max(threshold * params.a.w, 1e-4);
    let brightness = max(color.r, max(color.g, color.b));
    var soft = clamp(brightness - threshold + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee + 1e-5);
    let contribution = max(soft, brightness - threshold) / max(brightness, 1e-5);
    return vec4<f32>(color * contribution, 1.0);
}

@fragment
fn downsample(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(downsample13(in.uv), 1.0);
}

// 3x3 tent filter; the pipeline blends additively into the larger level.
@fragment
fn upsample(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = params.a.xy;
    var sum = sample_source(in.uv) * 4.0;
    sum += (sample_source(in.uv + vec2<f32>(-t.x, 0.0)) + sample_source(in.uv + vec2<f32>(t.x, 0.0))
        + sample_source(in.uv + vec2<f32>(0.0, -t.y)) + sample_source(in.uv + vec2<f32>(0.0, t.y))) * 2.0;
    sum += sample_source(in.uv + vec2<f32>(-t.x, -t.y)) + sample_source(in.uv + vec2<f32>(t.x, -t.y))
        + sample_source(in.uv + vec2<f32>(-t.x, t.y)) + sample_source(in.uv + vec2<f32>(t.x, t.y));
    return vec4<f32>(sum / 16.0, 1.0);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    // Narkowicz's fitted ACES curve.
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_to_srgb(x: vec3<f32>) -> vec3<f32> {
    let low = x * 12.92;
    let high = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, x <= vec3<f32>(0.0031308));
}

@fragment
fn composite(in: VertexOutput) -> @location(0) vec4<f32> {
    let sampled = textureSampleLevel(source, linear_sampler, in.uv, 0.0);
    var color = sampled.rgb * params.b.x;
    if (params.e.w > 0.5) {
        color += textureSampleLevel(bloom, linear_sampler, in.uv, 0.0).rgb * params.b.y;
    }
    let mode = params.b.z;
    if (mode > 1.5) {
        color = aces(color);
    } else if (mode > 0.5) {
        color = color / (vec3<f32>(1.0) + color);
    }
    // Grade in display-referred linear space: lift, gain/tint, contrast, saturation.
    color = params.e.rgb + color * params.d.rgb;
    color = (color - vec3<f32>(0.18)) * params.c.y + vec3<f32>(0.18);
    let luma = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    color = mix(vec3<f32>(luma), color, params.c.x);
    // Vignette: darken towards the corners.
    let centered = (in.uv - vec2<f32>(0.5)) * 2.0;
    let distance = length(centered) / 1.41421356;
    let falloff = smoothstep(params.c.w, params.c.w + max(params.d.w, 1e-4), distance);
    color *= 1.0 - params.c.z * falloff;
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    if (params.b.w > 0.5) {
        color = linear_to_srgb(color);
    }
    return vec4<f32>(color, 1.0);
}
