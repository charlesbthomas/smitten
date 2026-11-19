+++
title = "Input"
description = "Keyboard, mouse, gamepads, joysticks and touch."
weight = 6
+++

Input comes two ways. Callbacks on your
{{ api(path="trait.Game.html", text="Game") }} report each change as it
happens, and the `keyboard`, `mouse`, `joystick` and `touch` modules answer
"is this held right now?" from anywhere.

Use callbacks for things that happen once, such as jumping or opening a
menu, and the modules for things that last, such as walking while a key is
held.

## Keyboard

```rust
use smitten::{Game, Key, event, keyboard};

impl Game for Player {
    // ...

    fn update(&mut self, dt: f32) {
        // -1, 0 or 1 from the arrow keys or A and D.
        let steer = keyboard::axis(&[Key::Left, Key::A], &[Key::Right, Key::D]);
        self.x += steer * 200.0 * dt;

        // Or both axes at once, never longer than 1 so diagonals aren't faster.
        self.position += keyboard::direction() * 200.0 * dt;

        if keyboard::is_down(Key::LeftShift) {
            // running
        }
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        match key {
            Key::Space if !is_repeat => self.jump(),
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}
```

`keyboard::is_any_down(&[...])` checks several keys at once. Key repeat is
off by default; `keyboard::set_key_repeat(true)` turns it on, and the
`is_repeat` argument then tells repeats from the first press.

{{ api(path="enum.Key.html", text="Key") }} names follow the key's position on
a US layout, so W, A, S and D are the same keys on an AZERTY keyboard.
`keyboard::layout_key` and `is_layout_down` work with the letter printed on
the key instead. A `Key` prints a short name with `Display` and parses it
with `FromStr`, so `"lshift".parse::<Key>()` works for saved key bindings.

### Typing text

For text fields, use the `text_input` callback rather than `key_pressed`. It
gets the typed text after the keyboard layout and any input method have
been applied:

```rust
fn text_input(&mut self, text: &str) {
    self.name.push_str(text);
}

fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
    if key == Key::Backspace {
        self.name.pop();
    }
}
```

## Mouse

```rust
use smitten::{Game, MouseButton, MouseClick, mouse};

impl Game for Painter {
    // ...

    fn update(&mut self, _dt: f32) {
        let position = mouse::position();
        if mouse::is_down(MouseButton::Left) {
            self.strokes.push(position);
        }
    }

    fn mouse_pressed(&mut self, click: MouseClick) {
        if click.button == MouseButton::Right {
            self.strokes.clear();
        }
        if click.presses == 2 {
            // a double click
        }
    }

    fn wheel_moved(&mut self, _dx: f32, dy: f32) {
        self.zoom *= 1.0 + dy * 0.1;
    }
}
```

The mouse callbacks take a struct: a
{{ api(path="event/struct.MouseClick.html", text="MouseClick") }} has the `position`,
`button` and `presses`, and a
{{ api(path="event/struct.MouseMotion.html", text="MouseMotion") }} has the
`position` and `delta`. Positions are in the same units you draw in,
including when the game has a [design size](@/guide/window-and-screens.md).

`mouse::set_visible`, `set_grabbed` and `set_relative_mode` hide, confine
and capture the pointer, and `mouse::set_cursor` picks a system cursor.
`mouse::ImageCursor::from_image_data` makes a cursor from your own picture.

## Gamepads and joysticks

Every connected controller is a {{ api(path="joystick/struct.Joystick.html", text="Joystick") }}.
Those SDL recognizes as gamepads (Xbox, PlayStation, Switch and most
others) can be read by button and stick names:

```rust
use smitten::{GamepadAxis, GamepadButton, Joystick, joystick};

fn update(&mut self, dt: f32) {
    for pad in joystick::gamepads() {
        let stick = pad.left_stick(0.2);
        self.position += stick * 200.0 * dt;
        if pad.is_gamepad_down(GamepadButton::A) {
            // ...
        }
        let throttle = pad.gamepad_axis(GamepadAxis::TriggerRight);
    }
}

fn gamepad_pressed(&mut self, pad: Joystick, button: GamepadButton) {
    if button == GamepadButton::Start {
        self.paused = !self.paused;
        pad.set_vibration(0.3, 0.3, 0.1);
    }
}
```

`left_stick` and `right_stick` take a dead zone and return a `Vec2`.
`set_vibration(left, right, seconds)` rumbles the heavy and light motors;
the `rumble` example lets you try it with a real controller.

Other joysticks can be read by their raw buttons, axes and hats, through
`joystick::joysticks()` and the `joystick_*` callbacks, or given a mapping
with `joystick::set_gamepad_mapping` to make them gamepads.
`joystick_added` and `joystick_removed` report controllers coming and
going, and each one already connected is reported at startup.

## Touch

The `touch_pressed`, `touch_moved` and `touch_released` callbacks take a
{{ api(path="event/struct.TouchPoint.html", text="TouchPoint") }} with an `id`,
`position`, `delta` and `pressure`. `touch::touches()` lists the fingers on
the screen now, and `touch::position(id)` says where one is. The first
finger is also reported as the mouse, so a game that only needs taps can
use `mouse_pressed` alone.

## Trying it out

The `input` example shows every kind of input in one window:

```sh
cargo run --example input
```
