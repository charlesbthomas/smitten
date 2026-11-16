// A post-processing effect for a whole scene drawn to a canvas: the
// picture ripples, picks up scanlines and darkens toward the corners.

struct Params {
    time: f32,
    // 0.0 leaves the picture alone, 1.0 is the full effect.
    strength: f32,
    // The size of the picture in pixels.
    size: vec2<f32>,
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    // Shift each row sideways by a wave that travels down the picture.
    let wave = sin(uv.y * 24.0 + params.time * 3.0) * 0.008 * params.strength;
    var texel = textureSample(image, image_sampler, uv + vec2<f32>(wave, 0.0));

    // Darken every other pair of rows.
    let row = floor(uv.y * params.size.y / 2.0);
    let scanline = 1.0 - 0.25 * params.strength * (row % 2.0);

    // Darken toward the corners.
    let from_center = distance(uv, vec2<f32>(0.5, 0.5));
    let vignette = 1.0 - params.strength * smoothstep(0.35, 0.8, from_center);

    return vec4<f32>(texel.rgb * scanline * vignette, texel.a) * color;
}
