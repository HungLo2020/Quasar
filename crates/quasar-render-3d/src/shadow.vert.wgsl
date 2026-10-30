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
    alpha: vec4<f32>,
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
var<private> gl_InstanceIndex_1: u32;
var<private> gl_VertexIndex_1: u32;
var<private> gl_Position: vec4<f32>;

fn main_1() {
    var v: Vertex;

    let _e12: Batch = batchData;
    let _e15: u32 = gl_InstanceIndex_1;
    instanceIndex = (_e12.first.x + u32(_e15));
    let _e19: u32 = gl_VertexIndex_1;
    let _e22: Vertex = geometry.vertices[_e19];
    v = _e22;
    let _e25: u32 = instanceIndex;
    let _e28: InstanceData = instanceData.items[_e25];
    let _e30: u32 = instanceIndex;
    let _e33: InstanceData = instanceData.items[_e30];
    let _e36: Vertex = v;
    gl_Position = ((_e28.viewProjection * _e33.model) * _e36.position);
    let _e39: Vertex = v;
    uv = _e39.uv.xy;
    return;
}

@vertex
fn main(@builtin(instance_index) gl_InstanceIndex: u32, @builtin(vertex_index) gl_VertexIndex: u32) -> VertexOutput {
    gl_InstanceIndex_1 = gl_InstanceIndex;
    gl_VertexIndex_1 = gl_VertexIndex;
    main_1();
    let _e18: u32 = instanceIndex;
    let _e20: vec2<f32> = uv;
    let _e22: vec4<f32> = gl_Position;
    return VertexOutput(_e18, _e20, _e22);
}
