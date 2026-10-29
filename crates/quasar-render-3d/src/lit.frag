#version 450
layout(set=0,binding=0) uniform sampler2D image;
layout(set=0,binding=2,std140) uniform Draw {
    mat4 viewProjection; mat4 model; vec4 tint; vec4 specularShininess; vec4 emissiveCutoff; vec4 options;
} drawData;
struct Light { vec4 positionKind; vec4 directionRange; vec4 colorIntensity; vec4 coneAttenuation; };
layout(set=0,binding=3,std430) readonly buffer Lights { vec4 ambient; vec4 eye; vec4 info; Light lights[]; } lighting;
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
void main() {
    vec4 texel=texture(image,uv);
    if(drawData.options.y>0.5) texel.rgb=decodeSRGB(texel.rgb);
    vec4 base=texel*drawData.tint;
    if(drawData.options.z>=4.0) base.a=1.0;
    if(base.a<drawData.emissiveCutoff.a) discard;
    vec3 result=base.rgb;
    if(drawData.options.x>0.5) {
        vec3 n=safeNormal(worldNormal);
        if(mod(floor(drawData.options.z/2.0),2.0)>0.5 && !gl_FrontFacing) n=-n;
        vec3 viewDirection=safeNormal(lighting.eye.xyz-worldPosition);
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
            result+=response*light.colorIntensity.rgb*light.colorIntensity.a*attenuation;
        }
    }
    result+=drawData.emissiveCutoff.rgb;
    if(lighting.info.y>0.5) result=encodeSRGB(result);
    outputColor=vec4(result,base.a);
}
