#version 450
struct Vertex { vec4 position; vec4 normal; vec4 uv; };
layout(set=0,binding=1,std430) readonly buffer Geometry { Vertex vertices[]; } geometry;
layout(set=0,binding=2,std140) uniform Draw {
    mat4 viewProjection; mat4 model; vec4 tint; vec4 specularShininess; vec4 emissiveCutoff; vec4 options;
} drawData;
layout(location=0) out vec2 uv;
layout(location=1) out vec3 worldPosition;
layout(location=2) out vec3 worldNormal;
void main() {
    Vertex v=geometry.vertices[gl_VertexIndex];
    vec4 world=drawData.model*v.position;
    gl_Position=drawData.viewProjection*world;
    worldPosition=world.xyz;
    worldNormal=transpose(inverse(mat3(drawData.model)))*v.normal.xyz;
    uv=v.uv.xy;
}
