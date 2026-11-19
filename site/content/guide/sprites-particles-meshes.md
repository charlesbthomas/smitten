+++
title = "Sprite batches, particles and meshes"
description = "Drawing many sprites at once, particle effects and custom geometry."
weight = 5
[extra]
nav = "Batches, particles, meshes"
+++

## Batching is automatic

Smitten groups a frame's drawing into as few GPU draw calls as the draw
order allows, so drawing thousands of sprites one `graphics::draw` at a
time is already fast. The `benchmark` example moves, turns and tints
10,000 sprites in under a millisecond of Smitten's own time a frame:

```sh
cargo run --release --example benchmark
```

## Sprite batches

A {{ api(path="graphics/struct.SpriteBatch.html", text="SpriteBatch") }}
holds many copies of one image, or of parts of it, and draws them together.
It pays off for things that are built once and drawn often, such as a tile
map:

```rust
use smitten::graphics::{self, Quad, SpriteBatch};

let mut tiles = SpriteBatch::new(&sheet, 1000);
let grass = Quad::new((0.0, 0.0), (16.0, 16.0), sheet.size());
for row in 0..20 {
    for column in 0..30 {
        let at = (column as f32 * 16.0, row as f32 * 16.0);
        tiles.add_quad(&grass, at);
    }
}

// Every frame:
graphics::draw(&tiles, (0.0, 0.0));
```

`add` places the whole image and `add_quad` part of it, with a position or
`DrawParams`. Each returns an
index, which `set` uses to move that sprite later. `set_color` tints the
sprites added after it, and `clear` empties the batch.

## Particle systems

A {{ api(path="graphics/struct.ParticleSystem.html", text="ParticleSystem") }}
emits images that move, spin, grow and change color over their lives, for
fire, smoke, sparks and the like:

```rust
use std::f32::consts::FRAC_PI_2;
use smitten::graphics::{self, EmissionArea, ParticleSystem};
use smitten::{BlendMode, Color};

let mut fire = ParticleSystem::new(&dot, 600);
fire.set_emission_rate(220.0);
fire.set_particle_lifetime(0.4, 0.9);
fire.set_emission_area(EmissionArea::uniform((10.0, 4.0)));
fire.set_direction(-FRAC_PI_2);
fire.set_spread(0.6);
fire.set_speed(60.0, 160.0);
fire.set_linear_acceleration((0.0, -120.0), (0.0, -60.0));
fire.set_sizes(&[1.2, 0.9, 0.2]);
fire.set_colors(&[
    Color::rgba(1.0, 0.9, 0.4, 0.9),
    Color::rgba(1.0, 0.45, 0.1, 0.7),
    Color::rgba(0.6, 0.1, 0.05, 0.0),
]);

// In update:
fire.update(dt);

// In draw:
graphics::set_blend_mode(BlendMode::Add);
graphics::draw(&fire, (x, y));
graphics::set_blend_mode(BlendMode::Alpha);
```

A system with no emission rate emits only when asked, which suits bursts:
`sparks.set_position((x, y))` then `sparks.emit(150)`.

{{ figure(src="particles-example.png", alt="A fire with rising smoke, a burst of falling sparks, and a swirling ring of blue embers.", caption="The particles example.") }}

## Meshes

A {{ api(path="graphics/struct.Mesh.html", text="Mesh") }} is a shape made
of your own vertices, each with a position, a place in a texture and a
color. Use one for gradients, deformed images and anything shapes can't
draw:

```rust
use smitten::graphics::{self, Mesh, MeshDrawMode, MeshVertex};
use smitten::Color;

let (top, bottom) = (Color::rgb(0.15, 0.1, 0.4), Color::rgb(0.95, 0.45, 0.35));
let sky = Mesh::new(
    &[
        MeshVertex::colored((0.0, 0.0), top),
        MeshVertex::colored((800.0, 0.0), top),
        MeshVertex::colored((800.0, 600.0), bottom),
        MeshVertex::colored((0.0, 600.0), bottom),
    ],
    MeshDrawMode::Fan,
);
graphics::draw(&sky, (0.0, 0.0));
```

`MeshDrawMode` is `Fan`, `Strip`, `Triangles` or `Points`.
`MeshVertex::new(position, u, v)` gives a vertex texture coordinates,
`mesh.set_texture(Some(&image))` stretches an image over the mesh, and
`set_vertices` changes the vertices every frame for animation. Meshes can
also have index lists, custom vertex attributes for shaders, and be drawn
many times at once with `graphics::draw_instanced`.

{{ figure(src="mesh-example.png", alt="A gradient rectangle, a color wheel made as a fan, and a heart image waving like a flag on a strip.", caption="The mesh example.") }}
