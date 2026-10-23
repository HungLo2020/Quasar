#version 450
struct Quad { vec4 originX; vec4 axisY; vec4 uv; vec4 color; vec4 clip; };
layout(set=0,binding=1,std430) readonly buffer Geometry { Quad quads[]; } geometry;
layout(set=0,binding=2,std140) uniform Frame { vec4 sizeBase; vec4 viewport; } frame;
layout(location=0) out vec2 uv;
layout(location=1) out vec4 tint;
layout(location=2) out vec2 logical;
layout(location=3) flat out vec4 clipRect;
void main() {
    const vec2 corners[6]=vec2[6](vec2(0,0),vec2(1,0),vec2(0,1),vec2(0,1),vec2(1,0),vec2(1,1));
    Quad q=geometry.quads[int(frame.sizeBase.z)+gl_InstanceIndex];
    vec2 p=corners[gl_VertexIndex];
    logical=q.originX.xy+p.x*q.originX.zw+p.y*q.axisY.xy;
    vec2 screen=frame.viewport.xy+(logical/frame.sizeBase.xy)*frame.viewport.zw;
    gl_Position=vec4(screen*2.0-1.0,0.0,1.0);
    uv=mix(q.uv.xy,q.uv.zw,p);tint=q.color;clipRect=q.clip;
}
