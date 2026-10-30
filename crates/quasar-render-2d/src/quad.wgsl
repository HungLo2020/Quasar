struct Quad { origin_x: vec4<f32>, axis_y: vec2<f32>, effects: vec2<u32>, uv: vec4<f32>, color: vec4<f32>, clip: vec4<f32> }
struct Frame { size: vec4<f32>, viewport: vec4<f32> }
@group(0) @binding(0) var<storage,read> quads: array<Quad>;
@group(0) @binding(1) var<uniform> frame: Frame;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;
@group(1) @binding(2) var palette: texture_2d<f32>;
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>, @location(1) tint: vec4<f32>, @location(2) logical: vec2<f32>,
    @location(3) @interpolate(flat) clip: vec4<f32>, @location(4) @interpolate(flat) effects: vec2<u32>,
}
@vertex fn vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Output {
    let corners = array<vec2<f32>,6>(vec2(0.,0.),vec2(1.,0.),vec2(0.,1.),vec2(0.,1.),vec2(1.,0.),vec2(1.,1.));
    let q = quads[instance]; let p = corners[vertex];
    let logical = q.origin_x.xy + p.x * q.origin_x.zw + p.y * q.axis_y;
    let screen = frame.viewport.xy + logical / frame.size.xy * frame.viewport.zw;
    return Output(vec4(screen.x * 2. - 1., 1. - screen.y * 2., 0., 1.), mix(q.uv.xy,q.uv.zw,p),q.color,logical,q.clip,q.effects);
}
@fragment fn fragment(input: Output) -> @location(0) vec4<f32> {
    if any(input.logical < input.clip.xy) || any(input.logical >= input.clip.zw) { discard; }
    var texel: vec4<f32>;
    if (input.effects.x & 1u) != 0u {
        let size = textureDimensions(image);
        let position = clamp(vec2<i32>(floor(input.uv * vec2<f32>(size))),vec2<i32>(0),vec2<i32>(size)-vec2<i32>(1));
        let index = i32(round(textureLoad(image,position,0).r * 255.));
        if index == i32((input.effects.x >> 1u) & 511u)-1 || index == i32((input.effects.x >> 10u) & 511u)-1 { discard; }
        texel = textureLoad(palette,vec2<i32>(index,0),0);
    } else { texel = textureSample(image,image_sampler,input.uv); }
    let color = texel * input.tint;
    if color.a * 255. + 0.00001 < f32(input.effects.y) { discard; }
    return color;
}
