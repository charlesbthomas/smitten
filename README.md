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

Early. A window, the game loop, keyboard, mouse and gamepad input, timing,
shapes, images, text, the transform stack, canvases, blend modes, the
scissor, save files and audio (WAV, Ogg Vorbis, MP3 and FLAC, static or
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
cargo run --example pong
```

The examples live in `crates/examples/examples`. Run these commands from
the workspace root.

SDL3 is built from source on the first build, which needs CMake and a C
compiler.

- **Windows:** Visual Studio Build Tools with the C++ workload, plus CMake.
- **macOS:** Xcode command line tools, plus CMake.
- **Linux:** CMake, a C compiler, and SDL's
  [build dependencies](https://wiki.libsdl.org/SDL3/README-linux#build-dependencies).

To link against an SDL3 that is already installed, build with
`--no-default-features`.

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
