#version 450
layout(set=0,binding=0) uniform sampler2D image;
layout(set=0,binding=2,std140) uniform Draw {mat4 viewProjection;mat4 model;vec4 alpha;} drawData;
layout(location=0) in vec2 uv;
void main(){if(texture(image,uv).a*drawData.alpha.y<drawData.alpha.x)discard;}
