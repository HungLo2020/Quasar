// Migrated from Quasar's GLSL using upstream Naga 30.0.1.
diagnostic(off, derivative_uniformity);
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

@group(0) @binding(0)
var image_texture: texture_2d<f32>;
@group(0) @binding(16)
var image_sampler: sampler;
@group(0) @binding(7)
var<storage> instanceData: Instances;
@group(0) @binding(2)
var<uniform> batchData: Batch;
var<private> instanceIndex_1: u32;
var<private> uv_1: vec2<f32>;

fn main_1() {
    let _e10: vec2<f32> = uv_1;
    let _e11: vec4<f32> = textureSample(image_texture, image_sampler, _e10);
    let _e13: u32 = instanceIndex_1;
    let _e16: InstanceData = instanceData.items[_e13];
    let _e20: u32 = instanceIndex_1;
    let _e23: InstanceData = instanceData.items[_e20];
    if ((_e11.w * _e16.alpha.y) < _e23.alpha.x) {
        discard;
    } else {
        return;
    }
}

@fragment
fn main(@location(7) @interpolate(flat) instanceIndex: u32, @location(0) uv: vec2<f32>) {
    instanceIndex_1 = instanceIndex;
    uv_1 = uv;
    main_1();
    return;
}
