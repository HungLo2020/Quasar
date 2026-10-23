#version 450
layout(set=0,binding=0) uniform sampler2D image;
layout(location=0) in vec2 uv;
layout(location=1) in vec4 tint;
layout(location=2) in vec2 logical;
layout(location=3) flat in vec4 clipRect;
layout(location=0) out vec4 color;
void main() {
    if (any(lessThan(logical,clipRect.xy)) || any(greaterThanEqual(logical,clipRect.zw))) discard;
    color=texture(image,uv)*tint;
}
