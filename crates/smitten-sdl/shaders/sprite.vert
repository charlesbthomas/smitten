#version 450

// SDL's GPU API expects vertex shader uniforms in set 1.
layout(set = 1, binding = 0) uniform Screen {
    // Two divided by the drawable size in pixels.
    vec2 pixel_to_clip;
};

layout(location = 0) in vec2 in_position;
layout(location = 1) in vec2 in_uv;
layout(location = 2) in vec4 in_color;

layout(location = 0) out vec2 out_uv;
layout(location = 1) out vec4 out_color;

void main() {
    // Pixels run from the top-left with y down; clip space has y up.
    // Depth 0.5 is where z 0 lands in LÖVE's projection, as for meshes.
    vec2 clip = in_position * pixel_to_clip - 1.0;
    gl_Position = vec4(clip.x, -clip.y, 0.5, 1.0);
    out_uv = in_uv;
    out_color = in_color;
}
