// Depth-driven screen effects drawn over a SceneTarget's colour: ink edges
// (depth discontinuities and creases where the depth-reconstructed surface
// bends) and
// radial god rays from open sky. Both read only the scene depth, so they
// blend straight into the scene colour with no copy.
struct Params {
    inverse_view_projection: mat4x4<f32>,
    // xy: target size, zw: light position in uv (god rays)
    size: vec4<f32>,
    // xyz: eye, w: unused
    eye: vec4<f32>,
    // edges: rgb colour, a strength; god rays: rgb colour x intensity
    color: vec4<f32>,
    // edges: x width px, y depth threshold, z crease threshold (cos), w fade distance
    // god rays: x samples, y decay, z density, w on-screen weight
    settings: vec4<f32>,
};
@group(0) @binding(0) var scene_depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

fn depth_at(pixel: vec2<i32>) -> f32 {
    let size = vec2<i32>(params.size.xy);
    return textureLoad(scene_depth, clamp(pixel, vec2<i32>(0), size - vec2<i32>(1)), 0);
}

fn world_at(pixel: vec2<i32>, depth: f32) -> vec3<f32> {
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / params.size.xy;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = params.inverse_view_projection * ndc;
    return world.xyz / world.w;
}

@fragment
fn edges(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(position.xy);
    let w = i32(max(params.settings.x, 1.0));
    let d0 = depth_at(p);
    let dr = depth_at(p + vec2<i32>(w, 0));
    let dl = depth_at(p - vec2<i32>(w, 0));
    let du = depth_at(p - vec2<i32>(0, w));
    let dd = depth_at(p + vec2<i32>(0, w));
    let sky0 = d0 >= 1.0;
    let any_sky = dr >= 1.0 || dl >= 1.0 || du >= 1.0 || dd >= 1.0;
    if (sky0) {
        discard;
    }
    let p0 = world_at(p, d0);
    let distance0 = length(p0 - params.eye.xyz);
    var edge = 0.0;
    if (any_sky) {
        // Silhouette against the background, drawn on the object's side.
        edge = 1.0;
    } else {
        let pr = world_at(p + vec2<i32>(w, 0), dr);
        let pl = world_at(p - vec2<i32>(w, 0), dl);
        let pu = world_at(p - vec2<i32>(0, w), du);
        let pd = world_at(p + vec2<i32>(0, w), dd);
        // Depth discontinuity relative to the viewing distance.
        let far = max(
            max(abs(length(pr - params.eye.xyz) - distance0), abs(length(pl - params.eye.xyz) - distance0)),
            max(abs(length(pu - params.eye.xyz) - distance0), abs(length(pd - params.eye.xyz) - distance0)),
        );
        if (far > params.settings.y * distance0) {
            edge = 1.0;
        } else {
            // Creases: along each screen axis, the surface direction on one
            // side differs from the other. On a plane the five samples of a
            // row are collinear, so only real folds ink.
            let w2 = 2 * w;
            let prr = world_at(p + vec2<i32>(w2, 0), depth_at(p + vec2<i32>(w2, 0)));
            let pll = world_at(p - vec2<i32>(w2, 0), depth_at(p - vec2<i32>(w2, 0)));
            let puu = world_at(p - vec2<i32>(0, w2), depth_at(p - vec2<i32>(0, w2)));
            let pdd = world_at(p + vec2<i32>(0, w2), depth_at(p + vec2<i32>(0, w2)));
            let horizontal = dot(normalize(pl - pll), normalize(prr - pr));
            let vertical = dot(normalize(pu - puu), normalize(pdd - pd));
            if (min(horizontal, vertical) < params.settings.z) {
                edge = 1.0;
            }
        }
    }
    if (params.settings.w > 0.0) {
        edge *= 1.0 - smoothstep(params.settings.w * 0.7, params.settings.w, distance0);
    }
    if (edge <= 0.0) {
        discard;
    }
    return vec4<f32>(params.color.rgb, params.color.a * edge);
}

@fragment
fn god_rays(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = position.xy / params.size.xy;
    let samples = i32(params.settings.x);
    let step = (uv - params.size.zw) * params.settings.z / f32(max(samples, 1));
    var sample_uv = uv;
    var weight = 1.0;
    var sum = 0.0;
    var total = 0.0;
    for (var i = 0; i < samples; i++) {
        sample_uv -= step;
        let pixel = vec2<i32>(sample_uv * params.size.xy);
        var open = 0.0;
        if (all(sample_uv >= vec2<f32>(0.0)) && all(sample_uv <= vec2<f32>(1.0)) && depth_at(pixel) >= 1.0) {
            open = 1.0;
        }
        sum += open * weight;
        total += weight;
        weight *= params.settings.y;
    }
    let rays = sum / max(total, 1e-4) * params.settings.w;
    return vec4<f32>(params.color.rgb * rays, 1.0);
}
