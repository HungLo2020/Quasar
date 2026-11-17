// Projected box decals over an opaque scene. Each decal is a unit cube
// (-0.5..0.5) placed by `model`; scene depth inside the box is projected
// along the box's local Y onto the decal texture.
struct Decal {
    model: mat4x4<f32>,
    inverse: mat4x4<f32>,
    tint: vec4<f32>,
    // x: fade distance along local Y as a fraction of the half height
    params: vec4<f32>,
};
struct Frame {
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    // xy: target size in pixels
    size: vec4<f32>,
};
@group(0) @binding(0) var<storage, read> decals: array<Decal>;
@group(0) @binding(1) var<uniform> frame: Frame;
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(0) @binding(3) var image: texture_2d<f32>;
@group(0) @binding(4) var image_sampler: sampler;
@group(0) @binding(5) var<uniform> batch: vec4<u32>;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) decal: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOutput {
    // 12 triangles of a unit cube.
    var faces = array<vec3<f32>, 8>(
        vec3<f32>(-0.5, -0.5, -0.5), vec3<f32>(0.5, -0.5, -0.5), vec3<f32>(0.5, 0.5, -0.5), vec3<f32>(-0.5, 0.5, -0.5),
        vec3<f32>(-0.5, -0.5, 0.5), vec3<f32>(0.5, -0.5, 0.5), vec3<f32>(0.5, 0.5, 0.5), vec3<f32>(-0.5, 0.5, 0.5),
    );
    var indices = array<u32, 36>(
        0u, 2u, 1u, 0u, 3u, 2u, 4u, 5u, 6u, 4u, 6u, 7u,
        0u, 1u, 5u, 0u, 5u, 4u, 3u, 6u, 2u, 3u, 7u, 6u,
        0u, 4u, 7u, 0u, 7u, 3u, 1u, 2u, 6u, 1u, 6u, 5u,
    );
    let index = batch.x + instance;
    var out: VertexOutput;
    out.clip = frame.view_projection * decals[index].model * vec4<f32>(faces[indices[vertex]], 1.0);
    out.decal = index;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.clip.xy);
    let depth = textureLoad(scene_depth, pixel, 0);
    if (depth >= 1.0) {
        discard;
    }
    let ndc = vec4<f32>(
        in.clip.x / frame.size.x * 2.0 - 1.0,
        1.0 - in.clip.y / frame.size.y * 2.0,
        depth,
        1.0,
    );
    let world = frame.inverse_view_projection * ndc;
    let decal = decals[in.decal];
    let local = (decal.inverse * vec4<f32>(world.xyz / world.w, 1.0)).xyz;
    if (any(abs(local) > vec3<f32>(0.5))) {
        discard;
    }
    let uv = vec2<f32>(local.x + 0.5, 0.5 - local.z);
    let texel = textureSampleLevel(image, image_sampler, uv, 0.0);
    let fade_start = 0.5 * (1.0 - clamp(decal.params.x, 0.0, 1.0));
    let fade = 1.0 - smoothstep(fade_start, 0.5, abs(local.y));
    let color = texel * decal.tint;
    return vec4<f32>(color.rgb, color.a * fade);
}
