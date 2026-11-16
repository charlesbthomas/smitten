# Smitten

A LÖVE-style 2D game framework for Rust.

Smitten aims to feel like [LÖVE](https://love2d.org): you write `load`,
`update` and `draw`, and call modules such as `graphics` and `keyboard` from
anywhere. It is built on SDL3 and its GPU API.

```rust
use smitten::{graphics, Color, Config, Game};

struct Hello;

impl Game for Hello {
    fn load() -> Self {
        Hello
    }

    fn draw(&mut self) {
        graphics::clear(Color::rgb(0.1, 0.1, 0.2));
    }
}

fn main() {
    smitten::run::<Hello>(Config::default());
}
```

## Status

Early. A window (with fullscreen and an icon), the game loop, keyboard, mouse and gamepad input, timing,
shapes, images, text, the transform stack, canvases, blend modes, the
scissor, the stencil, particle systems, sprite batches, save files, random numbers and noise, physics, and audio (WAV, Ogg Vorbis, MP3 and FLAC, static or
streamed) work, as do custom shaders written in WGSL. Rendering currently
uses Vulkan only, so macOS is not supported yet. See
[docs/api-sketch.md](docs/api-sketch.md) for where the API is headed.

## Layout

Smitten is a Cargo workspace. Games depend only on `smitten`.

| Crate | Purpose |
| --- | --- |
| `crates/smitten` | The public API: the `Game` trait, `run`, and the LÖVE-style modules. |
| `crates/smitten-core` | Shared types (`Color`, `Key`, `Config`) and the `Backend` trait that platforms implement. No dependencies. |
| `crates/smitten-sdl` | The SDL3 backend. The only crate with `unsafe` code or a native library. |
| `crates/examples` | Example games, including Pong, and the assets they load. |
| `crates/cargo-smitten` | The `cargo smitten package` command, which builds a game for release. |

## Running the examples

```
cargo run --example hello
cargo run --example sprites
cargo run --example input
cargo run --example text
cargo run --example audio
cargo run --example canvas
cargo run --example shader
cargo run --example stencil
cargo run --example particles
cargo run --example mesh
cargo run --example physics
cargo run --example lighting
cargo run --example pong
cargo run --example design
cargo run --release --example benchmark
```

The examples live in `crates/examples/examples`. Run these commands from
the workspace root.

The `benchmark` example measures how many sprites fit in a frame. Smitten's
own work for 10,000 moving, turning, tinted sprites is under a millisecond
a frame, so the graphics card sets the limit.

SDL3 is built from source on the first build, which needs CMake and a C
compiler.

- **Windows:** Visual Studio Build Tools with the C++ workload, plus CMake.
  The Build Tools include a copy of CMake that is only on the path inside
  "Developer PowerShell for VS 2022", so run cargo from there or install
  CMake separately.
- **macOS:** Xcode command line tools, plus CMake.
- **Linux:** CMake, a C compiler, and SDL's
  [build dependencies](https://wiki.libsdl.org/SDL3/README-linux#build-dependencies).

To link against an SDL3 that is already installed, build with
`--no-default-features`.

## Screen sizes and high-DPI displays

Sizes and positions are measured in units, as in LÖVE. On an ordinary
display a unit is one pixel. On a high-DPI display (a Retina screen, or
Windows set to 150%) a unit covers more pixels, so an 800 by 600 game looks
the same size everywhere and is simply drawn sharper. Text and canvases are
made as sharp as the screen automatically. `graphics::pixel_dimensions()`
and `window::dpi_scale()` give the real numbers when you need them, and
`Config { dpi_scaling: false, .. }` switches to raw pixels.

A game laid out for one fixed size can set a design size instead:

```rust
smitten::run::<MyGame>(Config {
    resizable: true,
    design_size: Some((320, 180)),
    design_scaling: Scaling::PixelPerfect,
    ..Config::default()
});
```

The game then always draws in a 320 by 180 space and Smitten scales the
picture to the window, with black bars where the shapes differ. The mouse
position arrives in the same space. `Scaling::PixelPerfect` draws at
exactly that many pixels and enlarges by whole numbers, for pixel art.
`Scaling::Fit`, the default, scales smoothly and draws at the window's full
resolution, for high-resolution art.

## Physics

`smitten::physics` follows `love.physics`: a `World` holds bodies, bodies
get fixtures (circles, rectangles, polygons, edges, chains), and joints tie
bodies together. `world.contacts()` reports what started or stopped
touching, in place of LÖVE's contact callbacks. The simulation is done by
[Rapier](https://rapier.rs) where LÖVE uses Box2D, so the API carries over
but the exact motion does not.

A game that does not use physics can leave the engine out of its build:

```toml
smitten = { version = "...", default-features = false, features = ["sdl-from-source"] }
```

## The error screen

If a game panics, the window does not vanish. It turns blue and shows the
message and the file and line it came from, as LÖVE's error screen does,
until Escape is pressed or the window is closed. The message is still
printed to the terminal, and the program still ends with the panic.

## Hot reload

Under `cargo run`, an image, shader, font or sound loaded from a file is
loaded again when that file is saved, so edits show up in the running
game. A file with a mistake in it prints the error and the game keeps
using the last version that worked. A sound that is playing starts over
with the new version.

This is on in debug builds and off in release builds. Set
`Config::hot_reload` to choose.

## Releasing a game

`cargo smitten package` builds a game in release mode and gathers what a
player needs: the executable, with SDL linked into it, and the game's
`assets` folder. It writes a folder and a zip of that folder to
`target/package`.

```
cargo install --path crates/cargo-smitten
cd my-game
cargo smitten package
```

To try it on an example from this workspace without installing:

```
cargo run -p cargo-smitten -- package -p smitten-examples --example pong
```

The package is for the platform it was built on, so build on Windows for
Windows players and on Linux for Linux players. On Windows, add
`#![windows_subsystem = "windows"]` to the top of the game's `main.rs` so
it does not open a console window alongside the game.
