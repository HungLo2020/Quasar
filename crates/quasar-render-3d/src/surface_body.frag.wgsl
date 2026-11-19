// Native forward lighting, normal mapping and explicit directional/spot shadows.
struct InstanceData {
    viewProjection: mat4x4<f32>, model: mat4x4<f32>, tint: vec4<f32>,
    specularShininess: vec4<f32>, emissiveCutoff: vec4<f32>, options: vec4<f32>, normalMap: vec4<f32>,
    extra: vec4<f32>,
}
struct Instances { items: array<InstanceData> }
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(16) var image_sampler: sampler;
@group(0) @binding(4) var normal_image: texture_2d<f32>;
@group(0) @binding(20) var normal_sampler: sampler;
@group(0) @binding(22) var emissive_image: texture_2d<f32>;
@group(0) @binding(7) var<storage,read> instanceData: Instances;
fn material_normal(n:vec3<f32>,uv:vec2<f32>,position:vec3<f32>,draw:InstanceData)->vec3<f32> {
    if draw.options.w<0.5 || draw.normalMap.z<=0. {return n;}
    let mapped_uv=uv*draw.normalMap.xy;
    let dx=dpdx(mapped_uv);let dy=dpdy(mapped_uv);let px=dpdx(position);let py=dpdy(position);
    let determinant=dx.x*dy.y-dx.y*dy.x;
    if abs(determinant)<1e-10 {return n;}
    var tangent=(px*dy.y-py*dx.y)/determinant;let bitangent=(py*dx.x-px*dy.x)/determinant;
    tangent=safe_normal(tangent-n*dot(n,tangent));if dot(tangent,tangent)<0.1 {return n;}
    let handedness=select(1.,-1.,dot(cross(n,tangent),bitangent)<0.);
    var sampled=textureSample(normal_image,normal_sampler,mapped_uv).xyz*2.-1.;
    sampled=vec3<f32>(sampled.xy*draw.normalMap.z,sampled.z);sampled.y*=draw.normalMap.w;
    return safe_normal(tangent*sampled.x+cross(n,tangent)*handedness*sampled.y+n*sampled.z);
}
@fragment fn main(@location(0) uv:vec2<f32>,@location(1) position:vec3<f32>,@location(2) world_normal:vec3<f32>,@location(3) vertex_color:vec4<f32>,@location(7) @interpolate(flat) instance:u32,@builtin(front_facing) front:bool)->@location(0) vec4<f32> {
    let draw=instanceData.items[instance];var texel=textureSample(image,image_sampler,uv);
    if draw.options.y>0.5 {texel=vec4<f32>(decode_srgb(texel.rgb),texel.a);}
    var base=texel*draw.tint*vertex_color;if draw.options.z>=4. {base.a=1.;}
    if base.a<draw.emissiveCutoff.a {discard;}
    var geometric_normal=safe_normal(world_normal);
    if (u32(draw.options.z)&2u)!=0u && !front {geometric_normal=-geometric_normal;}
    var n=geometric_normal;
    if draw.options.x>0.5 {n=material_normal(geometric_normal,uv,position,draw);}
    let receives=fract(draw.options.z/2.)*2.>=0.5;
    var result=lit_color(base.rgb,n,geometric_normal,position,draw.options.x,draw.specularShininess,receives);
    var glow=textureSample(emissive_image,image_sampler,uv).rgb;if draw.extra.y>0.5 {glow=decode_srgb(glow);}
    result+=draw.emissiveCutoff.rgb*glow;result=apply_fog(result,position);if output_srgb() {result=encode_srgb(result);}
    return vec4<f32>(result,base.a);
}
