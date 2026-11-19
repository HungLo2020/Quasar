// Ray-marched volumetric fog over a SceneTarget. Prepended: diagnostic
// directive and LIGHTING_WGSL (lights at binding 3; shadows at 5, 6, 21).
//
// Each pixel marches from the eye to the scene depth (or `max distance`
// for the sky) through a fog volume: a height layer whose density is shaped
// by drifting 3D noise into banks and wisps. Every sample is lit by the
// pass's lights with a Henyey-Greenstein phase (halos around lamps you look
// towards) plus ambient light, and the result is blended as
// `scene * transmittance + in-scattered light` in one pass.
struct Params {
    inverse_view_projection: mat4x4<f32>,
    // xy target size, z time in seconds, w steps
    size: vec4<f32>,
    // xyz eye, w max distance
    eye: vec4<f32>,
    // rgb scattering albedo, w density at the base height
    albedo: vec4<f32>,
    // x base height, y falloff per metre, z noise frequency, w noise strength
    height: vec4<f32>,
    // xyz wind in metres per second, w anisotropy g
    wind: vec4<f32>,
    // rgb ambient light in the fog, w density everywhere (thin haze)
    ambient: vec4<f32>,
};
@group(0) @binding(0) var scene_depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

fn fog_hash(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.3183099 + vec3<f32>(0.71, 0.113, 0.419));
    q = q * 17.0;
    return fract(q.x * q.y * q.z * (q.x + q.y + q.z));
}

fn value_noise(x: vec3<f32>) -> f32 {
    let i = floor(x);
    let f = fract(x);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(fog_hash(i), fog_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(fog_hash(i + vec3<f32>(0.0, 1.0, 0.0)), fog_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(fog_hash(i + vec3<f32>(0.0, 0.0, 1.0)), fog_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(fog_hash(i + vec3<f32>(0.0, 1.0, 1.0)), fog_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

fn volume_density(p: vec3<f32>) -> f32 {
    let above = max(p.y - params.height.x, 0.0);
    let layer = params.albedo.w * exp(-params.height.y * above);
    let time = params.size.z;
    // Banks slide with the wind and slowly change shape (the noise also
    // moves along y, its "evolution" axis).
    let q = (p - params.wind.xyz * time) * params.height.z + vec3<f32>(0.0, time * 0.035, 0.0);
    // Two octaves, the finer wisps faster and slightly across the wind,
    // squashed vertically so banks spread along the ground.
    let squash = vec3<f32>(1.0, 2.2, 1.0);
    let gust = vec3<f32>(params.wind.z, 0.0, -params.wind.x) * 0.5 + params.wind.xyz * 0.8;
    let n = value_noise(q * squash) * 0.65
        + value_noise(q * squash * 2.7 + vec3<f32>(17.0, 3.0, 9.0) - gust * time * params.height.z * 2.7) * 0.35;
    // Sharpen the noise into distinct banks with clear gaps between them.
    let shaped = smoothstep(0.38, 0.72, n);
    return (layer + params.ambient.w) * mix(1.0, shaped * 2.0, params.height.w);
}

fn phase(cos_theta: f32) -> f32 {
    let g = params.wind.w;
    let g2 = g * g;
    return (1.0 - g2) / (12.566371 * pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4), 1.5));
}

@fragment
fn march(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let depth = textureLoad(scene_depth, pixel, 0);
    let uv = position.xy / params.size.xy;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let far = params.inverse_view_projection * vec4<f32>(ndc, min(depth, 0.9999999), 1.0);
    let end = far.xyz / far.w;
    let eye = params.eye.xyz;
    let ray = end - eye;
    var distance = length(ray);
    let v = ray / max(distance, 1e-4);
    if (depth >= 1.0) {
        distance = params.eye.w;
    }
    distance = min(distance, params.eye.w);
    let steps = max(i32(params.size.w), 1);
    let dt = distance / f32(steps);
    // Interleaved gradient noise, varied per frame: hides banding.
    let jitter = fract(52.9829189 * fract(dot(position.xy + vec2<f32>(params.size.z * 37.0, 0.0), vec2<f32>(0.06711056, 0.00583715))));
    var transmittance = 1.0;
    var light_sum = vec3<f32>(0.0);
    for (var s = 0; s < steps; s++) {
        let t = (f32(s) + jitter) * dt;
        let p = eye + v * t;
        let sigma = volume_density(p);
        if (sigma <= 1e-5) {
            continue;
        }
        var incoming = params.ambient.rgb;
        for (var i = 0; i < i32(lighting.info.x); i++) {
            let light = lighting.lights[i];
            let radiance = light.colorIntensity.rgb * light.colorIntensity.a;
            if (light.positionKind.w < 0.5) {
                incoming += radiance * phase(dot(light.directionRange.xyz, -v));
            } else {
                var l = vec3<f32>(0.0);
                let attenuation = light_attenuation(light, p, &l);
                if (attenuation > 0.0) {
                    incoming += radiance * attenuation * phase(dot(l, v));
                }
            }
        }
        let extinction = exp(-sigma * dt);
        light_sum += transmittance * incoming * params.albedo.rgb * (1.0 - extinction);
        transmittance *= extinction;
        if (transmittance < 0.01) {
            break;
        }
    }
    return vec4<f32>(light_sum, transmittance);
}
