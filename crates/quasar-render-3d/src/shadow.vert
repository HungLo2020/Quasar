#version 450
struct Vertex {vec4 position;vec4 normal;vec4 uv;};
layout(set=0,binding=1,std430) readonly buffer Geometry {Vertex vertices[];} geometry;
layout(set=0,binding=2,std140) uniform Draw {mat4 viewProjection;mat4 model;vec4 alpha;} drawData;
layout(location=0) out vec2 uv;
void main(){Vertex v=geometry.vertices[gl_VertexIndex];gl_Position=drawData.viewProjection*drawData.model*v.position;uv=v.uv.xy;}
