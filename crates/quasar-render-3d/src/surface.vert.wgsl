// Migrated from Quasar's GLSL using upstream Naga 30.0.1.
diagnostic(off, derivative_uniformity);
struct Vertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    uv: vec4<f32>,
}

struct Geometry {
    vertices: array<Vertex>,
}

struct InstanceData {
    viewProjection: mat4x4<f32>,
    model: mat4x4<f32>,
    tint: vec4<f32>,
    specularShininess: vec4<f32>,
    emissiveCutoff: vec4<f32>,
    options: vec4<f32>,
    normalMap: vec4<f32>,
}

struct Instances {
    items: array<InstanceData>,
}

struct Batch {
    first: vec4<u32>,
}

struct VertexOutput {
    @location(7) @interpolate(flat) instanceIndex: u32,
    @location(0) uv: vec2<f32>,
    @location(1) worldPosition: vec3<f32>,
    @location(2) worldNormal: vec3<f32>,
    @builtin(position) gl_Position: vec4<f32>,
}

@group(0) @binding(1)
var<storage> geometry: Geometry;
@group(0) @binding(7)
var<storage> instanceData: Instances;
@group(0) @binding(2)
var<uniform> batchData: Batch;
var<private> instanceIndex: u32;
var<private> uv: vec2<f32>;
var<private> worldPosition: vec3<f32>;
var<private> worldNormal: vec3<f32>;
var<private> gl_InstanceIndex_1: u32;
var<private> gl_VertexIndex_1: u32;
var<private> gl_Position: vec4<f32>;

fn main_1() {
    var v: Vertex;
    var world: vec4<f32>;

    let _e14: Batch = batchData;
    let _e17: u32 = gl_InstanceIndex_1;
    instanceIndex = (_e14.first.x + u32(_e17));
    let _e21: u32 = gl_VertexIndex_1;
    let _e24: Vertex = geometry.vertices[_e21];
    v = _e24;
    let _e26: u32 = instanceIndex;
    let _e29: InstanceData = instanceData.items[_e26];
    let _e31: Vertex = v;
    world = (_e29.model * _e31.position);
    let _e36: u32 = instanceIndex;
    let _e39: InstanceData = instanceData.items[_e36];
    let _e41: vec4<f32> = world;
    gl_Position = (_e39.viewProjection * _e41);
    let _e43: vec4<f32> = world;
    worldPosition = _e43.xyz;
    let _e45: u32 = instanceIndex;
    let _e48: InstanceData = instanceData.items[_e45];
    let _e61: Vertex = v;
    worldNormal = (transpose(_naga_inverse_3x3_f32(mat3x3<f32>(_e48.model[0].xyz, _e48.model[1].xyz, _e48.model[2].xyz))) * _e61.normal.xyz);
    let _e65: Vertex = v;
    uv = _e65.uv.xy;
    return;
}

@vertex
fn main(@builtin(instance_index) gl_InstanceIndex: u32, @builtin(vertex_index) gl_VertexIndex: u32) -> VertexOutput {
    gl_InstanceIndex_1 = gl_InstanceIndex;
    gl_VertexIndex_1 = gl_VertexIndex;
    main_1();
    let _e22: u32 = instanceIndex;
    let _e24: vec2<f32> = uv;
    let _e26: vec3<f32> = worldPosition;
    let _e28: vec3<f32> = worldNormal;
    let _e30: vec4<f32> = gl_Position;
    return VertexOutput(_e22, _e24, _e26, _e28, _e30);
}

fn _naga_inverse_3x3_f32(m: mat3x3<f32>) -> mat3x3<f32> {
    var adj: mat3x3<f32>;

    adj[0][0] =   (m[1][1] * m[2][2] - m[2][1] * m[1][2]);
    adj[1][0] = - (m[1][0] * m[2][2] - m[2][0] * m[1][2]);
    adj[2][0] =   (m[1][0] * m[2][1] - m[2][0] * m[1][1]);
    adj[0][1] = - (m[0][1] * m[2][2] - m[2][1] * m[0][2]);
    adj[1][1] =   (m[0][0] * m[2][2] - m[2][0] * m[0][2]);
    adj[2][1] = - (m[0][0] * m[2][1] - m[2][0] * m[0][1]);
    adj[0][2] =   (m[0][1] * m[1][2] - m[1][1] * m[0][2]);
    adj[1][2] = - (m[0][0] * m[1][2] - m[1][0] * m[0][2]);
    adj[2][2] =   (m[0][0] * m[1][1] - m[1][0] * m[0][1]);

    let det: f32 = (m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]));

    return adj * (1 / det);
}
