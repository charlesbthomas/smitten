+++
title = "Coming from LÖVE"
description = "How LÖVE code translates into Smitten: names, points, drawing, errors and what is left out."
weight = 13
+++

Smitten keeps LÖVE's shape: a game has `load`, `update` and `draw`, and
modules such as `graphics`, `keyboard` and `audio` can be called from
anywhere. The names and argument lists follow Rust conventions instead
of Lua's, so a call rarely ports word for word. This page lists the
patterns that change. LÖVE names that changed are search aliases in
the [API docs](../../api/smitten/index.html): search for
`love.graphics.newImage` or `Source:getDuration` and the docs show the
Smitten item that does it.

[What is left out](#what-is-left-out) lists what LÖVE has that Smitten
leaves out on purpose, and [LÖVE name lookup](@/guide/love-names.md) finds
the Smitten name for any LÖVE function whose name changed.

## A small game

```rust
use smitten::audio::{Source, SourceType};
use smitten::graphics::{self, DrawMode, DrawParams, Font, Image};
use smitten::{Color, Config, Game, Key, keyboard};

struct Catch {
    player: Image,
    font: Font,
    blip: Source,
    x: f32,
    score: u32,
}

impl Game for Catch {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.1, 0.1, 0.2));
        Ok(Catch {
            player: Image::load("player.png")?,
            font: Font::load("font.ttf", 24.0)?,
            blip: Source::load("blip.ogg", SourceType::Static)?,
            x: 400.0,
            score: 0,
        })
    }

    fn update(&mut self, dt: f32) {
        if keyboard::is_down(Key::Left) {
            self.x -= 300.0 * dt;
        }
        if keyboard::is_down(Key::Right) {
            self.x += 300.0 * dt;
        }
    }

    fn draw(&mut self) {
        let center = self.player.size() / 2.0;
        graphics::draw(&self.player, DrawParams::at((self.x, 500.0)).origin(center));
        graphics::rectangle(DrawMode::Fill, (0.0, 580.0), (800.0, 20.0));
        graphics::set_font(&self.font);
        graphics::print(&format!("Score: {}", self.score), (10.0, 10.0));
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        if key == Key::Space {
            self.blip.play();
            self.score += 1;
        }
    }
}

fn main() {
    smitten::run::<Catch>(Config::default());
}
```

## Names

| LÖVE | Smitten |
|---|---|
| `getWidth`, `getColor` | `width()`, `color()`: getters drop `get`. |
| `isDown`, `isPlaying` | `is_down`, `is_playing`. |
| `setColor` | `set_color`. |
| `love.graphics.newImage(path)` | `Image::load(path)`, which returns a `Result`. |
| `love.graphics.newCanvas(w, h)` | `Canvas::new(w, h)`, also a `Result`. |
| `newX` from data in memory | `X::from_bytes`, `X::from_image_data` and so on. |
| An optional settings table | A `..._with` constructor that takes a settings struct: `Image::load_with(path, ImageSettings { mipmaps: true, ..Default::default() })`, `Canvas::with_settings`, `Font::load_with(path, size, FontSettings { .. })`. |
| `love.load` | `Game::load`, which returns `smitten::Result<Self>`. An `Err` is shown on the error screen. |
| `love.mousepressed(x, y, button, istouch, presses)` | `Game::mouse_pressed(click: MouseClick)`, with `position`, `button` and `presses`. Touch callbacks take a `TouchPoint`. |

## Points and sizes

A pair of numbers that belong together is one argument. Anything that
converts into a `Vec2` works: a tuple, an array or a `Vec2`.

```rust
graphics::rectangle(DrawMode::Fill, (x, y), (width, height));
graphics::line((x1, y1), (x2, y2));
let world = World::new((0.0, 9.81 * 30.0));
let position: Vec2 = body.position();
```

Positions, velocities and other vectors come back as `Vec2` (re-exported
from [glam](https://docs.rs/glam)), which has the arithmetic built in.
Audio positions are 3D and use `Vec3`.

## Drawing

LÖVE's `draw(drawable, x, y, r, sx, sy, ox, oy, kx, ky)` is one function
with one placement argument:

```rust
graphics::draw(&image, (x, y));
graphics::draw(
    &image,
    DrawParams::at((x, y)).rotation(angle).scale(2.0).origin(image.size() / 2.0),
);
graphics::draw(&image, &transform);
```

| LÖVE | Smitten |
|---|---|
| `draw(image, quad, ...)` | `draw(&image.region(&quad), ...)` |
| `drawLayer(array, layer, ...)` | `draw(&array.layer(layer), ...)` |
| `drawInstanced(mesh, n, ...)` | `draw_instanced(&mesh, n, ...)` |
| `print(coloredtext, ...)` | `print(&[(color, "text"), ...], ...)`: `print` and `printf` take a string or colored parts. |
| `printf(text, x, y, limit, align, r, ...)` | `printf(text, params, limit, align)` |
| `clear(color, stencil, depth)` | `clear(color)`, or `clear(ClearValues::NONE.stencil(0))` to clear only some parts. |
| `setBlendMode(mode, alphamode)` | `set_blend_mode(mode)` or `set_blend_mode(mode.with_alpha(alpha))`. |
| `stencil(fn, action, value, keep)` | `stencil(fn)`, or `stencil_with(StencilWrite { .. }, fn)`. |
| `push()` ... `pop()` | The same, or `graphics::scoped(|| ...)`, which pops for you. |
| `setCanvas(canvas)` ... `setCanvas()` | The same, or `canvas.render_to(|| ...)`. |
| `discard`, `flushBatch`, `SpriteBatch:flush` | Not needed. Smitten keeps every target's contents and sends the frame at once. |

`draw` takes anything `Drawable`: images, canvases, regions, layers,
sprite batches, meshes, particle systems, `Text` objects and videos.

## Errors

Loading returns `Result`, so `load` can use `?`. `smitten::Error` holds any
error, much as `anyhow::Error` does, and `{:#}` prints its causes.
Functions that LÖVE makes raise an error on a programming mistake, such as
popping more than was pushed, panic instead.

## Time

Durations of sounds and videos, `Source::seek` and `tell`, and
`timer::sleep` use `std::time::Duration`. The `dt` given to `update` is
an `f32` in seconds, as in LÖVE.

## Physics

Each `new_..._joint` method returns the joint's own type, such as
`RevoluteJoint`, which has only the methods that kind of joint has, so
asking a weld joint for its motor speed does not compile. `Joint` holds
what every joint has; `joint.typed()` gives an `AnyJoint` to match on.
Shapes work the same way: `Shape::circle` returns a `CircleShape`.

LÖVE's contact callbacks are `World::contacts()`, read after each update,
plus `World::set_pre_solve` for one-way platforms and the like.

## Audio

A queueable source is a `QueueableSource`, the only kind that has
`queue`. It derefs to `Source` for everything else.

## Values with names

`Key`, `PixelFormat` and `CompressedFormat` print LÖVE's name with
`Display` and parse it with `FromStr`, so `"lshift".parse::<Key>()` and
`Key::LeftShift.to_string()` work, for saving key bindings.

## Colors

Colors are `Color` values with components from 0 to 1, as in LÖVE 11.
`Color::from_rgba8(255, 128, 0, 255)` and `color.to_rgba8()` replace
`colorFromBytes` and `colorToBytes`, and `to_linear` and `to_gamma`
replace `gammaToLinear` and `linearToGamma`.

## Your own main loop

Replacing `love.run` is `smitten::run_loop`, which hands a `MainLoop`
with the steps of a frame: `poll`, `step`, `clear` and `present`.

## Modules

Each LÖVE module is a Smitten module with the same name.

| LÖVE | Smitten | Guide |
|---|---|---|
| `love.graphics` | `smitten::graphics` | [Drawing](@/guide/drawing.md), [Canvases and shaders](@/guide/canvases-and-shaders.md) |
| `love.audio` | `smitten::audio` | [Audio](@/guide/audio.md) |
| `love.sound` | `smitten::sound` | [Audio](@/guide/audio.md#sounds-made-in-code) |
| `love.keyboard`, `love.mouse`, `love.joystick`, `love.touch` | `smitten::keyboard`, `mouse`, `joystick`, `touch` | [Input](@/guide/input.md) |
| `love.physics` | `smitten::physics` | [Physics](@/guide/physics.md) |
| `love.filesystem`, `love.data` | `smitten::filesystem`, `data` | [Files, saves and data](@/guide/files.md) |
| `love.window` | `smitten::window` | [Windows and screens](@/guide/window-and-screens.md) |
| `love.timer`, `love.math`, `love.event`, `love.system` | `smitten::timer`, `math`, `event`, `system` | [Timing, math and system](@/guide/timing-math-system.md) |
| `love.image`, `love.font` | `ImageData` in `smitten::graphics`, and `smitten::font` | [Drawing](@/guide/drawing.md) |
| `love.video` | `graphics::Video`, and `smitten::video` | [Audio](@/guide/audio.md#video) |
| `love.thread` | `std::thread` plus `smitten::thread::Channel` | [Files, saves and data](@/guide/files.md#threads) |
| `conf.lua` | `smitten::Config` | [The game loop](@/guide/game-loop.md#config) |
| `love.run` | `smitten::run_loop` | [The game loop](@/guide/game-loop.md#writing-your-own-main-loop) |
| `love.errorhandler` | `smitten::set_error_handler` | [The game loop](@/guide/game-loop.md#the-error-screen) |

## Callbacks

LÖVE's callbacks are methods on the `Game` trait, in snake case:
`love.keypressed` is `key_pressed`, `love.mousepressed` is `mouse_pressed`,
`love.gamepadaxis` is `gamepad_axis`, and so on. `love.resize` is
`resized`, `love.filedropped` is `file_dropped`, and the contact callbacks
of `love.physics` are `World::contacts`.

## Keys

`keyboard::is_down` checks a key by its position, which LÖVE calls a
scancode, so W, A, S and D stay put on an AZERTY keyboard. It is LÖVE's
`isScancodeDown`. `keyboard::is_layout_down` checks the key by its label,
which is what LÖVE's `isDown` does; use it for shortcuts named after a
letter, such as Ctrl+Z.

## What is left out

Every LÖVE 11.5 function, method, callback and `love.conf` field that
means something in Rust on the desktop is covered. These are left out on
purpose:

| Item | Why |
|---|---|
| ETC and PVRTC textures | SDL's GPU API has no ETC or PVRTC formats, so such DDS and KTX files are refused with that reason. KTX 2 files are refused too; KTX 1 files load. |
| `drawLayer` with a custom shader | A custom shader's `image` is a `texture_2d<f32>`. Send the `ArrayImage` to the shader with `Shader::send_texture` and pick the layer there instead. |
| macOS and web | The renderer is Vulkan only. |
| Lua-only filesystem functions | `load`, `getRequirePath`, `setRequirePath`, `getCRequirePath`, `setCRequirePath`, `init`, `setSource` and the like have no Rust meaning. |
| `love.threaderror`, the `Thread` type | Rust threads report panics through `JoinHandle`. |
| `Object`, `Data` base types | Rust ownership and slices do their job. |
| `setDeprecationOutput`, `hasDeprecationOutput` | Smitten has no deprecated LÖVE names to warn about. |
| LÖVE's GLSL shader dialect | Smitten shaders are WGSL. |
| `t.externalstorage`, `t.accelerometerjoystick`, `t.audio.mic` | They only do anything on Android and iOS, which Smitten doesn't run on. |
| `t.modules.*` | The `physics` and `video` Cargo features leave those modules out; the rest are always built in. |
| `love.graphics.discard`, `flushBatch`, `SpriteBatch:flush` | Smitten's targets always keep their contents and a frame is sent to the GPU at once, so there is nothing for them to do. |
| `setNewFont` | `set_font(&Font::load(path, size)?)`. |
