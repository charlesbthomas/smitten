+++
title = "Examples"
description = "Example games and demos that come with Smitten, with screenshots and how to run them."
template = "section.html"
+++

Smitten's repository comes with six small games and a demo of each part
of the framework. They live in
[`crates/examples/examples`](https://github.com/charlesbthomas/smitten/tree/main/crates/examples/examples),
and their images, sounds and shaders in `crates/examples/assets`. Run any
of them from the workspace root with `cargo run --example <name>`, or
`just run <name>`.

## Games

### Lantern

{{ figure(src="lantern.png", alt="A lantern beam lights a dark room of tiles, with glowing embers and a ghostly shade in the dark.") }}

Carry a light through dark rooms. Shades creep toward you whenever your
lantern isn't on them, and freeze while it is. Gather every ember in a room
to open its door, and get through all three rooms. You hear a shade before
you see it: each one whispers from the side it is on, muffled when a wall
is between you.

Besides being a game, Lantern shows how the pieces fit together: a light map
on a canvas with shadows cut out by the stencil, a shader for the beam,
positional and filtered sound, sounds made in code, gamepad rumble and a
best time kept in the save directory.

W, A, S, D or the arrow keys walk, and the lantern points at the mouse. On a
gamepad, the left stick walks and the right stick aims.

`cargo run --example lantern` · [Source](https://github.com/charlesbthomas/smitten/blob/main/crates/examples/examples/lantern.rs)

### Pong

<div class="pong">
<video src="../images/pong-gameplay.mp4" poster="../images/pong-rally.png" controls muted loop playsinline width="800" height="600"></video>
</div>

Pong for one or two players. Press 1 on the title screen to play the
computer, or 2 for two players. The left paddle moves with W and S and the
right with the arrow keys, and gamepads work too. Space serves, and first to
11 wins. It uses a [design size](@/guide/window-and-screens.md#a-fixed-design-size)
of 800 by 600, so the court fills any window.

`cargo run --example pong` · [Source](https://github.com/charlesbthomas/smitten/blob/main/crates/examples/examples/pong.rs)

### Stack and Topple

{{ figure(src="topple.png", alt="A wobbly tower of colored blocks on a tipping platform, with a height scale and a best-height line.") }}

Build the tallest tower you can on a platform that tips under its load.
Left and Right move the next block, Up turns it, and Space drops it. Every
block that falls off costs a heart, and the best height is kept between
runs. Built on [physics](@/guide/physics.md). P saves a screenshot.

`cargo run --example topple` · [Source](https://github.com/charlesbthomas/smitten/blob/main/crates/examples/examples/topple.rs)

### Undercroft

{{ figure(src="undercroft.png", alt="A hooded hero in a torchlit dungeon corridor, with a rat and a pale husk nearby, red dots marking the paths they plan to take, and remembered rooms drawn dim blue.") }}

A turn-based dungeon crawl. Every floor is built from a seed: rooms joined
by corridors, stocked with gold, tonics and monsters. Find the stairs and go
deeper. Walk into a monster to fight it. Monsters that see you follow you
around corners, and press P to see the paths they plan to take.

Besides being a game, Undercroft shows tile art painted in code into an
`ImageData`, each floor drawn as one sprite batch, fog of war made from a
light map with wall shadows cut out by the stencil, A* pathfinding for the
monsters and for click-to-walk, and a run saved as a text file in the save
directory.

W, A, S, D or the arrow keys walk, and clicking a tile you have seen walks
there. Space waits, Q drinks a tonic and Return takes the stairs. F5 saves
and F9 loads, and the game saves on every new floor and when you quit.
`--seed 1234` starts a run on the floors that seed builds.

`cargo run --example undercroft` · [Source](https://github.com/charlesbthomas/smitten/blob/main/crates/examples/examples/undercroft.rs)

### Shardfall

{{ figure(src="shardfall.png", alt="A glowing arrowhead ship fires a stream of bullets at neon shapes in a dark grid arena, while an explosion of amber sparks sends a shockwave across the screen.") }}

A twin-stick arena shooter. Waves of glowing shapes warp into an arena
larger than the screen: seekers that chase you, splitters that burst into
shards, and lancers that circle and fire volleys of orbs. Every kill raises
a score multiplier, and taking a hit resets it.

It shows gamepad sticks, triggers and rumble with a keyboard and mouse
fallback, a fixed-size bullet pool, thousands of particles, screen shake
driven by smooth noise, a full-screen shader with shockwaves, color split
and bloom, and sounds made in code that play where they happen in the arena.

On a gamepad, the left stick moves, the right stick aims and fires, and A or
a shoulder button dashes. With a keyboard, W, A, S and D move, the mouse
aims, the left button fires, and Space or the right button dashes. Tab, or
Back on a gamepad, switches the screen effects off to compare.

`cargo run --example shardfall` · [Source](https://github.com/charlesbthomas/smitten/blob/main/crates/examples/examples/shardfall.rs)

### Frostmarch

{{ figure(src="frostmarch.png", alt="A hooded fighter in a pink cape swings a greatsword at horned wights on a snowfield, while a brute's orange circle on the ground warns where its slam will land.") }}

A proof of concept for fast melee combat. One fighter holds a snowfield
against waves of wights, brutes and casters. There are three attacks,
light, medium and heavy, and four weapons that each turn them into
different moves. Up to four attacks chain into a string, and each weapon
has four named strings that finish harder. Enemies paint where they are
about to hit on the ground, a dash passes through anything, and a swing
that meets a caster's bolt sends it back.

The fighters are not sprites. Each is a small skeleton solved every frame,
with legs and arms bent by two-bone inverse kinematics and a cape that
trails as a chain, and B shows the bones. It also shows hit-stop, screen
shake, buffered input, a bitmap font built in code, and sounds made in
code.

W, A, S and D move and the mouse aims. The left button is the light attack,
the right button the medium, and F the heavy. Space dashes, and Q and E
change weapon. On a gamepad, X, Y and B attack, A dashes, and the shoulder
buttons change weapon. Tab lets the game play itself.

`cargo run --example frostmarch` · [Source](https://github.com/charlesbthomas/smitten/blob/main/crates/examples/examples/frostmarch.rs)

## Demos

<div class="gallery">

{% example(name="lighting", title="Lighting", src="lighting-example.png", alt="A room lit by colored lights with shadows behind blocks.") %}
2D lights and shadows from a canvas, a shader and the stencil. Click to
leave lights behind. See [2D lights and shadows](@/guide/canvases-and-shaders.md#2d-lights-and-shadows).
{% end %}

{% example(name="sprites", title="Sprites", src="sprites-example.png", alt="Tinted hearts bouncing over a floor and a sun.") %}
Loading, tinting and spinning images, shapes and the transform stack.
{% end %}

{% example(name="shapes", title="Shapes", src="shapes-example.png", alt="Every shape, filled and outlined.") %}
Every shape Smitten draws, filled and outlined.
{% end %}

{% example(name="text", title="Text", src="text-example.png", alt="Text at several sizes and alignments, and a bitmap font.") %}
Fonts, wrapping and alignment, rotated text, a bitmap font, and a counter
kept in the save directory.
{% end %}

{% example(name="canvas", title="Canvases", src="canvas-example.png", alt="A pixel art canvas scaled up beside a paint panel.") %}
A pixel art canvas, additive blending, a paint canvas and the scissor.
{% end %}

{% example(name="shader", title="Shaders", src="shader-example.png", alt="A rippling full-screen effect and hearts turning gray.") %}
A full-screen effect loaded from a WGSL file, and an inline shader.
{% end %}

{% example(name="stencil", title="Stencil", src="stencil-example.png", alt="A pattern showing through a circle and a star.") %}
Drawing kept inside, or out of, shapes marked in the stencil.
{% end %}

{% example(name="particles", title="Particles", src="particles-example.png", alt="Fire, smoke, sparks and a swirl of embers.") %}
Fire that follows the mouse, smoke, sparks on click, and a swirl.
{% end %}

{% example(name="mesh", title="Meshes", src="mesh-example.png", alt="A gradient, a color wheel and a waving heart flag.") %}
A gradient, a color wheel built as a fan, and an image waving on a strip.
{% end %}

{% example(name="physics", title="Physics", src="physics-example.png", alt="Crates and balls in a box with a hanging chain of planks.") %}
Drop crates and balls, drag them around, and watch a chain and a moving
platform.
{% end %}

{% example(name="spatial_audio", title="Spatial audio", src="spatial-audio-example.png", alt="A listener ring and a sound that follows the mouse.") %}
A hum made in code follows the mouse, panning and fading, with reverb and
echo to switch on.
{% end %}

{% example(name="design", title="Design size", src="design-pixel-perfect.png", alt="A 320 by 180 scene enlarged three times with black bars.") %}
A game drawn at 320 by 180 and scaled to the window in whole steps. Run it
with `-- smooth` to scale smoothly instead.
{% end %}

{% example(name="graphics_extras", title="Graphics extras", src="graphics-extras-example.png", alt="A transformed square, mipmaps, array and cube images and the color mask.") %}
Text objects, transforms, mipmaps, array and cube images, the color mask,
wireframe and the graphics statistics.
{% end %}

</div>

### More demos

| Example | What it shows |
|---|---|
| `hello` | A window whose color drifts over time, with fullscreen on F11. |
| `input` | Every kind of input at once: keys, mouse, wheel, gamepad, touch, typed text and a custom cursor. |
| `audio` | Sound effects that overlap at random pitches, and streamed music. |
| `video` | A video with its sound track, and a tone made a buffer at a time with a queueable source. |
| `rumble` | Gamepad rumble, to try with a controller in your hands. |
| `benchmark` | How many sprites fit in a frame. Run it with `--release`. |
