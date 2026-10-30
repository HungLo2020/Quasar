// Native forward lighting, normal mapping and explicit directional/spot shadows.
diagnostic(off, derivative_uniformity);
struct InstanceData {
    viewProjection: mat4x4<f32>, model: mat4x4<f32>, tint: vec4<f32>,
    specularShininess: vec4<f32>, emissiveCutoff: vec4<f32>, options: vec4<f32>, normalMap: vec4<f32>,
}
struct Instances { items: array<InstanceData> }
struct Light { positionKind: vec4<f32>, directionRange: vec4<f32>, colorIntensity: vec4<f32>, coneAttenuation: vec4<f32> }
struct Lights { ambient: vec4<f32>, eye: vec4<f32>, info: vec4<f32>, lights: array<Light> }
struct Shadow { viewProjection: mat4x4<f32>, settings: vec4<f32>, sampling: vec4<f32> }
struct Shadows { info: vec4<f32>, maps: array<Shadow> }
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(16) var image_sampler: sampler;
@group(0) @binding(3) var<storage,read> lighting: Lights;
@group(0) @binding(4) var normal_image: texture_2d<f32>;
@group(0) @binding(20) var normal_sampler: sampler;
@group(0) @binding(5) var shadow_image: texture_depth_2d_array;
@group(0) @binding(21) var shadow_sampler: sampler_comparison;
@group(0) @binding(6) var<storage,read> shadows: Shadows;
@group(0) @binding(7) var<storage,read> instanceData: Instances;
fn safe_normal(v:vec3<f32>) -> vec3<f32> {
    let scale=max(max(abs(v.x),abs(v.y)),abs(v.z)); let scaled=v/max(scale,1e-30);
    return scaled*inverseSqrt(max(dot(scaled,scaled),1e-12));
}
fn decode_srgb(c:vec3<f32>)->vec3<f32> {return mix(c/12.92,pow((c+0.055)/1.055,vec3<f32>(2.4)),step(vec3<f32>(0.04045),c));}
fn encode_srgb(value:vec3<f32>)->vec3<f32> {let c=max(value,vec3<f32>(0.));return mix(c*12.92,1.055*pow(c,vec3<f32>(1./2.4))-0.055,step(vec3<f32>(0.0031308),c));}
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
fn visibility(light_index:i32,n:vec3<f32>,position:vec3<f32>,draw:InstanceData)->f32 {
    if fract(draw.options.z/2.)*2.<0.5 {return 1.;}
    for(var j=0;j<i32(shadows.info.x);j++) {
        let map=shadows.maps[j];if i32(map.settings.x)!=light_index {continue;}
        let projected=map.viewProjection*vec4<f32>(position+n*map.settings.w,1.);
        if projected.w<=0. {return 1.;}
        let p=projected.xyz/projected.w;var coord=p.xy*0.5+0.5;coord.y=1.-coord.y;
        if p.z<0. || p.z>1. || any(coord<vec2<f32>(0.)) || any(coord>vec2<f32>(1.)) {return 1.;}
        var sum=0.;var weight=0.;let radius=i32(map.sampling.y);
        for(var y=-radius;y<=radius;y++) {for(var x=-radius;x<=radius;x++) {
            let tap=coord+vec2<f32>(f32(x),f32(y))*map.sampling.x;
            if any(tap<vec2<f32>(0.)) || any(tap>vec2<f32>(1.)) {sum+=1.;}
            else {sum+=textureSampleCompareLevel(shadow_image,shadow_sampler,tap,i32(map.settings.y),p.z-map.settings.z);}
            weight+=1.;
        }}
        return mix(1.,sum/weight,map.sampling.z);
    }
    return 1.;
}
@fragment fn main(@location(0) uv:vec2<f32>,@location(1) position:vec3<f32>,@location(2) world_normal:vec3<f32>,@location(7) @interpolate(flat) instance:u32,@builtin(front_facing) front:bool)->@location(0) vec4<f32> {
    let draw=instanceData.items[instance];var texel=textureSample(image,image_sampler,uv);
    if draw.options.y>0.5 {texel=vec4<f32>(decode_srgb(texel.rgb),texel.a);}
    var base=texel*draw.tint;if draw.options.z>=4. {base.a=1.;}
    if base.a<draw.emissiveCutoff.a {discard;}
    var result=base.rgb;
    if draw.options.x>0.5 {
        var geometric_normal=safe_normal(world_normal);
        if (u32(draw.options.z)&2u)!=0u && !front {geometric_normal=-geometric_normal;}
        let n=material_normal(geometric_normal,uv,position,draw);let view=safe_normal(lighting.eye.xyz-position);
        result=base.rgb*lighting.ambient.rgb;
        for(var i=0;i<i32(lighting.info.x);i++) {
            let light=lighting.lights[i];var l=-light.directionRange.xyz;var attenuation=1.;
            if light.positionKind.w>0.5 {
                let delta=light.positionKind.xyz-position;let distance_squared=dot(delta,delta);let range=light.directionRange.w;l=safe_normal(delta);
                let window=max(1.-distance_squared/(range*range),0.);attenuation=window*window;
                if light.coneAttenuation.z>0.5 {attenuation/=max(distance_squared,0.01);}
                if light.positionKind.w>1.5 {attenuation*=smoothstep(light.coneAttenuation.y,light.coneAttenuation.x,dot(-l,light.directionRange.xyz));}
            }
            let diffuse=max(dot(n,l),0.);var response=base.rgb*diffuse;
            if draw.options.x>1.5 && diffuse>0. {response+=draw.specularShininess.rgb*pow(max(dot(n,safe_normal(l+view)),0.),draw.specularShininess.a);}
            result+=response*light.colorIntensity.rgb*light.colorIntensity.a*attenuation*visibility(i,geometric_normal,position,draw);
        }
    }
    result+=draw.emissiveCutoff.rgb;if lighting.info.y>0.5 {result=encode_srgb(result);}
    return vec4<f32>(result,base.a);
}
