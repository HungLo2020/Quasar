#version 450
struct Vertex { vec4 position; vec4 normal; vec4 uv; };
layout(set=0,binding=1,std430) readonly buffer Geometry { Vertex vertices[]; } geometry;
layout(set=0,binding=2,std140) uniform Draw { mat4 viewProjection; mat4 model; vec4 tint; vec4 light; } drawData;
layout(location=0) out vec2 uv;
layout(location=1) out vec4 color;
void main() {
    Vertex v=geometry.vertices[gl_VertexIndex];
    gl_Position=drawData.viewProjection*drawData.model*v.position;
    vec3 normal=normalize(transpose(inverse(mat3(drawData.model)))*v.normal.xyz);
    float lighting=0.3+0.7*max(dot(normal,normalize(drawData.light.xyz)),0.0);
    color=vec4(drawData.tint.rgb*lighting,drawData.tint.a);
    uv=v.uv.xy;
}
