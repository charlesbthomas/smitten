+++
title = "Canvases, shaders and the stencil"
description = "Drawing off screen, WGSL shaders, the scissor, the stencil and 2D lighting."
weight = 4
[extra]
nav = "Canvases and shaders"
+++

## Canvases

A {{ api(path="graphics/struct.Canvas.html", text="Canvas") }} is an image you
can draw into. Use one to render a low-resolution game and scale it up, to
build up a light map, or to keep a picture that would be slow to redraw
every frame.

`canvas.render_to` sends drawing to the canvas for the length of a closure,
then puts things back the way they were:

```rust
use smitten::graphics::{self, Canvas, DrawMode, DrawParams};
use smitten::{BlendMode, Color};

let scene = Canvas::new(320, 180)?;

scene.render_to(|| {
    graphics::clear(Color::rgb(0.1, 0.1, 0.2));
    graphics::circle(DrawMode::Fill, (160.0, 90.0), 30.0);
});

// A canvas holds colors already multiplied by their alpha.
graphics::set_blend_mode(BlendMode::Premultiplied);
graphics::draw(&scene, DrawParams::at((0.0, 0.0)).scale(4.0));
graphics::set_blend_mode(BlendMode::Alpha);
```

`graphics::set_canvas(Some(&canvas))` and `set_canvas(None)` work too. A
canvas keeps its contents until you clear it, so a canvas you never clear collects everything drawn to it,
like paint.

A canvas is as sharp as the screen by default. For pixel art, give it
exactly one pixel per unit and draw it with hard edges:

```rust
use smitten::graphics::{Canvas, CanvasSettings};
use smitten::Filter;

let pixels = Canvas::with_settings(
    96,
    96,
    CanvasSettings {
        dpi_scale: Some(1.0),
        ..CanvasSettings::default()
    },
)?;
pixels.set_filter(Filter::Nearest);
```

{{ figure(src="canvas-example.png", alt="A low-resolution scene scaled up with hard pixel edges, beside a paint panel with striped background.", caption="The canvas example: a pixel art canvas, additive glows, a paint canvas that is never cleared, and the scissor.") }}

[`Config::design_size`](@/guide/window-and-screens.md#a-fixed-design-size)
does the scale-up for a whole game without a canvas of your own.

## Shaders

A {{ api(path="graphics/struct.Shader.html", text="Shader") }} decides the
color of every pixel drawn while it is active. Smitten shaders are written
in [WGSL](https://www.w3.org/TR/WGSL/). A shader
defines one function, `effect`, which gets the drawing color, the texture
coordinate and the pixel's position, and returns the pixel's color:

```
fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let texel = textureSample(image, image_sampler, uv);
    let gray = dot(texel.rgb, vec3<f32>(0.299, 0.587, 0.114));
    return vec4<f32>(gray, gray, gray, texel.a) * color;
}
```

`image` and `image_sampler` are the picture being drawn. Shapes have a
plain white picture, so the same shader works for them.

To pass values in from the game, declare a struct named `Params`, read it
through `params`, and set its members with `shader.send`:

```rust
use smitten::graphics::{self, Shader};

const GRAY: &str = "
struct Params {
    amount: f32,
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let texel = textureSample(image, image_sampler, uv) * color;
    let gray = dot(texel.rgb, vec3<f32>(0.299, 0.587, 0.114));
    return vec4<f32>(mix(texel.rgb, vec3<f32>(gray), params.amount), texel.a);
}
";

let gray = Shader::new(GRAY)?;
gray.send("amount", 0.75f32);

graphics::set_shader(Some(&gray));
graphics::draw(&heart, (100.0, 100.0));
graphics::set_shader(None);
```

`Params` members may be `f32`, `i32` or `u32`, vectors and square matrices
of them, or fixed-size arrays sent with `send_array`. `send_color` sends a
`Color`, and `send_texture` sends another image or canvas for the shader to
read. `Shader::from_file("effect.wgsl")` loads a shader from the assets
folder, and it reloads when you save the file.

For an effect over the whole screen, draw the scene to a canvas, then draw
the canvas through the shader.

{{ figure(src="shader-example.png", alt="A scene of bars and a sun with a rippling scanline effect, and a row of hearts that grow grayer from left to right.", caption="The shader example: a full-screen effect from a file, and a small inline shader.") }}

## The scissor

`set_scissor(x, y, width, height)` limits drawing to a rectangle on the
screen, and `clear_scissor` lifts the limit. `intersect_scissor` narrows the
current one.

## The stencil

The stencil marks pixels so that later drawing can be kept to them or kept
out of them. Draw the mask inside `stencil`, then set a test:

```rust
use smitten::graphics::{self, DrawMode};
use smitten::CompareMode;

graphics::stencil(|| {
    graphics::circle(DrawMode::Fill, (mouse_x, mouse_y), 90.0);
});

// Only inside the circle.
graphics::set_stencil_test(CompareMode::Greater, 0);
draw_pattern();

// Everywhere except inside it.
graphics::set_stencil_test(CompareMode::Equal, 0);
draw_grid();

graphics::clear_stencil_test();
```

`stencil_with(StencilWrite { .. }, draw)` chooses the action and value
written to the stencil.

{{ figure(src="stencil-example.png", alt="A colorful striped pattern showing through a circle and a star, with a dark grid everywhere else.", caption="The stencil example.") }}

## 2D lights and shadows

Canvases, a shader and the stencil together make 2D lighting. The
`lighting` example draws the room fully lit, then builds a light map on a
canvas: it clears it to a dim ambient color, and for each light marks the
shadows it casts in the stencil with
{{ api(path="graphics/fn.shadow.html", text="graphics::shadow") }}, then
adds the light's glow outside them. Finally it multiplies the light map
over the room:

```rust
self.light_map.render_to(|| {
    graphics::clear(Color::rgb(0.12, 0.12, 0.16));
    graphics::set_shader(Some(&self.glow));
    for light in &lights {
        graphics::stencil(|| {
            for block in &blocks {
                graphics::shadow((light.x, light.y), &block.corners, 2000.0);
            }
        });
        graphics::set_stencil_test(CompareMode::Equal, 0);
        graphics::set_blend_mode(BlendMode::Add);
        graphics::set_color(light.color);
        graphics::rectangle(
            DrawMode::Fill,
            (light.x - light.radius, light.y - light.radius),
            (light.radius * 2.0, light.radius * 2.0),
        );
        graphics::set_blend_mode(BlendMode::Alpha);
        graphics::clear_stencil_test();
    }
    graphics::set_shader(None);
});

graphics::set_color(Color::WHITE);
graphics::set_blend_mode(BlendMode::Multiply);
graphics::draw(&self.light_map, (0.0, 0.0));
graphics::set_blend_mode(BlendMode::Alpha);
```

<div class="gallery">
{{ figure(src="lighting-example.png", alt="A room lit by colored lights, with hard shadows cast behind blocks.", caption="The lit room.") }}
{{ figure(src="lighting-light-map.png", alt="The light map on its own: pools of colored light with shadow wedges cut out.", caption="The light map by itself.") }}
</div>

Run it with `cargo run --example lighting`. The
[Lantern](@/examples/_index.md#lantern) game uses the same technique for its
lantern beam.
