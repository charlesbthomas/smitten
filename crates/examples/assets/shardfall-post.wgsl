// The full-screen pass for the Shardfall example. The whole arena is drawn
// to a canvas first, and the canvas is drawn to the screen through this.
//
// - A shockwave ring pushes the picture outward from the last big
//   explosion.
// - The red and blue channels split apart toward the edges, more the
//   harder the screen is shaking.
// - Bright pixels bleed light into their neighbors, a cheap bloom.
// - The corners darken.

struct Params {
    // 0.0 leaves the picture alone, 1.0 is the full effect.
    strength: f32,
    // How far, in pixels, the color channels split at the edges.
    aberration: f32,
    // How far, in pixels, the shockwave ring has traveled.
    shock_radius: f32,
    // How hard the ring pushes, from 0.0 to 1.0.
    shock_power: f32,
    // Where the shockwave started, in pixels.
    shock_center: vec2<f32>,
    // The size of the picture in pixels.
    size: vec2<f32>,
}

fn bright(uv: vec2<f32>) -> vec3<f32> {
    let texel = textureSample(image, image_sampler, uv).rgb;
    return max(texel - vec3<f32>(0.45), vec3<f32>(0.0));
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let px = uv * params.size;

    // The shockwave: a band about 30 pixels wide around the ring.
    let away = px - params.shock_center;
    let distance_out = length(away);
    let band = 1.0 - smoothstep(0.0, 30.0, abs(distance_out - params.shock_radius));
    let outward = away / max(distance_out, 0.001);
    let push = outward * band * params.shock_power * 16.0 * params.strength;
    let base = (px - push) / params.size;

    // Split the channels along the line from the center.
    let split = (uv - vec2<f32>(0.5)) * (1.0 + params.aberration) * 2.0 * params.strength
        / params.size;
    let r = textureSample(image, image_sampler, base + split).r;
    let g = textureSample(image, image_sampler, base).g;
    let b = textureSample(image, image_sampler, base - split).b;
    var rgb = vec3<f32>(r, g, b);

    // Bloom from eight taps on two rings around the pixel.
    let texel = 1.0 / params.size;
    var glow = vec3<f32>(0.0);
    for (var i = 0; i < 8; i = i + 1) {
        let angle = f32(i) * 0.785398;
        let dir = vec2<f32>(cos(angle), sin(angle));
        glow = glow + bright(base + dir * texel * 3.0) * 0.6;
        glow = glow + bright(base + dir * texel * 8.0) * 0.4;
    }
    rgb = rgb + glow * 0.22 * params.strength;

    // A faint bright edge on the shockwave itself.
    rgb = rgb + vec3<f32>(0.5, 0.7, 1.0) * band * params.shock_power * 0.25 * params.strength;

    let from_center = distance(uv, vec2<f32>(0.5));
    let vignette = 1.0 - 0.6 * params.strength * smoothstep(0.4, 0.85, from_center);

    return vec4<f32>(rgb * vignette, 1.0) * color;
}
