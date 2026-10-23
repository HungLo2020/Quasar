#version 450
layout(set = 0, binding = 0) uniform sampler2D framebuffer;
layout(set = 0, binding = 1) uniform Viewport { vec4 rect; } viewport;
layout(location = 0) in vec2 screenUV;
layout(location = 0) out vec4 color;
void main() {
    vec2 uv = (screenUV - viewport.rect.xy) / viewport.rect.zw;
    if (any(lessThan(uv, vec2(0.0))) || any(greaterThanEqual(uv, vec2(1.0)))) {
        color = vec4(0.0, 0.0, 0.0, 1.0);
    } else {
        color = vec4(texture(framebuffer, uv).rgb, 1.0);
    }
}
