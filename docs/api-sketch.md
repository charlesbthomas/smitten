# API sketch

This is the game Smitten should make easy to write. It is a target, and most
of it does not compile yet. The parts marked "works today" are implemented.

## A tiny game

```rust
use smitten::{audio, graphics, keyboard, Color, Config, Game, Key};
use smitten::audio::Source;
use smitten::graphics::{DrawMode, Font, Image};

struct Catch {
    player: Image,
    font: Font,
    blip: Source,
    x: f32,
    score: u32,
}

impl Game for Catch {
    fn load() -> Self {                                   // works today
        graphics::set_background_color(Color::rgb(0.1, 0.1, 0.2)); // works today
        Catch {
            player: graphics::new_image("player.png"),      // works today
            font: graphics::new_font("font.ttf", 24.0),
            blip: audio::new_source("blip.ogg", audio::SourceType::Static),
            x: 400.0,
            score: 0,
        }
    }

    fn update(&mut self, dt: f32) {                       // works today
        if keyboard::is_down(Key::Left) {                 // works today
            self.x -= 300.0 * dt;
        }
        if keyboard::is_down(Key::Right) {
            self.x += 300.0 * dt;
        }
    }

    fn draw(&mut self) {                                  // works today
        graphics::draw(&self.player, self.x, 500.0);      // works today
        graphics::set_color(Color::WHITE);                // works today
        graphics::rectangle(DrawMode::Fill, 0.0, 580.0, 800.0, 20.0); // works today
        graphics::set_font(&self.font);
        graphics::print(&format!("Score: {}", self.score), 10.0, 10.0);
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) { // works today
        if key == Key::Space {
            self.blip.play();
            self.score += 1;
        }
    }
}

fn main() {
    smitten::run::<Catch>(Config::default());             // works today
}
```

## How it maps to LÖVE

| LÖVE | Smitten | Notes |
| --- | --- | --- |
| `love.load()` | `Game::load() -> Self` | Returns the game state instead of filling globals. |
| `love.update(dt)` | `Game::update(&mut self, dt)` | |
| `love.draw()` | `Game::draw(&mut self)` | |
| `love.keypressed(key, scancode, isrepeat)` | `Game::key_pressed(key, is_repeat)` | |
| `love.quit()` | `Game::quit() -> bool` | Return `true` to keep running, as in LÖVE. |
| `conf.lua` | `Config` passed to `run` | |
| `love.graphics.draw(img, x, y)` | `graphics::draw(&img, x, y)` | |
| `love.graphics.draw(img, x, y, r, sx, sy, ox, oy)` | `graphics::draw_ex(&img, x, y, r, sx, sy, ox, oy)` | Same arguments in the same order. |
| `love.graphics.draw(img, quad, x, y)` | `graphics::draw_quad(&img, &quad, x, y)` | |
| `love.graphics.push()` / `pop()` | `graphics::push()` / `graphics::pop()` | |
| `love.graphics.setColor(r, g, b, a)` | `graphics::set_color(Color)` | Components are 0.0 to 1.0. |
| `love.graphics.newImage(path)` | `graphics::new_image(path)` | |
| `love.keyboard.isDown(key)` | `keyboard::is_down(Key)` | Keys are an enum, not strings. |
| `love.mouse.getPosition()` | `mouse::position()` | In pixels, like `graphics`. |
| `love.mouse.isDown(1)` | `mouse::is_down(MouseButton::Left)` | Buttons are an enum, not numbers. |
| `love.mousepressed(x, y, button, istouch, presses)` | `Game::mouse_pressed(x, y, button, presses)` | |
| `love.wheelmoved(x, y)` | `Game::wheel_moved(dx, dy)` | |
| `love.textinput(text)` | `Game::text_input(&str)` | |
| `love.joystick.getJoysticks()` | `joystick::joysticks()` | Returns `Vec<Joystick>`. |
| `joystick:isGamepadDown("a")` | `joystick.is_gamepad_down(GamepadButton::A)` | |
| `joystick:getGamepadAxis("leftx")` | `joystick.gamepad_axis(GamepadAxis::LeftX)` | |
| `love.gamepadpressed(joystick, button)` | `Game::gamepad_pressed(joystick, button)` | |
| `love.timer.getDelta()` | `timer::delta()` | Getters drop the `get` prefix, as Rust style prefers. |
| `love.event.quit()` | `event::quit()` | |

## Conventions

- **Names** follow LÖVE, converted to `snake_case`. Getters drop `get`, and
  setters keep `set_`.
- **Modules are global.** Any code on the main thread can call them while
  `smitten::run` is active. Calling them at any other time panics with a
  clear message.
- **Optional arguments.** LÖVE's `draw(img, x, y, r, sx, sy, ox, oy)` has
  optional parameters, which Rust lacks. Smitten has a short form
  (`draw(&img, x, y)`) and an `_ex` form that takes LÖVE's full argument
  list in the same order.
- **Loading errors.** `new_image` and friends panic with the file name on
  failure, matching LÖVE's error screen behavior. `try_new_image` variants
  return a `Result` for games that want to recover.
- **Resources** such as `Image` are cheap handles that free their GPU memory
  when the last copy is dropped.
