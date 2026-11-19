// Shared Quasar 3D lighting: light and shadow records, their reserved
// bindings (group 0: 3 = lights, 5 = shadow depth array, 6 = shadow
// records, 21 = comparison sampler) and the forward shading loop. Included
// by render-3d's lit/surface shader and by other renderers (voxels) so every
// pass lights identically.
//
// `Lights.info`: x = light count, y = encode sRGB, z = clustered, w = unused.
// `fog`: rgb colour, a = enabled; `fogRange`: x start, y end (metres from the
// eye), z distance fog on. `fogHeight`: x base height, y density, z falloff,
// w scattering. `toon`: x bands, y softness, z rim, w shaft steps.
// Light records: `coneAttenuation` x/y inner/outer cone cosines, z falloff
// (0 smooth, 1 inverse square, 2 + e curve exponent e), w cookie bits.
// Clustered lighting splits space around the eye into a world-aligned
// grid (`clusterOrigin.xyz` minimum corner, `.w` cell size; `clusterDims.xyz`
// cells, `.w` first record holding cluster words). Cluster words are u32s
// packed 16 per `Light` record after the real lights: word 0 = global
// (directional) light count G, words 1..=G their indices, then per cell a
// (start word, count) pair, then the index lists. Fragments outside the grid
// shade every light, so clustering never changes the result.
struct Light { positionKind: vec4<f32>, directionRange: vec4<f32>, colorIntensity: vec4<f32>, coneAttenuation: vec4<f32> }
struct Lights { ambient: vec4<f32>, eye: vec4<f32>, info: vec4<f32>, fog: vec4<f32>, fogRange: vec4<f32>, fogHeight: vec4<f32>, toon: vec4<f32>, clusterOrigin: vec4<f32>, clusterDims: vec4<f32>, lights: array<Light> }
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
// Cookie multiplier for a spot light: `ray` runs from the light to the
// point, `axis` is the spot direction and `cos_outer` the cone edge.
fn cookie_hash(p:vec2<f32>)->f32 {let h=sin(dot(p,vec2<f32>(127.1,311.7)))*43758.547;return h-floor(h);}
fn light_cookie(packed:u32,ray:vec3<f32>,axis:vec3<f32>,cos_outer:f32)->f32 {
    let along=dot(ray,axis);if along<=1e-3 {return 1.;}
    var helper=vec3<f32>(0.,1.,0.);if abs(axis.y)>0.99 {helper=vec3<f32>(1.,0.,0.);}
    let u=safe_normal(cross(axis,helper));let v=cross(axis,u);
    let tan_outer=sqrt(max(1.-cos_outer*cos_outer,1e-6))/max(cos_outer,1e-3);
    var uv=vec2<f32>(dot(ray,u),dot(ray,v))/(along*tan_outer);
    let count=f32((packed>>4u)&255u);let contrast=f32((packed>>12u)&255u)/255.;
    let angle=f32(packed>>20u)/4096.*6.2831853;let c=cos(angle);let s=sin(angle);
    uv=vec2<f32>(uv.x*c-uv.y*s,uv.x*s+uv.y*c);
    var pattern=1.;let kind=packed&15u;
    if kind==1u {pattern=0.5+0.5*cos(length(uv)*count*6.2831853);}
    else if kind==2u {pattern=0.5+0.5*cos(uv.x*count*3.1415927);}
    else if kind==3u {pattern=0.5+0.5*cos(atan2(uv.y,uv.x)*count);}
    else {let p=uv*count;let i=floor(p);let f=p-i;let w=f*f*(3.-2.*f);
        let a=cookie_hash(i);let b=cookie_hash(i+vec2<f32>(1.,0.));let cc=cookie_hash(i+vec2<f32>(0.,1.));let d=cookie_hash(i+vec2<f32>(1.,1.));
        let top=mix(a,b,w.x);pattern=mix(top,mix(cc,d,w.x),w.y);}
    return 1.-contrast+contrast*pattern;
}
// Distance, cone and cookie attenuation of positional light `light` at
// `position` (0 outside its range); `l` receives the unit direction to it.
fn light_attenuation(light:Light,position:vec3<f32>,l:ptr<function,vec3<f32>>)->f32 {
    let delta=light.positionKind.xyz-position;let distance_squared=dot(delta,delta);let range=light.directionRange.w;
    *l=safe_normal(delta);
    var attenuation=0.;
    let falloff=light.coneAttenuation.z;
    if falloff>1.5 {
        attenuation=pow(max(1.-sqrt(distance_squared)/range,0.),falloff-2.);
        if distance_squared>=range*range {attenuation=0.;}
    } else {
        let window=max(1.-distance_squared/(range*range),0.);attenuation=window*window;
        if falloff>0.5 {attenuation/=max(distance_squared,0.01);}
    }
    if attenuation<=0. {return 0.;}
    if light.positionKind.w>1.5 {
        attenuation*=smoothstep(light.coneAttenuation.y,light.coneAttenuation.x,dot(-*l,light.directionRange.xyz));
        let packed=bitcast<u32>(light.coneAttenuation.w);
        if packed!=0u && attenuation>0. {attenuation*=light_cookie(packed,-*l,light.directionRange.xyz,light.coneAttenuation.y);}
    }
    return attenuation;
}
// Toon band of a light amount in [0, 1].
fn toon_band(x:f32)->f32 {
    let bands=max(lighting.toon.x,1.);let scaled=clamp(x,0.,1.)*bands;let base=floor(scaled);
    let soft=max(lighting.toon.y,1e-3)*0.5;
    return min((base+smoothstep(0.5-soft,0.5+soft,scaled-base))/bands,1.);
}
// One light's contribution; `view` is the unit direction to the eye.
fn light_response(i:i32,base:vec3<f32>,n:vec3<f32>,geometric_normal:vec3<f32>,position:vec3<f32>,view:vec3<f32>,shading:f32,specular:vec4<f32>,receives:bool)->vec3<f32> {
    let light=lighting.lights[i];var l=-light.directionRange.xyz;var attenuation=1.;
    if light.positionKind.w>0.5 {
        attenuation=light_attenuation(light,position,&l);
        if attenuation<=0. {return vec3<f32>(0.);}
    }
    let diffuse=max(dot(n,l),0.);
    let radiance=light.colorIntensity.rgb*light.colorIntensity.a;
    if shading>2.5 {
        // Toon: quantise the light reaching the surface (diffuse, falloff
        // and shadow together) so shadows fall on band edges too.
        let amount=diffuse*attenuation*shadow_visibility(i,geometric_normal,position,receives);
        var response=base*toon_band(amount)*radiance;
        if diffuse>0. && specular.a>1. {
            let highlight=pow(max(dot(n,safe_normal(l+view)),0.),specular.a);
            response+=specular.rgb*step(0.5,highlight*amount)*radiance;
        }
        return response;
    }
    var response=base*diffuse;
    if shading>1.5 && diffuse>0. {response+=specular.rgb*pow(max(dot(n,safe_normal(l+view)),0.),specular.a);}
    return response*radiance*attenuation*shadow_visibility(i,geometric_normal,position,receives);
}
// Forward shading of `base` at `position` with shading normal `n` and
// geometric normal `geometric_normal` (for shadow bias). `shading`: 0 unlit,
// 1 diffuse, 2 Blinn-Phong, 3 toon; `specular` = rgb + shininess.
fn lit_color(base:vec3<f32>,n:vec3<f32>,geometric_normal:vec3<f32>,position:vec3<f32>,shading:f32,specular:vec4<f32>,receives:bool)->vec3<f32> {
    if shading<=0.5 {return base;}
    let view=safe_normal(lighting.eye.xyz-position);
    var result=base*lighting.ambient.rgb;
    if shading>2.5 && lighting.toon.z>0. {
        result+=base*lighting.toon.z*smoothstep(0.55,0.8,1.-max(dot(n,view),0.));
    }
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
// Fraction of the view ray from the eye to `position` hidden by fog:
// distance fog and analytic exponential height fog.
fn fog_amount(position:vec3<f32>)->f32 {
    var keep=1.;
    let delta=position-lighting.eye.xyz;let distance=length(delta);
    if lighting.fogRange.z>0.5 {
        keep*=1.-clamp((distance-lighting.fogRange.x)/max(lighting.fogRange.y-lighting.fogRange.x,1e-3),0.,1.);
    }
    let density=lighting.fogHeight.y;
    if density>0. {
        let k=lighting.fogHeight.z;
        let at_eye=density*exp(clamp(-k*(lighting.eye.y-lighting.fogHeight.x),-80.,80.));
        let rise=k*delta.y;
        var depth=distance*at_eye;
        if abs(rise)>1e-4 {depth=distance*at_eye*(1.-exp(clamp(-rise,-80.,80.)))/rise;}
        keep*=exp(-max(depth,0.));
    }
    return 1.-keep;
}
// Fog density (extinction per metre) at `point`: distance fog spreads its
// fade evenly between start and end, height fog falls off with height.
fn fog_density(point:vec3<f32>)->f32 {
    var density=0.;
    if lighting.fogRange.z>0.5 {
        let distance=length(point-lighting.eye.xyz);
        if distance>=lighting.fogRange.x && distance<=lighting.fogRange.y {
            density+=1./max(lighting.fogRange.y-lighting.fogRange.x,1e-3);
        }
    }
    if lighting.fogHeight.y>0. {
        density+=lighting.fogHeight.y*exp(clamp(-lighting.fogHeight.z*(point.y-lighting.fogHeight.x),-80.,80.));
    }
    return density;
}
// Single scattering of light `i` towards the eye along the ray to
// `position`, with an isotropic phase function (1/4pi). Positional lights
// are sampled where the ray crosses their range; directional lights use the
// ray's optical depth, or shadow-marched density when shafts are on.
fn scatter_light(i:i32,position:vec3<f32>,dither:f32,optical_depth:f32)->vec3<f32> {
    let phase=0.0795775;
    let light=lighting.lights[i];
    let eye=lighting.eye.xyz;let delta=position-eye;let distance=length(delta);
    if distance<1e-4 {return vec3<f32>(0.);}
    let v=delta/distance;let radiance=light.colorIntensity.rgb*light.colorIntensity.a;
    if light.positionKind.w<0.5 {
        let steps=i32(lighting.toon.w);
        // In-scattering seen through the fog saturates at 1 - e^-depth.
        if steps<=0 {return radiance*phase*(1.-exp(-optical_depth));}
        let length=min(distance,200.);let step=length/f32(steps);
        var sum=0.;var seen=1.;
        for(var s=0;s<steps;s++) {
            let p=eye+v*((f32(s)+dither)*step);
            let density=fog_density(p)*step;
            sum+=shadow_visibility(i,vec3<f32>(0.),p,true)*seen*(1.-exp(-density));
            seen*=exp(-density);
        }
        return radiance*phase*sum;
    }
    // Clip the ray to the light's range sphere and sample it.
    let m=eye-light.positionKind.xyz;let b=dot(m,v);let range=light.directionRange.w;
    let h2=dot(m,m)-b*b;let s2=range*range-h2;
    if s2<=0. {return vec3<f32>(0.);}
    let s=sqrt(s2);let t0=max(-b-s,0.);let t1=min(-b+s,distance);
    if t1<=t0 {return vec3<f32>(0.);}
    var sum=0.;
    for(var k=0;k<6;k++) {
        let t=mix(t0,t1,(f32(k)+dither)/6.);
        let p=eye+v*t;var l=vec3<f32>(0.);
        // Fog between the eye and the sample dims what it scatters back.
        let seen=exp(-optical_depth*t/distance);
        sum+=light_attenuation(light,p,&l)*fog_density(p)*seen;
    }
    return radiance*phase*sum/6.*(t1-t0);
}
// Fog over a lit colour (linear, before any sRGB encode).
fn apply_fog(color:vec3<f32>,position:vec3<f32>)->vec3<f32> {
    if lighting.fog.a<0.5 {return color;}
    let f=fog_amount(position);
    var result=mix(color,lighting.fog.rgb,f);
    let scattering=lighting.fogHeight.w;
    if scattering>0. && f>1e-4 {
        let dither=fract(sin(dot(position,vec3<f32>(12.9898,78.233,37.719)))*43758.547);
        let depth=-log(max(1.-f,1e-6));
        var glow=vec3<f32>(0.);
        var clustered=false;
        if lighting.info.z>0.5 {
            let dims=vec3<i32>(lighting.clusterDims.xyz);
            let cell=vec3<i32>(floor((position-lighting.clusterOrigin.xyz)/lighting.clusterOrigin.w));
            if all(cell>=vec3<i32>(0)) && all(cell<dims) {
                clustered=true;
                let globals=light_word(0u);
                for(var j=0u;j<globals;j++) {glow+=scatter_light(i32(light_word(1u+j)),position,dither,depth);}
                let entry=1u+globals+2u*u32(cell.x+dims.x*(cell.y+dims.y*cell.z));
                let start=light_word(entry);let count=light_word(entry+1u);
                for(var j=0u;j<count;j++) {glow+=scatter_light(i32(light_word(start+j)),position,dither,depth);}
            }
        }
        if !clustered {
            for(var i=0;i<i32(lighting.info.x);i++) {glow+=scatter_light(i,position,dither,depth);}
        }
        result+=glow*scattering;
    }
    return result;
}
// Whether final colours are encoded to sRGB in the shader.
fn output_srgb()->bool {return lighting.info.y>0.5;}
