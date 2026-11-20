+++
title = "Getting started"
description = "Install the tools, run the examples and make your first game."
weight = 1
+++

Smitten runs on Windows, Linux and macOS. Rendering uses Vulkan on Windows
and Linux and Metal on macOS.

## Install the tools

Smitten builds SDL3 from source the first time you build it, so besides
Rust you need CMake and a C compiler. The quickest way to get everything is
[just](https://github.com/casey/just) 1.44 or newer, which runs the
setup recipe in Smitten's repository.

1. Install `just` with `winget install Casey.Just`, `cargo install just`,
   or your package manager.
2. Clone the repository and run the setup recipe from its root:

   ```sh
   git clone https://github.com/charlesbthomas/smitten
   cd smitten
   just setup
   ```

What `just setup` installs depends on the system:

| System | What it installs |
|---|---|
| Windows | The Visual Studio Build Tools with the C++ workload, CMake and rustup, all through winget, then the stable Rust toolchain. Open a new terminal afterwards so the updated path takes effect. |
| Linux | CMake, a C compiler and SDL's build dependencies with apt, dnf or pacman, plus rustup and the stable toolchain. |
| macOS | Checks for the Xcode command line tools and installs CMake with Homebrew, plus rustup and the stable toolchain. |

To install things by hand instead:

- **Windows:** the Visual Studio Build Tools with the C++ workload, plus
  CMake. The Build Tools include a copy of CMake that is only on the path
  inside "Developer PowerShell for VS 2022", so either run cargo from there
  or install CMake separately.
- **Linux:** CMake, a C compiler and SDL's
  [build dependencies](https://wiki.libsdl.org/SDL3/README-linux#build-dependencies).
- **macOS:** the Xcode command line tools (`xcode-select --install`) and
  CMake.

Smitten uses the 2024 edition of Rust, so it needs Rust 1.85 or newer.

## Run the examples

The repository comes with example games. From the workspace root:

```sh
just run pong
# or, without just:
cargo run --example pong
```

The first build takes a few minutes because it compiles SDL3. Later builds
are quick. See [the examples page](@/examples/_index.md) for the full list.

## Make your own game

Make a new binary crate and add Smitten to it. Smitten isn't on crates.io
yet, so depend on it from Git:

```sh
cargo new my-game
cd my-game
```

```toml
# Cargo.toml
[dependencies]
smitten = { git = "https://github.com/charlesbthomas/smitten" }
```

Then replace `src/main.rs` with a small game:

```rust
use smitten::graphics::{self, DrawMode};
use smitten::{Color, Config, Game, Key, event, keyboard};

struct MyGame {
    x: f32,
}

impl Game for MyGame {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.1, 0.1, 0.2));
        Ok(MyGame { x: 400.0 })
    }

    fn update(&mut self, dt: f32) {
        self.x += keyboard::axis(&[Key::Left, Key::A], &[Key::Right, Key::D]) * 300.0 * dt;
    }

    fn draw(&mut self) {
        graphics::set_color(Color::rgb(0.95, 0.35, 0.45));
        graphics::circle(DrawMode::Fill, (self.x, 300.0), 40.0);
        graphics::set_color(Color::WHITE);
        graphics::print("Left and right move. Escape quits.", (20.0, 20.0));
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        if key == Key::Escape {
            event::quit();
        }
    }
}

fn main() {
    smitten::run::<MyGame>(Config {
        title: "My game".to_string(),
        ..Config::default()
    });
}
```

Run it with `cargo run`. A window opens with a circle you can move with the
arrow keys.

## Where assets go

Put images, sounds, fonts and shaders in an `assets` folder next to your
game's `Cargo.toml`. Paths you pass to loaders are relative to it:

```
my-game/
├── Cargo.toml
├── assets/
│   ├── player.png
│   └── jump.wav
└── src/
    └── main.rs
```

```rust
let player = Image::load("player.png")?;
```

In a released game, the `assets` folder sits next to the executable
instead. [Files, saves and data](@/guide/files.md) explains how paths are
looked up, and [Hot reload and releasing](@/guide/releasing.md) shows how
to build a game for players.

## Linking an installed SDL3

If SDL3 is already installed on your system, you can skip building it by
turning off the default `sdl-from-source` feature:

```toml
smitten = { git = "https://github.com/charlesbthomas/smitten", default-features = false, features = ["physics", "video"] }
```

## Next steps

- [The game loop](@/guide/game-loop.md) explains `load`, `update`, `draw`
  and the other callbacks.
- [Drawing](@/guide/drawing.md) covers shapes, images and text.
- [The examples](@/examples/_index.md) show complete games.
