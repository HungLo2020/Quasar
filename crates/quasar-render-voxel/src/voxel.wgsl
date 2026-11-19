// Quasar voxel pipelines: vertex pulling for packed quads, model vertices
// and smooth vertices; lighting through the shared render-3d module.
// Prepended at build time: diagnostic directive, LIGHTING_WGSL, WGSL_FORMATS.

struct Instance {
    origin: vec3<f32>,
    scale: f32,
    grid: u32,
    flags: u32,
    _pad0: u32,
    _pad1: u32,
};
struct GridData {
    model: mat4x4<f32>,
};
struct MaterialRecord {
    layer: u32,
    flags: u32,
    cutoff: f32,
    scale: f32,
    tint: vec4<f32>,
    emissive: vec4<f32>,
};
struct Frame {
    view_projection: mat4x4<f32>,
    // x: ao strength, y: voxel light on, z: sky brightness, w: shading model
    shading: vec4<f32>,
    // rgb: specular, a: shininess
    specular: vec4<f32>,
    // rgb: fog colour, a: fog enabled (fog itself is applied from the
    // shared lighting header)
    fog_color: vec4<f32>,
    // x: fog start, y: fog end, z: minimum light, w: reserved
    fog_range: vec4<f32>,
};

// Set per pipeline: 0 opaque, 1 cutout, 2 translucent.
override LAYER: f32 = 0.0;

@group(0) @binding(0) var material_images: texture_2d_array<f32>;
@group(0) @binding(1) var material_sampler: sampler;
@group(0) @binding(2) var<storage, read> materials: array<MaterialRecord>;
@group(0) @binding(7) var<storage, read> instances: array<Instance>;
@group(0) @binding(8) var<storage, read> geometry: array<u32>;
@group(0) @binding(9) var<uniform> frame: Frame;
@group(0) @binding(10) var<storage, read> grids: array<GridData>;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) position: vec3<f32>,
    @location(3) @interpolate(flat) material0: u32,
    @location(4) @interpolate(flat) material1: u32,
    @location(5) blend: f32,
    @location(6) ao: f32,
    @location(7) light: vec2<f32>,
    @location(8) tint: vec4<f32>,
    @location(9) local: vec3<f32>,
    @location(10) @interpolate(flat) smooth_surface: u32,
};

fn place(instance_index: u32, local: vec3<f32>, normal: vec3<f32>) -> VertexOutput {
    let instance = instances[instance_index];
    let model = grids[instance.grid].model;
    let voxel = instance.origin + local * instance.scale;
    let world = model * vec4<f32>(voxel, 1.0);
    var out: VertexOutput;
    out.clip = frame.view_projection * world;
    out.position = world.xyz;
    out.normal = normalize((model * vec4<f32>(normal, 0.0)).xyz);
    out.local = voxel;
    out.tint = vec4<f32>(1.0);
    out.blend = 0.0;
    out.smooth_surface = 0u;
    return out;
}

fn unpack_tint(t: u32) -> vec4<f32> {
    return vec4<f32>(f32(t & 255u), f32((t >> 8u) & 255u), f32((t >> 16u) & 255u), f32(t >> 24u)) / 255.0;
}

@vertex
fn vs_packed(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOutput {
    let quad = vertex / 6u;
    let w0 = geometry[quad * 3u];
    let corner = qv_quad_corner(w0, vertex % 6u);
    let v = qv_packed_quad_vertex(w0, geometry[quad * 3u + 1u], geometry[quad * 3u + 2u], corner);
    var out = place(instance, v.position, v.normal);
    out.uv = v.uv;
    out.material0 = v.material;
    out.material1 = v.material;
    out.ao = v.ao;
    out.light = v.light;
    return out;
}

@vertex
fn vs_model(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOutput {
    let order = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let index = (vertex / 6u) * 4u + order[vertex % 6u];
    let base = index * 5u;
    let v = qv_model_vertex(geometry[base], geometry[base + 1u], geometry[base + 2u], geometry[base + 3u], geometry[base + 4u]);
    var out = place(instance, v.position, v.normal);
    out.uv = v.uv;
    out.material0 = v.material;
    out.material1 = v.material;
    out.ao = v.ao;
    out.light = v.light;
    out.tint = unpack_tint(geometry[base + 4u]);
    return out;
}

@vertex
fn vs_smooth(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOutput {
    let base = vertex * 4u;
    let v = qv_smooth_vertex(geometry[base], geometry[base + 1u], geometry[base + 2u], geometry[base + 3u]);
    var out = place(instance, v.position, v.normal);
    out.uv = vec2<f32>(0.0);
    out.material0 = v.material0;
    out.material1 = v.material1;
    out.blend = v.blend;
    out.ao = 1.0;
    out.light = vec2<f32>(0.0, 1.0);
    out.smooth_surface = 1u;
    return out;
}

fn sample_material(material: u32, uv: vec2<f32>) -> vec4<f32> {
    let record = materials[material];
    return textureSample(material_images, material_sampler, uv, record.layer) * record.tint;
}

fn triplanar(material: u32, local: vec3<f32>, normal: vec3<f32>) -> vec4<f32> {
    let record = materials[material];
    let p = local * record.scale;
    var w = abs(normal);
    w = w / max(w.x + w.y + w.z, 1e-5);
    let x = textureSample(material_images, material_sampler, p.zy, record.layer);
    let y = textureSample(material_images, material_sampler, p.xz, record.layer);
    let z = textureSample(material_images, material_sampler, p.xy, record.layer);
    return (x * w.x + y * w.y + z * w.z) * record.tint;
}

// Per-texel emission: `emissive.rgb` scaled by the mask layer when one is
// set (`emissive.w` = layer + 1). Explicit-level sampling keeps it legal in
// non-uniform control flow.
fn emission(material: u32, uv: vec2<f32>, local: vec3<f32>, normal: vec3<f32>, smooth_surface: bool) -> vec3<f32> {
    let record = materials[material];
    if (record.emissive.w < 0.5) {
        return record.emissive.rgb;
    }
    let layer = i32(record.emissive.w - 0.5);
    if (!smooth_surface) {
        return record.emissive.rgb * textureSampleLevel(material_images, material_sampler, uv, layer, 0.0).rgb;
    }
    let p = local * record.scale;
    var w = abs(normal);
    w = w / max(w.x + w.y + w.z, 1e-5);
    let x = textureSampleLevel(material_images, material_sampler, p.zy, layer, 0.0).rgb;
    let y = textureSampleLevel(material_images, material_sampler, p.xz, layer, 0.0).rgb;
    let z = textureSampleLevel(material_images, material_sampler, p.xy, layer, 0.0).rgb;
    return record.emissive.rgb * (x * w.x + y * w.y + z * w.z);
}

@fragment
fn fs_main(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    if (!front) { n = -n; }
    // Sample both paths unconditionally to keep derivatives uniform.
    let flat_colour = sample_material(in.material0, in.uv);
    let smooth0 = triplanar(in.material0, in.local, n);
    let smooth1 = triplanar(in.material1, in.local, n);
    var albedo = flat_colour;
    if (in.smooth_surface == 1u) {
        albedo = mix(smooth0, smooth1, in.blend);
    }
    albedo = albedo * in.tint;
    let record = materials[in.material0];
    if ((record.flags & 1u) != 0u && albedo.a < record.cutoff) { discard; }
    let ao = mix(1.0 - frame.shading.x, 1.0, in.ao);
    var brightness = 1.0;
    if (frame.shading.y > 0.5) {
        let level = max(in.light.x, in.light.y * frame.shading.z);
        brightness = mix(frame.fog_range.z, 1.0, level * level);
    }
    let base = albedo.rgb * ao * brightness;
    var result = lit_color(base, n, n, in.position, frame.shading.w, frame.specular, true);
    result += emission(in.material0, in.uv, in.local, n, in.smooth_surface == 1u);
    result = apply_fog(result, in.position);
    if (output_srgb()) { result = encode_srgb(result); }
    var alpha = albedo.a;
    if (LAYER < 1.5) { alpha = 1.0; }
    return vec4<f32>(result, alpha);
}

// Outline boxes for picking and placement previews: 12 edges, 24 vertices.
struct OverlayBox {
    min: vec4<f32>,
    max: vec4<f32>,
    color: vec4<f32>,
    grid: vec4<u32>,
};
@group(1) @binding(0) var<storage, read> boxes: array<OverlayBox>;

struct LineOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_overlay(@builtin(vertex_index) vertex: u32) -> LineOutput {
    let b = boxes[vertex / 24u];
    let e = vertex % 24u;
    // Edge endpoints as corner bit patterns (x, y, z).
    let corners = array<u32, 24>(0u, 1u, 2u, 3u, 4u, 5u, 6u, 7u, 0u, 2u, 1u, 3u, 4u, 6u, 5u, 7u, 0u, 4u, 1u, 5u, 2u, 6u, 3u, 7u);
    let c = corners[e];
    let p = vec3<f32>(
        select(b.min.x, b.max.x, (c & 1u) != 0u),
        select(b.min.y, b.max.y, (c & 2u) != 0u),
        select(b.min.z, b.max.z, (c & 4u) != 0u),
    );
    let world = grids[b.grid.x].model * vec4<f32>(p, 1.0);
    var out: LineOutput;
    out.clip = frame.view_projection * world;
    out.color = b.color;
    return out;
}

@fragment
fn fs_overlay(in: LineOutput) -> @location(0) vec4<f32> {
    var c = in.color.rgb;
    if (output_srgb()) { c = encode_srgb(c); }
    return vec4<f32>(c, in.color.a);
}
