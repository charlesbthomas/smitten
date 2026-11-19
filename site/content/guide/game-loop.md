+++
title = "The game loop"
description = "The Game trait, its callbacks, Config, errors and the error screen."
weight = 2
+++

A Smitten game is a type that implements {{ api(path="trait.Game.html", text="Game") }}.
You hand it to {{ api(path="fn.run.html", text="smitten::run") }} along with a
{{ api(path="struct.Config.html", text="Config") }}, and Smitten opens the
window and calls your methods every frame.

```rust
use smitten::{graphics, Color, Config, Game};

struct Counter {
    seconds: f32,
}

impl Game for Counter {
    fn load() -> smitten::Result<Self> {
        Ok(Counter { seconds: 0.0 })
    }

    fn update(&mut self, dt: f32) {
        self.seconds += dt;
    }

    fn draw(&mut self) {
        graphics::clear(Color::rgb(0.1, 0.1, 0.2));
        graphics::print(&format!("{:.1} seconds", self.seconds), (40.0, 40.0));
    }
}

fn main() {
    smitten::run::<Counter>(Config::default());
}
```

## load, update and draw

- **`load`** is called once, after the window exists. It builds the game's
  starting state and returns it. It is the only method you must write.
- **`update(dt)`** is called every frame with the seconds since the last
  frame, as an `f32`. Move things here.
- **`draw`** is called every frame after `update`. Draw here. The screen is
  cleared to the background color before `draw` runs, so a game that calls
  {{ api(path="graphics/fn.set_background_color.html", text="graphics::set_background_color") }}
  once in `load` doesn't need to clear it.

The modules such as `graphics`, `audio` and `keyboard` can be
called from anywhere on the main thread, not only from these methods. You
never pass a context around.

## Other callbacks

Every other method has an empty default, so write only the ones you need:

| Method | Called when |
|---|---|
| `key_pressed(key, is_repeat)`, `key_released(key)` | A key goes down or up. |
| `text_input(text)`, `text_edited(text, start, length)` | The player types text, or an input method is composing it. |
| `mouse_moved(motion)`, `mouse_pressed(click)`, `mouse_released(click)`, `wheel_moved(dx, dy)` | The mouse moves, clicks or scrolls. |
| `touch_pressed`, `touch_moved`, `touch_released` | A finger touches, moves on or leaves a touch screen. |
| `joystick_added`, `joystick_removed` | A controller is plugged in or out. Each one already connected is reported at startup. |
| `gamepad_pressed`, `gamepad_released`, `gamepad_axis` | A gamepad button or stick changes. |
| `joystick_pressed`, `joystick_released`, `joystick_axis`, `joystick_hat` | A raw joystick input changes. |
| `resized(width, height)` | The window changes size. |
| `focus`, `mouse_focus`, `visible` | The window gains or loses focus, or is minimized. |
| `file_dropped`, `directory_dropped` | The player drops a file or folder on the window. |
| `custom_event(event)` | An event the game sent itself with `event::push` arrives. |
| `quit()` | The game is asked to close. Return `true` to keep running. |

[Input](@/guide/input.md) has examples of the input callbacks.

## Config

{{ api(path="struct.Config.html", text="Config") }} holds the game's
settings. Set the fields you care about and take the rest from
`Config::default()`:

```rust
smitten::run::<MyGame>(Config {
    title: "My game".to_string(),
    width: 1280,
    height: 720,
    resizable: true,
    vsync: true,
    msaa: 4,
    icon: Some("icon.png".into()),
    identity: Some("my-game".to_string()),
    ..Config::default()
});
```

Some of the fields:

| Field | What it does |
|---|---|
| `title`, `width`, `height` | The window's title and size in units. |
| `resizable`, `fullscreen`, `borderless` | How the window behaves. |
| `design_size`, `design_scaling` | Draw at a fixed size and scale it to the window. See [Windows and screens](@/guide/window-and-screens.md). |
| `dpi_scaling` | Measure in units that look the same size on every display (on by default). |
| `vsync`, `msaa` | Wait for vertical sync, and smooth edges with multisampling. |
| `icon` | An image file for the window icon. |
| `identity` | The name of the game's save directory. |
| `gamma_correct` | Blend colors in linear light. |
| `hot_reload` | Reload assets when their files change. On in debug builds. |

## Errors from load

`load` returns {{ api(path="type.Result.html", text="smitten::Result<Self>") }},
so loaders can use `?`:

```rust
fn load() -> smitten::Result<Self> {
    Ok(MyGame {
        player: Image::load("player.png")?,
        font: Font::load("font.ttf", 24.0)?,
        jump: Source::load("jump.wav", SourceType::Static)?,
    })
}
```

{{ api(path="struct.Error.html", text="smitten::Error") }} holds any error,
much as `anyhow::Error` does, and `{:#}` prints the causes too. An `Err`
from `load` stops the game and shows the error on the error screen.

A programming mistake, such as popping the transform stack more often
than you pushed, panics rather than returning an error.

## The error screen

When a game panics, the window doesn't vanish. It turns blue and shows the
message and the file and line it came from until the player presses Escape or closes the window. The message is still
printed to the terminal, and the program still ends with the panic.

{{ figure(src="error-screen.png", alt="Smitten's blue error screen, showing a panic message and the file and line it came from.", caption="The built-in error screen.") }}

A game can draw its own screen instead, or quit straight away, with
{{ api(path="fn.set_error_handler.html", text="set_error_handler") }}. Return
`Some(draw)` to draw every frame, or `None` to end the program:

```rust
use smitten::{graphics, Color};

smitten::set_error_handler(|report| {
    eprintln!("crashed: {}", report.message);
    let text = format!("Oops!\n\n{}", report.message);
    Some(move || {
        graphics::clear(Color::rgb(0.2, 0.0, 0.0));
        graphics::print(&text, (40.0, 40.0));
    })
});
```

{{ figure(src="custom-error-handler.png", alt="A custom error screen drawn by a game's own error handler.", caption="A screen drawn by a custom error handler.") }}

## Quitting

{{ api(path="event/fn.quit.html", text="event::quit") }} asks the game to
close. Smitten then calls your `quit` method, which can return `true` to
keep running, for instance to ask "Save before quitting?" first.

## Writing your own main loop

The `Game` trait covers nearly every game. If you need full control over
the frame, use
{{ api(path="fn.run_loop.html", text="smitten::run_loop") }}. It hands you a
{{ api(path="struct.MainLoop.html", text="MainLoop") }} with the steps of a
frame: `poll`, `step`, `clear` and `present`. This one runs its game logic
at a fixed 120 steps a second:

```rust
use smitten::event::Event;
use smitten::{graphics, Color, Config, Key};

fn main() {
    smitten::run_loop(Config::default(), |main| {
        let mut x = 0.0;
        main.step();
        let tick = 1.0 / 120.0;
        let mut owed = 0.0;
        'running: loop {
            for event in main.poll() {
                match event {
                    Event::Quit | Event::KeyPressed { key: Key::Escape, .. } => break 'running,
                    _ => {}
                }
            }
            owed += main.step();
            while owed >= tick {
                x += 60.0 * tick;
                owed -= tick;
            }
            if main.is_active() {
                main.clear(Color::rgb(0.1, 0.1, 0.2));
                graphics::circle(graphics::DrawMode::Fill, (x % 800.0, 300.0), 20.0);
                main.present();
            }
        }
    });
}
```
