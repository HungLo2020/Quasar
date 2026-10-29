#version 450
layout(set=0,binding=0) uniform sampler2D image;
layout(location=0) in vec2 uv;
layout(location=1) in vec4 tint;
layout(location=2) in vec2 logical;
layout(location=3) flat in vec4 clipRect;
layout(location=4) flat in uvec2 indexed;
layout(set=0,binding=3) uniform sampler2D palette;
layout(location=0) out vec4 color;
void main() {
    if (any(lessThan(logical,clipRect.xy)) || any(greaterThanEqual(logical,clipRect.zw))) discard;
    vec4 texel;
    if ((indexed.x & 1u) != 0u) {
        ivec2 size=textureSize(image,0);
        int index=int(round(texelFetch(image,clamp(ivec2(floor(uv*vec2(size))),ivec2(0),size-1),0).r*255.0));
        if (index==int((indexed.x >> 1u) & 511u)-1 || index==int((indexed.x >> 10u) & 511u)-1) discard;
        texel=texelFetch(palette,ivec2(index,0),0);
    } else texel=texture(image,uv);
    color=texel*tint;
    if (color.a*255.0+0.00001 < float(indexed.y)) discard;
}
