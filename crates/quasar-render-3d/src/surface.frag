#version 450
layout(set=0,binding=0) uniform sampler2D image;
layout(set=0,binding=2,std140) uniform Draw {
    mat4 viewProjection; mat4 model; vec4 tint; vec4 specularShininess; vec4 emissiveCutoff; vec4 options; vec4 normalMap;
} drawData;
struct Light { vec4 positionKind; vec4 directionRange; vec4 colorIntensity; vec4 coneAttenuation; };
layout(set=0,binding=3,std430) readonly buffer Lights { vec4 ambient; vec4 eye; vec4 info; Light lights[]; } lighting;
layout(set=0,binding=4) uniform sampler2D normalImage;
layout(set=0,binding=5) uniform sampler2DArrayShadow shadowImage;
struct Shadow { mat4 viewProjection; vec4 settings; vec4 sampling; };
layout(set=0,binding=6,std430) readonly buffer Shadows { vec4 info; Shadow maps[]; } shadows;
layout(location=0) in vec2 uv;
layout(location=1) in vec3 worldPosition;
layout(location=2) in vec3 worldNormal;
layout(location=0) out vec4 outputColor;
vec3 safeNormal(vec3 v) {
    float scale=max(max(abs(v.x),abs(v.y)),abs(v.z));
    vec3 scaled=v/max(scale,1e-30);
    return scaled*inversesqrt(max(dot(scaled,scaled),1e-12));
}
vec3 decodeSRGB(vec3 c) { return mix(c/12.92,pow((c+0.055)/1.055,vec3(2.4)),step(vec3(0.04045),c)); }
vec3 encodeSRGB(vec3 c) { c=max(c,vec3(0)); return mix(c*12.92,1.055*pow(c,vec3(1.0/2.4))-0.055,step(vec3(0.0031308),c)); }
vec3 materialNormal(vec3 n) {
    if(drawData.options.w<0.5 || drawData.normalMap.z<=0.0) return n;
    vec2 mappedUV=uv*drawData.normalMap.xy;
    vec2 dx=dFdx(mappedUV),dy=dFdy(mappedUV);
    vec3 px=dFdx(worldPosition),py=dFdy(worldPosition);
    float determinant=dx.x*dy.y-dx.y*dy.x;
    if(abs(determinant)<1e-10) return n;
    vec3 tangent=(px*dy.y-py*dx.y)/determinant;
    vec3 bitangent=(py*dx.x-px*dy.x)/determinant;
    tangent=safeNormal(tangent-n*dot(n,tangent));
    if(dot(tangent,tangent)<0.1) return n;
    float handedness=dot(cross(n,tangent),bitangent)<0.0 ? -1.0 : 1.0;
    bitangent=cross(n,tangent)*handedness;
    vec3 sampleNormal=texture(normalImage,mappedUV).xyz*2.0-1.0;
    sampleNormal.xy*=drawData.normalMap.z;
    sampleNormal.y*=drawData.normalMap.w;
    return safeNormal(tangent*sampleNormal.x+bitangent*sampleNormal.y+n*sampleNormal.z);
}
float visibility(int lightIndex,vec3 geometricNormal) {
    if(drawData.options.z<0.5) return 1.0;
    for(int j=0;j<int(shadows.info.x);j++) {
        Shadow map=shadows.maps[j];
        if(int(map.settings.x)!=lightIndex) continue;
        vec4 projected=map.viewProjection*vec4(worldPosition+geometricNormal*map.settings.w,1);
        if(projected.w<=0.0) return 1.0;
        vec3 p=projected.xyz/projected.w;
        vec2 coord=p.xy*0.5+0.5;
        // GAL's Up raster convention maps positive clip Y to the top of a texture.
        coord.y=1.0-coord.y;
        if(p.z<0.0 || p.z>1.0 || any(lessThan(coord,vec2(0))) || any(greaterThan(coord,vec2(1)))) return 1.0;
        float sum=0.0,weight=0.0;
        int radius=int(map.sampling.y);
        for(int y=-radius;y<=radius;y++) for(int x=-radius;x<=radius;x++) {
            vec2 tap=coord+vec2(x,y)*map.sampling.x;
            if(any(lessThan(tap,vec2(0))) || any(greaterThan(tap,vec2(1)))) sum+=1.0;
            else sum+=texture(shadowImage,vec4(tap,map.settings.y,p.z-map.settings.z));
            weight+=1.0;
        }
        return mix(1.0,sum/weight,map.sampling.z);
    }
    return 1.0;
}
void main() {
    vec4 texel=texture(image,uv);
    if(drawData.options.y>0.5) texel.rgb=decodeSRGB(texel.rgb);
    vec4 base=texel*drawData.tint;
    if(base.a<drawData.emissiveCutoff.a) discard;
    vec3 result=base.rgb;
    if(drawData.options.x>0.5) {
        vec3 geometricNormal=safeNormal(worldNormal);
        vec3 n=materialNormal(geometricNormal), viewDirection=safeNormal(lighting.eye.xyz-worldPosition);
        result=base.rgb*lighting.ambient.rgb;
        for(int i=0;i<int(lighting.info.x);i++) {
            Light light=lighting.lights[i];
            vec3 l=-light.directionRange.xyz;
            float attenuation=1.0;
            if(light.positionKind.w>0.5) {
                vec3 delta=light.positionKind.xyz-worldPosition;
                float distanceSquared=dot(delta,delta), range=light.directionRange.w;
                l=safeNormal(delta);
                float window=max(1.0-distanceSquared/(range*range),0.0);
                attenuation=window*window;
                if(light.coneAttenuation.z>0.5) attenuation/=max(distanceSquared,0.01);
                if(light.positionKind.w>1.5) {
                    float angle=dot(-l,light.directionRange.xyz);
                    attenuation*=smoothstep(light.coneAttenuation.y,light.coneAttenuation.x,angle);
                }
            }
            float diffuse=max(dot(n,l),0.0);
            vec3 response=base.rgb*diffuse;
            if(drawData.options.x>1.5 && diffuse>0.0) {
                vec3 halfway=safeNormal(l+viewDirection);
                response+=drawData.specularShininess.rgb*pow(max(dot(n,halfway),0.0),drawData.specularShininess.a);
            }
            result+=response*light.colorIntensity.rgb*light.colorIntensity.a*attenuation*visibility(i,geometricNormal);
        }
    }
    result+=drawData.emissiveCutoff.rgb;
    if(lighting.info.y>0.5) result=encodeSRGB(result);
    outputColor=vec4(result,base.a);
}
