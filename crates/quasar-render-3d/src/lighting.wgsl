// Shared Quasar 3D lighting: light and shadow records, their reserved
// bindings (group 0: 3 = lights, 5 = shadow depth array, 6 = shadow
// records, 21 = comparison sampler) and the forward shading loop. Included
// by render-3d's lit/surface shader and by other renderers (voxels) so every
// pass lights identically.
//
// `Lights.info`: x = light count, y = encode sRGB, z = clustered, w = unused.
// `fog`: rgb colour, a = enabled; `fogRange`: x start, y end (metres from the
// eye). Clustered lighting splits space around the eye into a world-aligned
// grid (`clusterOrigin.xyz` minimum corner, `.w` cell size; `clusterDims.xyz`
// cells, `.w` first record holding cluster words). Cluster words are u32s
// packed 16 per `Light` record after the real lights: word 0 = global
// (directional) light count G, words 1..=G their indices, then per cell a
// (start word, count) pair, then the index lists. Fragments outside the grid
// shade every light, so clustering never changes the result.
struct Light { positionKind: vec4<f32>, directionRange: vec4<f32>, colorIntensity: vec4<f32>, coneAttenuation: vec4<f32> }
struct Lights { ambient: vec4<f32>, eye: vec4<f32>, info: vec4<f32>, fog: vec4<f32>, fogRange: vec4<f32>, clusterOrigin: vec4<f32>, clusterDims: vec4<f32>, lights: array<Light> }
struct Shadow { viewProjection: mat4x4<f32>, settings: vec4<f32>, sampling: vec4<f32> }
struct Shadows { info: vec4<f32>, maps: array<Shadow> }
@group(0) @binding(3) var<storage,read> lighting: Lights;
@group(0) @binding(5) var shadow_image: texture_depth_2d_array;
@group(0) @binding(21) var shadow_sampler: sampler_comparison;
@group(0) @binding(6) var<storage,read> shadows: Shadows;
fn safe_normal(v:vec3<f32>) -> vec3<f32> {
    let scale=max(max(abs(v.x),abs(v.y)),abs(v.z)); let scaled=v/max(scale,1e-30);
    return scaled*inverseSqrt(max(dot(scaled,scaled),1e-12));
}
fn decode_srgb(c:vec3<f32>)->vec3<f32> {return mix(c/12.92,pow((c+0.055)/1.055,vec3<f32>(2.4)),step(vec3<f32>(0.04045),c));}
fn encode_srgb(value:vec3<f32>)->vec3<f32> {let c=max(value,vec3<f32>(0.));return mix(c*12.92,1.055*pow(c,vec3<f32>(1./2.4))-0.055,step(vec3<f32>(0.0031308),c));}
fn shadow_visibility(light_index:i32,n:vec3<f32>,position:vec3<f32>,receives:bool)->f32 {
    if !receives {return 1.;}
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
fn light_word(index:u32)->u32 {
    let record=lighting.lights[u32(lighting.clusterDims.w)+index/16u];
    let k=index%16u;
    var v=record.positionKind;
    if k>=12u {v=record.coneAttenuation;} else if k>=8u {v=record.colorIntensity;} else if k>=4u {v=record.directionRange;}
    return bitcast<u32>(v[k%4u]);
}
// One light's contribution; `view` is the unit direction to the eye.
fn light_response(i:i32,base:vec3<f32>,n:vec3<f32>,geometric_normal:vec3<f32>,position:vec3<f32>,view:vec3<f32>,shading:f32,specular:vec4<f32>,receives:bool)->vec3<f32> {
    let light=lighting.lights[i];var l=-light.directionRange.xyz;var attenuation=1.;
    if light.positionKind.w>0.5 {
        let delta=light.positionKind.xyz-position;let distance_squared=dot(delta,delta);let range=light.directionRange.w;l=safe_normal(delta);
        let window=max(1.-distance_squared/(range*range),0.);attenuation=window*window;
        if attenuation<=0. {return vec3<f32>(0.);}
        if light.coneAttenuation.z>0.5 {attenuation/=max(distance_squared,0.01);}
        if light.positionKind.w>1.5 {attenuation*=smoothstep(light.coneAttenuation.y,light.coneAttenuation.x,dot(-l,light.directionRange.xyz));}
    }
    let diffuse=max(dot(n,l),0.);var response=base*diffuse;
    if shading>1.5 && diffuse>0. {response+=specular.rgb*pow(max(dot(n,safe_normal(l+view)),0.),specular.a);}
    return response*light.colorIntensity.rgb*light.colorIntensity.a*attenuation*shadow_visibility(i,geometric_normal,position,receives);
}
// Forward shading of `base` at `position` with shading normal `n` and
// geometric normal `geometric_normal` (for shadow bias). `shading`: 0 unlit,
// 1 diffuse, 2 Blinn-Phong; `specular` = rgb + shininess.
fn lit_color(base:vec3<f32>,n:vec3<f32>,geometric_normal:vec3<f32>,position:vec3<f32>,shading:f32,specular:vec4<f32>,receives:bool)->vec3<f32> {
    if shading<=0.5 {return base;}
    let view=safe_normal(lighting.eye.xyz-position);
    var result=base*lighting.ambient.rgb;
    if lighting.info.z>0.5 {
        let dims=vec3<i32>(lighting.clusterDims.xyz);
        let cell=vec3<i32>(floor((position-lighting.clusterOrigin.xyz)/lighting.clusterOrigin.w));
        if all(cell>=vec3<i32>(0)) && all(cell<dims) {
            let globals=light_word(0u);
            for(var j=0u;j<globals;j++) {result+=light_response(i32(light_word(1u+j)),base,n,geometric_normal,position,view,shading,specular,receives);}
            let entry=1u+globals+2u*u32(cell.x+dims.x*(cell.y+dims.y*cell.z));
            let start=light_word(entry);let count=light_word(entry+1u);
            for(var j=0u;j<count;j++) {result+=light_response(i32(light_word(start+j)),base,n,geometric_normal,position,view,shading,specular,receives);}
            return result;
        }
    }
    for(var i=0;i<i32(lighting.info.x);i++) {result+=light_response(i,base,n,geometric_normal,position,view,shading,specular,receives);}
    return result;
}
// Distance fog towards the eye, in linear colour, before any sRGB encode.
fn apply_fog(color:vec3<f32>,position:vec3<f32>)->vec3<f32> {
    if lighting.fog.a<0.5 {return color;}
    let f=clamp((length(position-lighting.eye.xyz)-lighting.fogRange.x)/max(lighting.fogRange.y-lighting.fogRange.x,1e-3),0.,1.);
    return mix(color,lighting.fog.rgb,f);
}
// Whether final colours are encoded to sRGB in the shader.
fn output_srgb()->bool {return lighting.info.y>0.5;}
