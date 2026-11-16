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
shapes, images and the transform stack work. Text and audio come next. Rendering currently
uses Vulkan only, so macOS is not supported yet. See
[docs/api-sketch.md](docs/api-sketch.md) for where the API is headed.

## Layout

Smitten is a Cargo workspace. Games depend only on `smitten`.

| Crate | Purpose |
| --- | --- |
| `crates/smitten` | The public API: the `Game` trait, `run`, and the LÖVE-style modules. |
| `crates/smitten-core` | Shared types (`Color`, `Key`, `Config`) and the `Backend` trait that platforms implement. No dependencies. |
| `crates/smitten-sdl` | The SDL3 backend. The only crate with `unsafe` code or a native library. |

## Running the examples

```
cargo run --example hello
cargo run --example sprites
cargo run --example input
```

SDL3 is built from source on the first build, which needs CMake and a C
compiler.

- **Windows:** Visual Studio Build Tools with the C++ workload, plus CMake.
- **macOS:** Xcode command line tools, plus CMake.
- **Linux:** CMake, a C compiler, and SDL's
  [build dependencies](https://wiki.libsdl.org/SDL3/README-linux#build-dependencies).

To link against an SDL3 that is already installed, build with
`--no-default-features`.
