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
    light: vec4<f32>,
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
    @location(1) color: vec4<f32>,
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
var<private> color: vec4<f32>;
var<private> gl_InstanceIndex_1: u32;
var<private> gl_VertexIndex_1: u32;
var<private> gl_Position: vec4<f32>;

fn main_1() {
    var v: Vertex;
    var normal: vec3<f32>;
    var lighting: f32;

    let _e13: Batch = batchData;
    let _e16: u32 = gl_InstanceIndex_1;
    instanceIndex = (_e13.first.x + u32(_e16));
    let _e20: u32 = gl_VertexIndex_1;
    let _e23: Vertex = geometry.vertices[_e20];
    v = _e23;
    let _e26: u32 = instanceIndex;
    let _e29: InstanceData = instanceData.items[_e26];
    let _e31: u32 = instanceIndex;
    let _e34: InstanceData = instanceData.items[_e31];
    let _e37: Vertex = v;
    gl_Position = ((_e29.viewProjection * _e34.model) * _e37.position);
    let _e40: u32 = instanceIndex;
    let _e43: InstanceData = instanceData.items[_e40];
    let _e56: Vertex = v;
    normal = normalize((transpose(_naga_inverse_3x3_f32(mat3x3<f32>(_e43.model[0].xyz, _e43.model[1].xyz, _e43.model[2].xyz))) * _e56.normal.xyz));
    let _e64: vec3<f32> = normal;
    let _e65: u32 = instanceIndex;
    let _e68: InstanceData = instanceData.items[_e65];
    lighting = (0.3f + (0.7f * max(dot(_e64, normalize(_e68.light.xyz)), 0f)));
    let _e78: u32 = instanceIndex;
    let _e81: InstanceData = instanceData.items[_e78];
    let _e84: f32 = lighting;
    let _e85: vec3<f32> = (_e81.tint.xyz * _e84);
    let _e86: u32 = instanceIndex;
    let _e89: InstanceData = instanceData.items[_e86];
    color = vec4<f32>(_e85.x, _e85.y, _e85.z, _e89.tint.w);
    let _e96: Vertex = v;
    uv = _e96.uv.xy;
    return;
}

@vertex
fn main(@builtin(instance_index) gl_InstanceIndex: u32, @builtin(vertex_index) gl_VertexIndex: u32) -> VertexOutput {
    gl_InstanceIndex_1 = gl_InstanceIndex;
    gl_VertexIndex_1 = gl_VertexIndex;
    main_1();
    let _e20: u32 = instanceIndex;
    let _e22: vec2<f32> = uv;
    let _e24: vec4<f32> = color;
    let _e26: vec4<f32> = gl_Position;
    return VertexOutput(_e20, _e22, _e24, _e26);
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
