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
            font: graphics::new_font("font.ttf", 24.0),     // works today
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
        graphics::set_font(&self.font);                   // works today
        graphics::print(&format!("Score: {}", self.score), 10.0, 10.0); // works today
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
| `love.graphics.newFont(path, size)` | `graphics::new_font(path, size)` | |
| `love.graphics.newFont(size)` | `graphics::new_default_font(size)` | The same built-in font as LÖVE. |
| `love.graphics.print(text, x, y)` | `graphics::print(text, x, y)` | `print_ex` takes the rotation, scale and origin. |
| `love.graphics.printf(text, x, y, limit, align)` | `graphics::printf(text, x, y, limit, Align)` | |
| `font:getWidth(text)` / `font:getWrap(text, limit)` | `font.width(text)` / `font.wrap(text, limit)` | |
| `love.filesystem.read(name)` | `filesystem::read(name)` | Returns `io::Result`. Save directory first, then assets. |
| `love.filesystem.write(name, data)` | `filesystem::write(name, data)` | Always inside the save directory. |
| `t.identity` in `conf.lua` | `Config::identity` | Names the save directory. |
| `love.graphics.newCanvas(w, h)` | `graphics::new_canvas(w, h)` | A `Canvas` can be passed wherever an `&Image` is expected. |
| `love.graphics.setCanvas(canvas)` / `setCanvas()` | `graphics::set_canvas(Some(&canvas))` / `set_canvas(None)` | |
| `canvas:renderTo(fn)` | `canvas.render_to(\|\| { ... })` | |
| `love.graphics.setBlendMode("add")` | `graphics::set_blend_mode(BlendMode::Add)` | `Alpha`, `Add`, `Subtract`, `Multiply`, `Replace`. |
| `love.graphics.setBlendMode("alpha", "premultiplied")` | `graphics::set_blend_mode(BlendMode::Premultiplied)` | Use it to draw a canvas with see-through parts, as in LÖVE. |
| `love.graphics.setScissor(x, y, w, h)` / `setScissor()` | `graphics::set_scissor(x, y, w, h)` / `clear_scissor()` | |
| `love.graphics.stencil(fn)` | `graphics::stencil(\|\| { ... })` | `stencil_ex(action, value, keep_values, fn)` takes the other arguments. |
| `love.graphics.setStencilTest("greater", 0)` / `setStencilTest()` | `graphics::set_stencil_test(CompareMode::Greater, 0)` / `clear_stencil_test()` | |
| `love.graphics.newSpriteBatch(image, size)` | `graphics::new_sprite_batch(&image, size)` | Grows as needed; the size only reserves memory. |
| `batch:add(x, y)` / `batch:add(quad, x, y)` | `batch.add(x, y)` / `batch.add_quad(&quad, x, y)` | Return the sprite's index. `add_ex` and `add_quad_ex` take the full `draw` arguments. |
| `batch:set(id, x, y)` / `batch:setColor(r, g, b, a)` / `batch:clear()` | `batch.set(index, x, y)` / `batch.set_color(color)` / `batch.clear()` | Indexes start at 0. |
| `love.graphics.draw(batch, x, y)` | `graphics::draw_sprite_batch(&batch, x, y)` | |
| `love.graphics.newParticleSystem(image, max)` | `graphics::new_particle_system(&image, max)` | Setters take the same values as LÖVE's, such as `set_emission_rate`, `set_speed`, `set_sizes`, `set_colors`. |
| `psystem:update(dt)` / `psystem:emit(n)` | `system.update(dt)` / `system.emit(n)` | |
| `love.graphics.draw(psystem, x, y)` | `graphics::draw_particles(&system, x, y)` | |
| `love.graphics.newShader(code)` | `graphics::new_shader(code)` | WGSL, not GLSL. Defines `fn effect(color, uv, pixel) -> vec4<f32>`. `Shader::from_file(path)` loads a file. |
| `love.graphics.setShader(shader)` / `setShader()` | `graphics::set_shader(Some(&shader))` / `set_shader(None)` | |
| `shader:send(name, value)` | `shader.send(name, value)` | Sets a member of the shader's `Params` struct. |
| `love.audio.newSource(path, "static")` | `audio::new_source(path, SourceType::Static)` | `SourceType::Stream` for music. |
| `source:play()` / `pause()` / `stop()` | `source.play()` / `pause()` / `stop()` | |
| `source:setLooping(true)` / `setVolume(v)` / `setPitch(p)` | `source.set_looping(true)` / `set_volume(v)` / `set_pitch(p)` | |
| `source:seek(t)` / `tell()` / `getDuration()` | `source.seek(t)` / `tell()` / `duration()` | Seconds. `duration` is an `Option`. |
| `source:clone()` | `source.clone()` | An independent copy, so one sound can overlap itself. |
| `love.audio.setVolume(v)` / `pause()` / `stop()` | `audio::set_volume(v)` / `pause()` / `stop()` | |
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
| `love.math.random()` / `random(1, 6)` | `math::random()` / `math::random_int(1, 6)` | `random_range(min, max)` gives a fraction. Also `random_normal`, `chance`, `choose`, `shuffle`. |
| `love.math.setRandomSeed(seed)` / `newRandomGenerator(seed)` | `math::set_random_seed(seed)` / `RandomGenerator::new(seed)` | A seed gives the same numbers on every platform. |
| `love.math.noise(x, y)` | `math::noise2(x, y)` | `noise`, `noise2`, `noise3` by number of inputs. |
| `love.math.triangulate(points)` / `isConvex(points)` | `math::triangulate(&points)` / `math::is_convex(&points)` | |
| `love.window.setFullscreen(true)` / `getFullscreen()` | `window::set_fullscreen(true)` / `window::fullscreen()` | Desktop fullscreen. `Config::fullscreen` starts that way. |
| `love.window.setIcon(imagedata)` | `window::set_icon("icon.png")` | Or `Config::icon`. `set_icon_rgba` takes raw pixels. |
| `love.window.getDPIScale()` | `window::dpi_scale()` | Applied for you, as in LÖVE: coordinates are in units. `Config::dpi_scaling` is `usedpiscale`. |
| `love.graphics.getPixelDimensions()` | `graphics::pixel_dimensions()` | `graphics::dimensions()` is in units. |
| `love.graphics.newCanvas(w, h, {dpiscale = 1})` | `Canvas::with_dpi_scale(w, h, 1.0)` | `new_canvas` matches the screen's scale. |
| The push library, or a canvas scaled by hand | `Config::design_size` with `Scaling::Fit` or `Scaling::PixelPerfect` | Built in: a fixed design size scaled to the window, mouse included. |
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
