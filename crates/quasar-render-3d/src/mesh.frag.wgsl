// Migrated from Quasar's GLSL using upstream Naga 30.0.1.
diagnostic(off, derivative_uniformity);
struct FragmentOutput {
    @location(0) outputColor: vec4<f32>,
}

@group(0) @binding(0)
var image_texture: texture_2d<f32>;
@group(0) @binding(16)
var image_sampler: sampler;
var<private> uv_1: vec2<f32>;
var<private> color_1: vec4<f32>;
var<private> outputColor: vec4<f32>;

fn main_1() {
    let _e5: vec2<f32> = uv_1;
    let _e6: vec4<f32> = textureSample(image_texture, image_sampler, _e5);
    let _e7: vec4<f32> = color_1;
    outputColor = (_e6 * _e7);
    return;
}

@fragment
fn main(@location(0) uv: vec2<f32>, @location(1) color: vec4<f32>) -> FragmentOutput {
    uv_1 = uv;
    color_1 = color;
    main_1();
    let _e15: vec4<f32> = outputColor;
    return FragmentOutput(_e15);
}
