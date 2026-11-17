#version 450

// Draws one layer of an array texture. SDL's GPU API expects fragment
// shader samplers in set 2 and uniforms in set 3.
layout(set = 2, binding = 0) uniform sampler2DArray sprite_texture;

layout(set = 3, binding = 0) uniform Layer {
    // Which picture to draw, as a float since that is what texture() takes.
    float layer;
};

layout(location = 0) in vec2 in_uv;
layout(location = 1) in vec4 in_color;

layout(location = 0) out vec4 out_color;

void main() {
    out_color = texture(sprite_texture, vec3(in_uv, layer)) * in_color;
}
