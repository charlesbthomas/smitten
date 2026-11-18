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
| `love.graphics.push("all")` | `graphics::push_all()` | Saves every drawing setting, not only the transform. `push_ex(StackType)` takes the kind. `pop` restores either. |
| `love.graphics.getStackDepth()` | `graphics::stack_depth()` | |
| `love.graphics.reset()` | `graphics::reset()` | |
| `love.graphics.newFont(path, size)` | `graphics::new_font(path, size)` | A `.fnt` path loads a BMFont. `new_font_ex` takes the hinting and DPI scale. |
| `love.graphics.newFont(size)` | `graphics::new_default_font(size)` | The same built-in font as LÖVE. |
| `love.graphics.newImageFont(path, glyphs)` | `graphics::new_image_font(path, glyphs)` | Same image layout as LÖVE. `new_image_font_ex` takes the extra spacing. |
| `love.graphics.print(text, x, y)` | `graphics::print(text, x, y)` | `print_ex` takes the rotation, scale and origin. |
| `love.graphics.printf(text, x, y, limit, align)` | `graphics::printf(text, x, y, limit, Align)` | |
| `font:getWidth(text)` / `font:getWrap(text, limit)` | `font.width(text)` / `font.wrap(text, limit)` | |
| `font:setFallbacks(...)` / `font:hasGlyphs(text)` | `font.set_fallbacks(&[&other])` / `font.has_glyphs(text)` | |
| `love.filesystem.read(name)` | `filesystem::read(name)` | Returns `io::Result`. Save directory first, then assets. |
| `love.filesystem.write(name, data)` | `filesystem::write(name, data)` | Always inside the save directory. |
| `t.identity` in `conf.lua` | `Config::identity` | Names the save directory. |
| `love.graphics.newCanvas(w, h)` | `graphics::new_canvas(w, h)` | A `Canvas` can be passed wherever an `&Image` is expected. |
| `love.graphics.setCanvas(canvas)` / `setCanvas()` | `graphics::set_canvas(Some(&canvas))` / `set_canvas(None)` | |
| `canvas:renderTo(fn)` | `canvas.render_to(\|\| { ... })` | |
| `love.graphics.setBlendMode("add")` | `graphics::set_blend_mode(BlendMode::Add)` | `Alpha`, `Add`, `Subtract`, `Multiply`, `Replace`, `Screen`, `Lighten`, `Darken`. |
| `love.graphics.setBlendMode("add", "premultiplied")` | `graphics::set_blend_mode_ex(BlendMode::Add, BlendAlphaMode::Premultiplied)` | `blend_mode_ex()` returns both, as `getBlendMode`. |
| `love.graphics.setBlendMode("alpha", "premultiplied")` | `graphics::set_blend_mode(BlendMode::Premultiplied)` | Use it to draw a canvas with see-through parts, as in LÖVE. |
| `love.graphics.setScissor(x, y, w, h)` / `setScissor()` | `graphics::set_scissor(x, y, w, h)` / `clear_scissor()` | |
| `love.graphics.intersectScissor(x, y, w, h)` | `graphics::intersect_scissor(x, y, w, h)` | |
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
| `joystick:setVibration(left, right, duration)` | `joystick.set_vibration(left, right, seconds)` | `stop_vibration()` ends it. `is_vibration_supported()` asks first. |
| `love.gamepadpressed(joystick, button)` | `Game::gamepad_pressed(joystick, button)` | |
| `love.math.random()` / `random(1, 6)` | `math::random()` / `math::random_int(1, 6)` | `random_range(min, max)` gives a fraction. Also `random_normal`, `chance`, `choose`, `shuffle`. |
| `love.math.setRandomSeed(seed)` / `newRandomGenerator(seed)` | `math::set_random_seed(seed)` / `RandomGenerator::new(seed)` | A seed gives the same numbers on every platform. |
| `love.math.noise(x, y)` | `math::noise2(x, y)` | `noise`, `noise2`, `noise3` by number of inputs. |
| `love.math.triangulate(points)` / `isConvex(points)` | `math::triangulate(&points)` / `math::is_convex(&points)` | |
| `love.math.newBezierCurve(points)` | `math::BezierCurve::new(&points)` | `evaluate`, `render`, `derivative`, `segment` and the rest keep their names. |
| `love.window.setFullscreen(true)` / `getFullscreen()` | `window::set_fullscreen(true)` / `window::fullscreen()` | Desktop fullscreen. `Config::fullscreen` starts that way. |
| `love.window.setIcon(imagedata)` | `window::set_icon("icon.png")` | Or `Config::icon`. `set_icon_rgba` takes raw pixels. |
| `love.window.getDPIScale()` | `window::dpi_scale()` | Applied for you, as in LÖVE: coordinates are in units. `Config::dpi_scaling` is `usedpiscale`. |
| `love.graphics.getPixelDimensions()` | `graphics::pixel_dimensions()` | `graphics::dimensions()` is in units. |
| `love.graphics.newCanvas(w, h, {dpiscale = 1})` | `Canvas::with_dpi_scale(w, h, 1.0)` | `new_canvas` matches the screen's scale. |
| The push library, or a canvas scaled by hand | `Config::design_size` with `Scaling::Fit` or `Scaling::PixelPerfect` | Built in: a fixed design size scaled to the window, mouse included. |
| `love.physics.newWorld(0, 9.81 * 64)` | `physics::World::new(0.0, 9.81 * 64.0)` | `world.set_meter(64.0)` replaces the global `setMeter`. |
| `love.physics.newBody(world, x, y, "dynamic")` | `world.new_body(x, y, BodyType::Dynamic)` | `Static`, `Dynamic`, `Kinematic`. |
| `newFixture(body, newRectangleShape(w, h))` | `body.add_rectangle(w, h)` | Shape and fixture in one step: `add_circle`, `add_polygon`, `add_edge`, `add_chain`. |
| `fixture:setRestitution(0.5)` / `setFriction` / `setSensor` | `fixture.set_restitution(0.5)` and so on | Setters chain. |
| `body:getPosition()` / `applyLinearImpulse(x, y)` | `body.position()` / `body.apply_linear_impulse(x, y)` | |
| `world:update(dt)` | `world.update(dt)` | |
| `world:setCallbacks(beginContact, endContact)` | `world.contacts()` after each update | A list to loop over instead of callbacks. |
| `contact:getPositions()`, `contact:getNormal()` | `contact.x`, `contact.y`, `contact.normal`, `contact.speed` | Where the two met and how fast they closed, so a landing can be told from a brush. |
| *(none)* | `body.apply_acceleration(x, y)`, `body.apply_angular_acceleration(a)` | A push in pixels (or radians) per second squared that does not depend on the body's mass or the world's meter. |
| `body:getInertia()` | `body.inertia()` | |
| *(tables keyed by body)* | `HashMap<Body, T>` | `Body`, `Fixture` and `Joint` are `Eq + Hash`. |
| `world:rayCast(...)` / `queryBoundingBox` | `world.ray_cast(x1, y1, x2, y2)` / `world.fixtures_at(x, y)` | `ray_cast_all` for every hit, nearest first. |
| `love.physics.newRevoluteJoint(a, b, x, y)` | `world.new_revolute_joint(&a, &b, x, y, collide)` | Also weld, rope, spring, distance, prismatic, wheel, mouse, friction and motor joints. Gear and pulley joints are missing. |
| `love.graphics.ellipse("fill", x, y, rx, ry)` | `graphics::ellipse(DrawMode::Fill, x, y, rx, ry)` | |
| `love.graphics.arc("line", "open", x, y, r, a1, a2)` | `graphics::arc(DrawMode::Line, ArcType::Open, x, y, r, a1, a2)` | `Pie`, `Open` or `Closed`. |
| `love.graphics.rectangle("fill", x, y, w, h, rx, ry)` | `graphics::rounded_rectangle(DrawMode::Fill, x, y, w, h, rx, ry)` | |
| `love.graphics.points(...)` / `setPointSize(4)` | `graphics::points(&points)` / `graphics::set_point_size(4.0)` | |
| `love.graphics.line(x1, y1, x2, y2, x3, y3, ...)` | `graphics::polyline(&points)` | Corners are joined. `graphics::line` stays the two-point form. |
| `love.system.getOS()` / `getProcessorCount()` | `system::os()` / `system::processor_count()` | |
| `love.system.setClipboardText(text)` / `getClipboardText()` | `system::set_clipboard_text(text)` / `system::clipboard_text()` | |
| `love.system.openURL(url)` | `system::open_url(url)` | Returns whether it opened. |
| `love.system.getPowerInfo()` | `system::power_info()` | A struct with the state, percent and seconds left. |
| `love.mouse.setCursor(love.mouse.getSystemCursor("hand"))` | `mouse::set_cursor(Cursor::Hand)` | |
| `love.mouse.setRelativeMode(true)` / `setGrabbed(true)` | `mouse::set_relative_mode(true)` / `mouse::set_grabbed(true)` | |
| `love.mouse.setPosition(x, y)` | `mouse::set_position(x, y)` | In units, like `mouse::position()`. |
| `love.filesystem.getDirectoryItems(dir)` | `filesystem::directory_items(dir)` | Sorted. Saved and shipped files together, as in LÖVE. |
| `love.filesystem.getInfo(path)` | `filesystem::info(path)` | `Option<FileInfo>`. Also `is_file` and `is_directory`. |
| `love.filesystem.lines(path)` | `filesystem::lines(path)` | |
| `love.image.newImageData(w, h)` / `newImageData(path)` | `ImageData::new(w, h)` / `graphics::new_image_data(path)` | |
| `data:getPixel(x, y)` / `setPixel` / `mapPixel` / `paste` | `data.pixel(x, y)` / `set_pixel` / `map_pixels` / `paste` | Colors are `Color`. |
| `data:encode("png", path)` | `data.save(path)` | Or `data.encode_png()` for the bytes. |
| `love.graphics.newImage(data)` / `image:replacePixels(data)` | `Image::from_image_data(&data)` / `image.replace_pixels(&data, x, y)` | |
| `image:setWrap("repeat", "repeat")` | `image.set_wrap(Wrap::Repeat, Wrap::Repeat)` | `Clamp`, `Repeat` or `MirroredRepeat`. A quad larger than the image then tiles it. |
| `love.graphics.newMesh(vertices, "fan")` | `graphics::new_mesh(&vertices, MeshDrawMode::Fan)` | Vertices are `MeshVertex { x, y, u, v, color }`. `Fan`, `Strip` or `Triangles`. |
| `mesh:setTexture(image)` / `setVertexMap` / `setDrawRange` | `mesh.set_texture(Some(&image))` / `set_vertex_map` / `set_draw_range` | |
| `love.graphics.draw(mesh, x, y)` | `graphics::draw_mesh(&mesh, x, y)` | |
| `canvas:newImageData()` | `canvas.new_image_data()` | Returns a `Result`. Slow, since it waits for the GPU. |
| `love.graphics.captureScreenshot(callback)` | `graphics::capture_screenshot(\|picture\| ...)` | The callback gets an `ImageData` at the end of the frame. |
| `love.graphics.captureScreenshot("shot.png")` | `graphics::save_screenshot("shot.png")` | Written to the save directory. |
| `love.graphics.setLineJoin("bevel")` | `graphics::set_line_join(LineJoin::Bevel)` | `Miter` (the default), `Bevel` or `None`. |
| *(none)* | `graphics::dashed_line(x1, y1, x2, y2, dash, gap)`, `graphics::dashed_polyline(points, dash, gap)` | LÖVE has no dashed lines. |
| *(none)* | `graphics::shadow(light_x, light_y, polygon, length)` | The shadow a convex shape casts away from a light, for stencil lighting. |
| `love.graphics.rectangle` under a shader | the same | A filled shape's `uv` runs 0 to 1 across the box around it, so a shader can paint a shape with no image. |
| *(none)* | `keyboard::axis(negative, positive)`, `keyboard::direction()` | Keys as -1, 0 or 1, and arrows plus WASD as a direction no longer than 1. |
| *(none)* | `pad.left_stick(dead_zone)`, `pad.right_stick(dead_zone)` | A stick as `(x, y)` with the dead zone taken out. |
| *(none)* | `color.lerp(other, t)`, `color.with_alpha(a)` | Blend two colors, or fade one. |
| `source:clone():play()` | `source.play_once()`, `source.play_once_with(volume, pitch)` | Plays a copy that cleans itself up, so one sound can overlap itself. |
| `love.graphics.setLineStyle("rough")` | `graphics::set_line_style(LineStyle::Rough)` | Lines start out `Smooth`, as in LÖVE. |
| `love.window.setMode(w, h)` | `window::set_size(w, h)` | Only the size. Other window flags are set in `Config`. |
| `love.window.getPosition()` / `setPosition(x, y)` | `window::position()` / `window::set_position(x, y)` | |
| `love.window.getTitle()` | `window::title()` | |
| `love.window.minimize()` / `maximize()` / `restore()` | `window::minimize()` / `maximize()` / `restore()` | |
| `love.window.isMinimized()` / `isMaximized()` / `isVisible()` | `window::is_minimized()` / `is_maximized()` / `is_visible()` | |
| `love.window.hasFocus()` / `hasMouseFocus()` | `window::has_focus()` / `window::has_mouse_focus()` | |
| `love.window.setVSync(1)` / `getVSync()` | `window::set_vsync(true)` / `window::vsync()` | On or off. Adaptive vsync is not offered. |
| `love.window.getDesktopDimensions()` / `getDisplayCount()` | `window::desktop_dimensions()` / `window::display_count()` | The desktop the window is on. |
| `love.window.showMessageBox(title, message, "warning")` | `window::show_message_box(title, message, MessageBoxKind::Warning)` | One OK button. |
| `love.window.showMessageBox(title, message, {"Quit", "Stay"}, "warning")` | `window::show_message_box_with_buttons(title, message, MessageBoxKind::Warning, &["Quit", "Stay"])` | Returns the button's index from 0 as an `Option`. Enter picks the first button and Escape the last; they cannot be chosen yet. |
| `love.window.requestAttention(continuous)` | `window::request_attention(continuous)` | |
| `love.focus(f)` / `love.mousefocus(f)` / `love.visible(v)` | `Game::focus` / `Game::mouse_focus` / `Game::visible` | |
| `love.audio.setPosition(x, y, z)` / `setOrientation(fx, fy, fz, ux, uy, uz)` / `setVelocity(x, y, z)` | `audio::set_position(x, y, z)` / `set_orientation((fx, fy, fz), (ux, uy, uz))` / `set_velocity(x, y, z)` | The listener. Getters drop `get`, as everywhere. |
| `love.audio.setDistanceModel("inverseclamped")` / `setDopplerScale(s)` | `audio::set_distance_model(DistanceModel::InverseClamped)` / `set_doppler_scale(s)` | The same OpenAL formulas as LÖVE. |
| `source:setPosition(x, y, z)` / `setVelocity(x, y, z)` / `setDirection(x, y, z)` / `setRelative(true)` | `source.set_position(x, y, z)` / `set_velocity(x, y, z)` / `set_direction(x, y, z)` / `set_relative(true)` | Mono sources only, as in LÖVE. Stereo sources are heard as they are rather than raising an error. |
| `source:setAttenuationDistances(ref, max)` / `setRolloff(r)` / `setCone(inner, outer, vol)` / `setVolumeLimits(min, max)` | `source.set_attenuation_distances(reference, max)` / `set_rolloff(r)` / `set_cone(inner, outer, volume)` / `set_volume_limits(min, max)` | The maximum volume limit starts unlimited rather than at 1.0, so volumes above 1.0 keep working. |
| `source:setFilter{type = "lowpass", highgain = 0.2}` / `setFilter()` | `source.set_filter(Some(Filter::low_pass(0.2)))` / `set_filter(None)` | `Filter::high_pass` and `Filter::band_pass` too, or build a `Filter` with a `volume`. |
| `love.audio.setEffect("hall", {type = "reverb", decaytime = 3})` / `setEffect("hall", false)` | `audio::set_effect("hall", Reverb { decay_time: 3.0, ..Default::default() })` / `audio::remove_effect("hall")` | Every LÖVE effect: `Reverb`, `Chorus`, `Distortion`, `Echo`, `Flanger`, `Compressor`, `Equalizer`, `RingModulator`, with LÖVE's defaults. |
| `source:setEffect("hall")` / `setEffect("hall", false)` / `getActiveEffects()` | `source.set_effect("hall", None)` / `source.remove_effect("hall")` / `source.active_effects()` | The second argument is an optional `Filter` on the send. Returns `false` if there is no such effect. |
| `love.sound.newSoundData(path)` / `newSoundData(samples, rate, bits, channels)` | `sound::new_sound_data(path)` / `SoundData::new(samples, rate, channels)` | Samples are `f32` from -1.0 to 1.0, so there is no bit depth. |
| `sounddata:getSample(i, channel)` / `setSample(i, channel, v)` / `getSampleCount()` | `data.sample(i, channel)` / `data.set_sample(i, channel, v)` / `data.sample_count()` | Channels count from 0. `samples()` gives the whole interleaved slice. |
| `love.audio.newSource(sounddata)` | `Source::from_sound_data(&data)` | Copies the samples, as LÖVE does. |
| `love.touch.getTouches()` | `touch::touches()` | Returns `Vec<TouchId>`, oldest finger first. |
| `love.touch.getPosition(id)` / `getPressure(id)` | `touch::position(id)` / `touch::pressure(id)` | `None` once the finger has lifted, where LÖVE raises an error. |
| `love.touchpressed(id, x, y, dx, dy, pressure)` | `Game::touch_pressed(id, x, y, dx, dy, pressure)` | Also `touch_moved` and `touch_released`. A canceled touch counts as released. |
| `love.mouse.newCursor(path, hotx, hoty)` / `setCursor(cursor)` | `mouse::new_cursor(path, hot_x, hot_y)` / `mouse::set_image_cursor(&cursor)` | `ImageCursor::from_image_data` takes an `ImageData`. `mouse::image_cursor()` is `getCursor`. One image pixel per screen pixel. |
| `love.filesystem.mount(archive, mountpoint)` / `unmount(archive)` | `filesystem::mount(archive, mount_point)` / `filesystem::unmount(archive)` | Zip archives or directories. `mount_after` is `appendToPath = true`. Images, fonts, shaders and sounds load from mounted files too. |
| `love.audio.newQueueableSource(rate, bits, channels, buffers)` / `source:queue(sounddata)` / `getFreeBufferCount()` | `audio::new_queueable_source(rate, channels, buffers)` / `source.queue(&data)` / `source.free_buffer_count()` | `queue` returns `false` when full. A queue that runs dry stops; `play` it again after queueing. Stopping empties it. It cannot loop or seek. |
| `love.sound.newDecoder(path, bufferSize)` / `decoder:decode()` / `seek(s)` / `clone()` | `sound::new_decoder(path, samples)` / `decoder.decode()` / `decoder.seek(s)` / `decoder.clone()` | The buffer size counts samples per channel, not bytes. `decode` gives `Option<SoundData>`. |
| `love.audio.newSource(decoder, "stream")` | `Source::from_decoder(&decoder, SourceType::Stream)` | Plays the decoder's file from its start. |
| `love.audio.getRecordingDevices()` / `device:start(samples, rate, bits, channels)` / `getData()` / `stop()` | `audio::recording_devices()` / `device.start(samples, rate, channels)` / `device.data()` / `device.stop()` | No bit depth. A full buffer drops its oldest samples. `start` returns a `Result`. |
| `love.graphics.newVideo(path)` / `love.graphics.draw(video, x, y)` | `graphics::new_video(path)` / `graphics::draw_video(&video, x, y)` | Ogg Theora, with an Ogg Vorbis sound track if there is one. `video.image()` gives the current picture for quads and shaders. |
| `video:play()` / `pause()` / `seek(s)` / `tell()` / `rewind()` / `getSource()` | `video.play()` / `pause()` / `seek(s)` / `tell()` / `rewind()` / `source()` | The picture follows the sound when there is some. It stops on the last frame. |
| `love.video.newVideoStream(path)` | `video::new_video_stream(path)` | A decoder only: `seek(s)` decodes the frame at that time and `pixels()` reads it as RGBA. The `Video` keeps time. |
| `love.math.newTransform(x, y, r, sx, sy, ox, oy, kx, ky)` | `math::new_transform(x, y, r, sx, sy, ox, oy, kx, ky)` | Or `Transform::new()`. `translate`, `rotate`, `scale`, `shear`, `set_transformation`, `reset`, `inverse`, `transform_point`, `inverse_transform_point` keep their names. `transform:apply(other)` is `apply_transform(&other)`, or `outer * inner`. |
| `transform:getMatrix()` / `setMatrix(...)` | `transform.matrix()` / `transform.set_matrix(rows)` | A 3 by 3 matrix in rows, since the transform is 2D. |
| `love.graphics.applyTransform(t)` / `replaceTransform(t)` / `shear(kx, ky)` | `graphics::apply_transform(&t)` / `replace_transform(&t)` / `shear(kx, ky)` | `graphics::transform()` returns the current one. |
| `love.graphics.transformPoint(x, y)` / `inverseTransformPoint(x, y)` | `graphics::transform_point(x, y)` / `inverse_transform_point(x, y)` | In units, under the current transform. |
| `love.graphics.draw(image, transform)` / `print(text, transform)` | `graphics::draw_with(&image, &transform)` / `print_with(text, &transform)` | Also `draw_quad_with` and `printf_with`. |
| `love.graphics.newText(font, text)` | `graphics::new_text(&font, text)` | Laid out once, the first time it is drawn after a change. |
| `text:set(s)` / `setf(s, limit, align)` / `add(s, x, y)` / `addf(s, limit, align, x, y)` / `clear()` | `text.set(s)` / `set_f(s, limit, align)` / `add(s, x, y)` / `add_f(s, limit, align, x, y)` / `clear()` | `add` returns the index from 0. `add_ex` takes the full `draw` arguments. |
| `text:getDimensions()` / `getWidth()` / `getHeight()` / `setFont(font)` | `text.dimensions()` / `width()` / `height()` / `set_font(&font)` | `text.bounds(index)` gives one piece's box. |
| `love.graphics.draw(text, x, y)` | `graphics::draw_text(&text, x, y)` | `draw_text_ex` and `draw_text_with` take the other forms. |
| `love.graphics.setColorMask(r, g, b, a)` / `getColorMask()` | `graphics::set_color_mask(r, g, b, a)` / `graphics::color_mask()` | Booleans. Clearing ignores the mask. |
| `love.graphics.setWireframe(true)` / `isWireframe()` | `graphics::set_wireframe(true)` / `graphics::is_wireframe()` | Triangle edges, one pixel wide. |
| `love.graphics.getStats()` | `graphics::stats()` | A `Stats` struct with LÖVE's fields in `snake_case`. Draw counts start from zero each frame. |
| `love.graphics.newImage(path, {mipmaps = true})` | `graphics::new_image_with(path, ImageSettings { mipmaps: true })` | Also `Image::from_rgba_with` and `Image::from_image_data_with`. Hot reloading keeps the mipmaps. |
| `love.graphics.newCanvas(w, h, {mipmaps = "auto"})` / `canvas:generateMipmaps()` | `Canvas::with_mipmaps(w, h, MipmapMode::Auto)` / `canvas.generate_mipmaps()` | `Auto` makes them again each frame the canvas is drawn to; `Manual` only when asked. |
| `image:setMipmapFilter("linear", sharpness)` / `getMipmapFilter()` / `getMipmapCount()` | `image.set_mipmap_filter(Some(Filter::Linear), sharpness)` / `mipmap_filter()` / `mipmap_count()` | `None` turns them off. Starts out `Linear` with a sharpness of 0. |
| `love.graphics.newArrayImage({path1, path2})` | `graphics::new_array_image(&[path1, path2])` | `ArrayImage::from_image_data` takes `ImageData` layers. Every layer must be the same size. |
| `love.graphics.drawLayer(array, layer, x, y)` | `graphics::draw_layer(&array, layer, x, y)` | Layers count from 0. `draw_layer_ex` and `draw_layer_quad_with` take the other forms. Not with a custom shader active: send the array to the shader instead. |
| `love.graphics.newCubeImage({px, nx, py, ny, pz, nz})` | `graphics::new_cube_image(&[px, nx, py, ny, pz, nz])` | Six square faces, in LÖVE's order. |
| `love.graphics.newCubeImage(path)` | `graphics::new_cube_image_from_layout(path)` | One picture of all six faces: a 6 by 1 or 1 by 6 strip, or a 4 by 3 or 3 by 4 cross, as in LÖVE. `CubeImage::from_layout` takes an `ImageData`. |
| `shader:send("sky", cubeimage)` | `shader.send_texture("sky", &cube)` | The shader declares `var sky: texture_cube<f32>;` with no binding and samples it with `sky_sampler`. Also `texture_2d<f32>` for an `Image` or `Canvas`, and `texture_2d_array<f32>` for an `ArrayImage`. One not sent reads as white. |
| `love.image.newCompressedData(path)` / `love.graphics.newImage(compresseddata)` | `graphics::new_compressed_data(path)` / `Image::from_compressed_data(&data)` | DDS files in BC1 to BC5 and BC7 (DXT1, DXT3, DXT5 and the rest), with their mipmaps. `new_image` loads a `.dds` file too, and reloads it when it changes like any image. KTX, ASTC and ETC files are not supported yet. |
| `love.graphics.getImageFormats()` | `graphics::is_compressed_format_supported(CompressedFormat::Bc7)` | One format at a time. |
| `love.graphics.print({color1, "a", color2, "b"}, x, y)` / `printf(coloredtext, ...)` | `graphics::print_colored(&[(color1, "a"), (color2, "b")], x, y)` / `printf_colored(...)` | Each part is tinted by its color and the current one. `Text` has `add_colored`, `add_f_colored`, `set_colored` and `set_f_colored`. |
| `world:queryBoundingBox(x1, y1, x2, y2, callback)` | `world.fixtures_in_box(x1, y1, x2, y2)` | A list, by bounding box as in LÖVE. `fixture.bounding_box()` is `getBoundingBox`. |
| `world:setCallbacks(nil, nil, preSolve)` | `world.set_pre_solve(\|contact\| ...)` | A closure. Set `contact.enabled = false` for a one-way platform, or change its `friction` and `restitution`. It must not call into the world. |
| `world:setCallbacks(nil, nil, nil, postSolve)` / `world:getContacts()` | `world.touching()` after each update | Each `Touch` carries `normal_impulse` and `tangent_impulse`. |
| `joint:setMotorEnabled(true)` / `setMotorSpeed(s)` / `setMaxMotorTorque(t)` | `joint.set_motor_enabled(true)` / `set_motor_speed(s)` / `set_max_motor_torque(t)` | Revolute, prismatic (`set_max_motor_force`) and wheel joints. `motor_torque()` reads what it applied. |
| `joint:setLimitsEnabled(true)` / `setLimits(lo, hi)` | `joint.set_limits_enabled(true)` / `joint.set_limits(lo, hi)` | Revolute and prismatic joints, measured from where the joint was made. |
| `joint:getJointAngle()` / `getJointTranslation()` / `getJointSpeed()` | `joint.joint_angle()` / `joint_translation()` / `joint_speed()` | |
| `joint:getType()` / `getBodies()` / `getAnchors()` | `joint.kind()` / `joint.bodies()` / `joint.anchors()` | `kind()` is a `JointType`. Asking a joint for what its kind lacks panics. |
| `joint:getReactionForce(1 / dt)` / `getReactionTorque(1 / dt)` | `joint.reaction_force()` / `joint.reaction_torque()` | Already per second, for the last update. |
| `ropejoint:setMaxLength(l)` / `distancejoint:setLength(l)` | `joint.set_max_length(l)` / `joint.set_length(l)` | `set_length` also moves a spring joint's rest length; `set_stiffness` and `set_damping` tune it. |
| `love.physics.newMouseJoint(body, x, y)` / `setTarget(x, y)` | `world.new_mouse_joint(&body, x, y)` / `joint.set_target(x, y)` | `set_frequency`, `set_damping_ratio` and `set_max_force` as in LÖVE. |
| `wheeljoint:setSpringFrequency(f)` / `setSpringDampingRatio(r)` | `joint.set_frequency(f)` / `joint.set_damping_ratio(r)` | The same names as the mouse joint's. |
| `motorjoint:setLinearOffset(x, y)` / `setAngularOffset(a)` | `joint.set_linear_offset(x, y)` / `joint.set_angular_offset(a)` | Unlimited force and torque until `set_max_force` and `set_max_torque` say otherwise. |
| `world:getJoints()` / `body:getJoints()` | `world.joints()` / `body.joints()` | Also `world.joint_count()`. |
| `love.filesystem.openFile(name, mode)` / `newFile(name, mode)` | `filesystem::open_file(name, FileMode::Read)` | A `File` implements `Read`, `BufRead`, `Write` and `Seek`. `read_bytes`, `read_all`, `write_bytes`, `tell`, `size` and `is_eof` mirror LÖVE's methods, and `lines` comes from `BufRead`. Closed when dropped. `setBuffer` is left out: reads and writes are always buffered. |
| `love.filedropped(file, x, y)` / `love.directorydropped(path, x, y)` | `Game::file_dropped(path, x, y)` / `Game::directory_dropped(path, x, y)` | An absolute `&Path` instead of a `DroppedFile`. `filesystem` functions take it as it is, and `filesystem::mount` makes a dropped directory readable. |
| `love.event.push(name, ...)` / `love.handlers[name]` | `event::push(value)` / `Game::custom_event(event)` | Any type can be pushed, usually an enum of the game's own. `CustomEvent::downcast` gets it back. Delivered at the start of the next frame, after the platform's events. |
| `love.event.clear()` | `event::clear()` | Clears the game's own pushed events. |
| `love.lowmemory()` | `Game::low_memory()` | |
| `love.keyboard.isScancodeDown(sc)` / `love.keypressed(key, scancode, isrepeat)` | `keyboard::is_down(Key)` / `Game::key_pressed(key, is_repeat)` | A `Key` always names a position, by what a US layout has there, which is LÖVE's scancode. WASD stays put on an AZERTY keyboard. |
| `love.keyboard.getKeyFromScancode(sc)` / `getScancodeFromKey(key)` | `keyboard::layout_key(Key)` / `keyboard::layout_position(Key)` | `None` for a key Smitten has no name for. On AZERTY, `layout_key(Key::Q)` is `Some(Key::A)`. |
| `love.keyboard.isDown(key)` with layout keys | `keyboard::is_layout_down(Key)` | Follows the letter rather than the position, for shortcuts such as Ctrl+Z. |
| `love.keyboard.setKeyRepeat(enable)` / `hasKeyRepeat()` | `keyboard::set_key_repeat(enabled)` / `keyboard::has_key_repeat()` | Off by default, as in LÖVE. Text input repeats either way. |
| `love.keyboard.setTextInput(enable)` / `setTextInput(true, x, y, w, h)` / `hasTextInput()` / `hasScreenKeyboard()` | `keyboard::set_text_input(enabled)` / `keyboard::set_text_input_area(x, y, w, h)` / `keyboard::has_text_input()` / `keyboard::has_screen_keyboard()` | The area is in units, like `mouse::position()`. Text input starts on, as in LÖVE on desktop. |
| `love.textedited(text, start, length)` | `Game::text_edited(text, start, length)` | Text an input method is still composing. |
| `love.mouse.isDown(1, 2)` / `love.mouse.isCursorSupported()` | `mouse::is_any_down(&[MouseButton::Left, MouseButton::Right])` / `mouse::is_cursor_supported()` | |
| `joystick:getAxis(i)` / `getAxes()` / `getAxisCount()` | `joystick.axis(i)` / `joystick.axes()` / `joystick.axis_count()` | Raw inputs count from 0, where LÖVE counts from 1. Every joystick is listed now, not just gamepads. |
| `joystick:isDown(b, ...)` / `getButtonCount()` / `getHat(i)` / `getHatCount()` | `joystick.is_down(b)` / `joystick.is_any_down(&[b, ...])` / `joystick.button_count()` / `joystick.hat(i)` / `joystick.hat_count()` | Hats are a `JoystickHat` enum, such as `JoystickHat::LeftUp` for `"lu"`. |
| `joystick:isGamepad()` / `getGUID()` / `getDeviceInfo()` | `joystick.is_gamepad()` / `joystick.guid()` / `joystick.device_info()` | `device_info()` is `(vendor_id, product_id, product_version)`. |
| `love.joystickpressed(j, button)` / `joystickreleased` / `joystickaxis(j, axis, value)` / `joystickhat(j, hat, direction)` | `Game::joystick_pressed(joystick, button)` / `joystick_released` / `joystick_axis(joystick, axis, value)` / `joystick_hat(joystick, hat, direction)` | Sent for gamepads too, alongside the `gamepad_` callbacks. |
| `love.joystick.setGamepadMapping(guid, "a", "button", 3)` | `joystick::set_gamepad_mapping(guid, GamepadInput::Button(GamepadButton::A), JoystickInput::Button(2))` | A joystick with no mapping becomes a gamepad once it has one. `JoystickInput::Axis(i)` and `JoystickInput::Hat(i, JoystickHat::Up)` too. |
| `joystick:getGamepadMapping("a")` / `getGamepadMappingString()` | `joystick.gamepad_mapping(GamepadInput::Button(GamepadButton::A))` / `joystick.gamepad_mapping_string()` | `joystick::gamepad_mapping_string(guid)` works without a connected device. |
| *(none)* | `joystick::gamepads()` | The connected joysticks that are gamepads, for games that want player one's controller rather than whatever was plugged in first. |
| `love.joystick.loadGamepadMappings(file)` / `saveGamepadMappings()` | `joystick::load_gamepad_mappings(path)` / `joystick::load_gamepad_mappings_from_str(text)` / `joystick::save_gamepad_mappings()` | SDL's `gamecontrollerdb.txt` format. Saving returns the text rather than writing a file. |
| `love.window.getIcon()` | `window::icon()` | `Option<ImageData>`, `None` until the game sets one. |
| `love.window.getSafeArea()` | `window::safe_area()` | `(x, y, width, height)` in units. The whole window on desktop. |
| `love.window.getDisplayOrientation()` / `love.displayrotated(index, orientation)` | `window::display_orientation()` / `Game::display_rotated(orientation)` | For the display the window is on, so there is no index. Desktops report what the display's shape suggests. |
| `love.window.toPixels(value)` / `fromPixels(value)` | `window::to_pixels(value)` / `window::from_pixels(value)` | Multiply or divide by `window::dpi_scale()`. |
| `love.timer.getAverageDelta()` | `timer::average_delta()` | Over the last second, updated once a second. |
| `love.timer.sleep(seconds)` | `timer::sleep(seconds)` | |
| `love.system.getPreferredLocales()` | `system::preferred_locales()` | `Vec<String>` such as `["en_US", "fr"]`, most preferred first. |
| `love.system.vibrate(seconds)` | `system::vibrate(seconds)` | Android only; returns whether it vibrated. Gamepads use `Joystick::set_vibration`. |
| `love.timer.getDelta()` | `timer::delta()` | Getters drop the `get` prefix, as Rust style prefers. |
| `love.event.quit()` | `event::quit()` | |
| `love.errorhandler` | Built in | A panic shows the blue error screen with its message and location. It cannot be replaced yet. |

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
