// Linear-blend skinning into a regular Quasar mesh vertex buffer. The layout
// matches `Mesh::upload`: position.xyz1, normal.xyz0, uv.xy00.

struct MeshVertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    uv: vec4<f32>,
};

struct Influence {
    joints: vec4<u32>,
    weights: vec4<f32>,
};

struct Params {
    vertex_count: u32,
    joint_count: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var<storage, read> bind_pose: array<MeshVertex>;
@group(0) @binding(1) var<storage, read> influences: array<Influence>;
@group(0) @binding(2) var<storage, read> palette: array<mat4x4<f32>>;
@group(0) @binding(3) var<storage, read_write> skinned: array<MeshVertex>;
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= params.vertex_count) {
        return;
    }
    let vertex = bind_pose[index];
    let influence = influences[index];
    let last = params.joint_count - 1u;
    var skin = palette[min(influence.joints.x, last)] * influence.weights.x;
    skin += palette[min(influence.joints.y, last)] * influence.weights.y;
    skin += palette[min(influence.joints.z, last)] * influence.weights.z;
    skin += palette[min(influence.joints.w, last)] * influence.weights.w;
    let position = skin * vec4<f32>(vertex.position.xyz, 1.0);
    let normal = (skin * vec4<f32>(vertex.normal.xyz, 0.0)).xyz;
    let length_squared = dot(normal, normal);
    var unit = vertex.normal.xyz;
    if (length_squared > 1e-12) {
        unit = normal * inverseSqrt(length_squared);
    }
    skinned[index] = MeshVertex(
        vec4<f32>(position.xyz / max(position.w, 1e-6), 1.0),
        vec4<f32>(unit, 0.0),
        vertex.uv,
    );
}
