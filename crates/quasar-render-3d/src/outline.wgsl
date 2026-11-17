// Screen-space outlines and see-through silhouettes.
// Pass 1 writes a mask (style id + 1, bit 7 set where the mesh is hidden by
// scene depth); pass 2 composites outline rings and occluded fills.
struct Vertex { position: vec4<f32>, normal: vec4<f32>, uv: vec4<f32> };
struct Instance {
    clip: mat4x4<f32>,
    // x: vertex base, y: style id
    info: vec4<f32>,
};
@group(0) @binding(0) var<storage, read> vertices: array<Vertex>;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<uniform> batch: vec4<u32>;

struct MaskOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) style: u32,
};

@vertex
fn vs_mask(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> MaskOutput {
    let item = instances[batch.x + instance];
    var out: MaskOutput;
    out.clip = item.clip * vec4<f32>(vertices[vertex + u32(item.info.x)].position.xyz, 1.0);
    out.style = u32(item.info.y);
    return out;
}

@fragment
fn fs_visible(in: MaskOutput) -> @location(0) u32 {
    return in.style + 1u;
}

@fragment
fn fs_occluded(in: MaskOutput) -> @location(0) u32 {
    return (in.style + 1u) | 128u;
}

// --- composite ---
struct Style {
    outline: vec4<f32>,
    occluded_outline: vec4<f32>,
    occluded_fill: vec4<f32>,
    // x: width in pixels
    width: vec4<f32>,
};
struct Styles { radius: vec4<f32>, items: array<Style, 64> };
@group(0) @binding(0) var mask: texture_2d<u32>;
@group(0) @binding(1) var<uniform> styles: Styles;

@vertex
fn vs_full(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_composite(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(mask));
    let pixel = vec2<i32>(position.xy);
    let here = textureLoad(mask, pixel, 0).r;
    if (here != 0u) {
        if ((here & 128u) != 0u) {
            return styles.items[(here & 127u) - 1u].occluded_fill;
        }
        discard;
    }
    let radius = i32(styles.radius.x);
    var best = 1e9;
    var found = 0u;
    for (var y = -radius; y <= radius; y++) {
        for (var x = -radius; x <= radius; x++) {
            let p = pixel + vec2<i32>(x, y);
            if (any(p < vec2<i32>(0)) || any(p >= size)) { continue; }
            let m = textureLoad(mask, p, 0).r;
            if (m == 0u) { continue; }
            let d = f32(x * x + y * y);
            let w = styles.items[(m & 127u) - 1u].width.x;
            if (d <= w * w + 0.5 && d < best) {
                best = d;
                found = m;
            }
        }
    }
    if (found == 0u) {
        discard;
    }
    let style = styles.items[(found & 127u) - 1u];
    if ((found & 128u) != 0u) {
        return style.occluded_outline;
    }
    return style.outline;
}
