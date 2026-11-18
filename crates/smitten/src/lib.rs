//! Smitten is a LÖVE-style 2D game framework for Rust.
//!
//! A game implements [`Game`] and hands it to [`run`]. While the game is
//! running, the framework modules ([`graphics`], [`audio`], [`keyboard`], [`mouse`], [`touch`],
//! [`joystick`], [`filesystem`], [`timer`], [`window`], [`event`], [`math`], [`sound`], [`system`]) can be called from anywhere on the main thread,
//! the same way `love.graphics` and friends work in LÖVE. [`data`], [`math`] and [`thread`]
//! also work on other threads and when no game is running.
//!
//! ```no_run
//! use smitten::{graphics, Color, Config, Game};
//!
//! struct Hello;
//!
//! impl Game for Hello {
//!     fn load() -> Self {
//!         Hello
//!     }
//!
//!     fn draw(&mut self) {
//!         graphics::clear(Color::rgb(0.1, 0.1, 0.2));
//!     }
//! }
//!
//! fn main() {
//!     smitten::run::<Hello>(Config::default());
//! }
//! ```

mod context;
mod error;
mod gamepad_mapping;
mod input;
mod reload;

pub mod audio;
pub mod data;
pub mod event;
pub mod filesystem;
pub mod font;
pub mod graphics;
pub mod joystick;
pub mod keyboard;
pub mod math;
pub mod mouse;
#[cfg(feature = "physics")]
pub mod physics;
pub mod sound;
pub mod system;
pub mod thread;
pub mod timer;
pub mod touch;
#[cfg(feature = "video")]
pub mod video;
pub mod window;

pub use error::{ErrorReport, reset_error_handler, set_error_handler};
pub use joystick::Joystick;
pub use smitten_core::{
    BlendAlphaMode, BlendMode, Color, CompareMode, Config, Cursor, DisplayOrientation, Filter,
    FullscreenType, GamepadAxis, GamepadButton, GamepadInput, JoystickHat, JoystickInput, Key,
    MessageBoxKind, MouseButton, Scaling, StencilAction, TouchId, Wrap,
};

use std::path::Path;

/// Smitten's version, such as `"0.1.0"`, the equivalent of the string
/// `love.getVersion` builds.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Smitten's major, minor and patch version numbers, the equivalent of
/// `love.getVersion`.
pub fn version() -> (u32, u32, u32) {
    let mut parts = VERSION
        .split(['.', '-', '+'])
        .map(|part| part.parse().unwrap_or(0));
    let mut next = || parts.next().unwrap_or(0);
    (next(), next(), next())
}

use smitten_core::backend::BackendEvent;

/// The callbacks a game provides. Only [`Game::load`] is required.
///
/// Each method mirrors the LÖVE callback of the same name.
pub trait Game: Sized + 'static {
    /// Called once after the window exists. Build the game's starting state.
    fn load() -> Self;

    /// Called every frame with the seconds elapsed since the last frame.
    fn update(&mut self, _dt: f32) {}

    /// Called every frame after `update` to draw the frame.
    fn draw(&mut self) {}

    /// Called when a key goes down, and again for each key repeat.
    fn key_pressed(&mut self, _key: Key, _is_repeat: bool) {}

    /// Called when a key is released.
    fn key_released(&mut self, _key: Key) {}

    /// Called with text the user typed, after the keyboard layout and any
    /// input method have been applied. Use this for text fields, and
    /// `key_pressed` for controls.
    fn text_input(&mut self, _text: &str) {}

    /// Called while an input method is composing text that has not been
    /// typed yet, as for Chinese or Japanese. `start` and `length` select
    /// the part being edited, counted in characters. A game showing a text
    /// box can draw `text` at the cursor until [`Game::text_input`] brings
    /// the finished text.
    fn text_edited(&mut self, _text: &str, _start: usize, _length: usize) {}

    /// Called when the pointer moves to `(x, y)`, having moved `(dx, dy)`.
    fn mouse_moved(&mut self, _x: f32, _y: f32, _dx: f32, _dy: f32) {}

    /// Called when a mouse button goes down. `presses` is 2 for a double
    /// click.
    fn mouse_pressed(&mut self, _x: f32, _y: f32, _button: MouseButton, _presses: u32) {}

    /// Called when a mouse button is released.
    fn mouse_released(&mut self, _x: f32, _y: f32, _button: MouseButton, _presses: u32) {}

    /// Called when the mouse wheel moves. Positive `dy` is away from the
    /// user, which usually means scroll up.
    fn wheel_moved(&mut self, _dx: f32, _dy: f32) {}

    /// Called when a finger touches the screen at `(x, y)`. `pressure`
    /// runs from 0.0 to 1.0, and is 1.0 on screens that cannot measure it.
    ///
    /// The platform also reports the first finger as a mouse, so a game
    /// that only needs taps can use `mouse_pressed` alone.
    fn touch_pressed(
        &mut self,
        _id: TouchId,
        _x: f32,
        _y: f32,
        _dx: f32,
        _dy: f32,
        _pressure: f32,
    ) {
    }

    /// Called when a finger on the screen moves to `(x, y)`, having moved
    /// `(dx, dy)`.
    fn touch_moved(&mut self, _id: TouchId, _x: f32, _y: f32, _dx: f32, _dy: f32, _pressure: f32) {}

    /// Called when a finger leaves the screen. Its id is not reported
    /// again.
    fn touch_released(
        &mut self,
        _id: TouchId,
        _x: f32,
        _y: f32,
        _dx: f32,
        _dy: f32,
        _pressure: f32,
    ) {
    }

    /// Called when a joystick or gamepad is plugged in, and once at
    /// startup for each one that was already connected.
    fn joystick_added(&mut self, _joystick: Joystick) {}

    /// Called when a joystick or gamepad is unplugged.
    fn joystick_removed(&mut self, _joystick: Joystick) {}

    /// Called when a gamepad button goes down.
    fn gamepad_pressed(&mut self, _joystick: Joystick, _button: GamepadButton) {}

    /// Called when a gamepad button is released.
    fn gamepad_released(&mut self, _joystick: Joystick, _button: GamepadButton) {}

    /// Called when a gamepad stick or trigger moves.
    fn gamepad_axis(&mut self, _joystick: Joystick, _axis: GamepadAxis, _value: f32) {}

    /// Called when a raw joystick button goes down, gamepad or not.
    /// Buttons count from 0.
    fn joystick_pressed(&mut self, _joystick: Joystick, _button: usize) {}

    /// Called when a raw joystick button is released.
    fn joystick_released(&mut self, _joystick: Joystick, _button: usize) {}

    /// Called when a raw joystick axis moves, with its new value from
    /// -1.0 to 1.0.
    fn joystick_axis(&mut self, _joystick: Joystick, _axis: usize, _value: f32) {}

    /// Called when a joystick hat changes direction.
    fn joystick_hat(&mut self, _joystick: Joystick, _hat: usize, _direction: JoystickHat) {}

    /// Called when the window's size changes, with its new size in units
    /// (see [`window::dimensions`]).
    fn resized(&mut self, _width: u32, _height: u32) {}

    /// Called when the window starts or stops receiving the keyboard,
    /// for instance when the player switches to another program. A game
    /// might pause here.
    fn focus(&mut self, _focused: bool) {}

    /// Called when the pointer enters or leaves the window.
    fn mouse_focus(&mut self, _focused: bool) {}

    /// Called when the window is minimized or hidden, and when it comes
    /// back.
    fn visible(&mut self, _visible: bool) {}

    /// Called when the player drops a file on the window, with its
    /// absolute path and where it was dropped, the equivalent of
    /// `love.filedropped`. Several files dropped together arrive one call
    /// each.
    ///
    /// [`filesystem::read`] and friends take the absolute path as it is.
    /// The file is not copied anywhere; save what the game needs to keep
    /// with [`filesystem::write`].
    ///
    /// Not every platform reports where a drop happened. Where it is not
    /// known, `(x, y)` is `(0, 0)`.
    fn file_dropped(&mut self, _path: &Path, _x: f32, _y: f32) {}

    /// Called when the player drops a directory on the window, with its
    /// absolute path and where it was dropped, the equivalent of
    /// `love.directorydropped`.
    ///
    /// To read the files inside it like the game's own, mount it:
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// # use smitten::filesystem;
    /// # fn directory_dropped(path: &Path) {
    /// filesystem::mount(path, "dropped").expect("the directory can be mounted");
    /// let names = filesystem::directory_items("dropped");
    /// # }
    /// ```
    fn directory_dropped(&mut self, _path: &Path, _x: f32, _y: f32) {}

    /// Called with each event the game sent itself with [`event::push`],
    /// the equivalent of a handler in `love.handlers`. They arrive at the
    /// start of a frame, after the platform's events and before
    /// `update`.
    fn custom_event(&mut self, _event: event::CustomEvent) {}

    /// Called when the system is running out of memory, the equivalent of
    /// `love.lowmemory`. Mobile systems send this before closing a game
    /// to reclaim memory, so drop caches and anything that can be loaded
    /// again.
    fn low_memory(&mut self) {}

    /// Called when the display the window is on turns, as when a phone
    /// is rotated, the equivalent of `love.displayrotated`. The safe area
    /// (see [`window::safe_area`]) may have moved too.
    fn display_rotated(&mut self, _orientation: DisplayOrientation) {}

    /// Called when the game is asked to close. Return `true` to keep running.
    fn quit(&mut self) -> bool {
        false
    }
}

/// Opens the window and runs the game until it quits.
///
/// Must be called from the main thread. Panics if the platform cannot be
/// initialized, because there is nothing a game can do without a window.
///
/// If the game panics, the window stays open and shows the message and
/// where it came from until the player presses Escape, like LÖVE's error
/// screen. The panic then continues as usual.
pub fn run<G: Game>(config: Config) {
    let backend = match smitten_sdl::SdlBackend::new(&config) {
        Ok(backend) => backend,
        Err(message) => panic!("smitten: could not start: {message}"),
    };
    context::install(Box::new(backend), &config);
    context::with(|ctx| {
        // Fonts and canvases made in `load` need to know how sharp the
        // screen is.
        ctx.gfx.screen = graphics::Screen::new(&config);
        ctx.gfx
            .screen
            .update(ctx.backend.size_in_pixels(), ctx.backend.dpi_scale());
    });
    if let Some(icon) = &config.icon {
        window::set_icon(icon);
    }

    let outcome = error::catch(|| {
        let mut game = G::load();
        context::with(|ctx| ctx.timer.reset(ctx.backend.ticks_ns()));

        'running: loop {
            while let Some(event) = context::with(|ctx| ctx.backend.poll_event()) {
                dispatch(&mut game, event);
            }
            for event in context::with(|ctx| ctx.events.take()) {
                game.custom_event(event);
            }

            if context::with(|ctx| std::mem::take(&mut ctx.quit_requested)) && !game.quit() {
                break 'running;
            }

            let dt = context::with(|ctx| {
                let now = ctx.backend.ticks_ns();
                ctx.timer.step(now)
            });
            reload::check();
            game.update(dt);

            context::with(|ctx| ctx.gfx.begin_frame(&mut *ctx.backend));
            game.draw();
            let screenshots = context::with(|ctx| {
                ctx.gfx.finish_frame();
                let screenshots = ctx.gfx.take_screenshots(&mut *ctx.backend);
                if ctx.window_closed {
                    ctx.backend.render_offscreen(&ctx.gfx.frame());
                } else {
                    ctx.backend.present(&ctx.gfx.frame());
                }
                ctx.gfx.end_frame();
                screenshots
            });
            for (callback, picture) in screenshots {
                callback(picture);
            }
        }
    });

    // A panic is shown in the window, as LÖVE shows its error screen, and
    // then carries on to end the program the way it would have.
    if let Err(panic) = outcome {
        error::show(panic.as_ref());
        context::uninstall();
        std::panic::resume_unwind(panic);
    }
    context::uninstall();
}

/// Turns the pixels in a backend event into the units the game works in.
fn localize(ctx: &mut context::Context, event: BackendEvent) -> BackendEvent {
    if let BackendEvent::Resized { width, height } = event {
        let scale = ctx.backend.dpi_scale();
        ctx.gfx.screen.update((width, height), scale);
        let (width, height) = ctx.gfx.screen.window_size();
        return BackendEvent::Resized { width, height };
    }
    let view = ctx.gfx.screen.view();
    match event {
        BackendEvent::MouseMoved { x, y, dx, dy } => {
            let (x, y) = view.to_units(x, y);
            BackendEvent::MouseMoved {
                x,
                y,
                dx: dx / view.scale,
                dy: dy / view.scale,
            }
        }
        BackendEvent::MousePressed {
            x,
            y,
            button,
            presses,
        } => {
            let (x, y) = view.to_units(x, y);
            BackendEvent::MousePressed {
                x,
                y,
                button,
                presses,
            }
        }
        BackendEvent::MouseReleased {
            x,
            y,
            button,
            presses,
        } => {
            let (x, y) = view.to_units(x, y);
            BackendEvent::MouseReleased {
                x,
                y,
                button,
                presses,
            }
        }
        BackendEvent::TouchPressed {
            id,
            x,
            y,
            dx,
            dy,
            pressure,
        } => {
            let (x, y) = view.to_units(x, y);
            BackendEvent::TouchPressed {
                id,
                x,
                y,
                dx: dx / view.scale,
                dy: dy / view.scale,
                pressure,
            }
        }
        BackendEvent::TouchMoved {
            id,
            x,
            y,
            dx,
            dy,
            pressure,
        } => {
            let (x, y) = view.to_units(x, y);
            BackendEvent::TouchMoved {
                id,
                x,
                y,
                dx: dx / view.scale,
                dy: dy / view.scale,
                pressure,
            }
        }
        BackendEvent::TouchReleased {
            id,
            x,
            y,
            dx,
            dy,
            pressure,
        } => {
            let (x, y) = view.to_units(x, y);
            BackendEvent::TouchReleased {
                id,
                x,
                y,
                dx: dx / view.scale,
                dy: dy / view.scale,
                pressure,
            }
        }
        BackendEvent::FileDropped { path, x, y } => {
            let (x, y) = view.to_units(x, y);
            BackendEvent::FileDropped { path, x, y }
        }
        BackendEvent::DirectoryDropped { path, x, y } => {
            let (x, y) = view.to_units(x, y);
            BackendEvent::DirectoryDropped { path, x, y }
        }
        other => other,
    }
}

/// Records an event in the input state, then tells the game about it.
fn dispatch<G: Game>(game: &mut G, event: BackendEvent) {
    let event = context::with(|ctx| {
        let event = localize(ctx, event);
        ctx.input.apply(&event);
        event
    });

    match event {
        BackendEvent::Quit => event::quit(),
        BackendEvent::Resized { width, height } => game.resized(width, height),
        BackendEvent::KeyDown { key, is_repeat } => {
            if !is_repeat || context::with(|ctx| ctx.input.key_repeat) {
                game.key_pressed(key, is_repeat);
            }
        }
        BackendEvent::KeyUp { key } => game.key_released(key),
        BackendEvent::TextInput(text) => game.text_input(&text),
        BackendEvent::TextEdited {
            text,
            start,
            length,
        } => game.text_edited(&text, start, length),
        BackendEvent::MouseMoved { x, y, dx, dy } => game.mouse_moved(x, y, dx, dy),
        BackendEvent::MousePressed {
            x,
            y,
            button,
            presses,
        } => game.mouse_pressed(x, y, button, presses),
        BackendEvent::MouseReleased {
            x,
            y,
            button,
            presses,
        } => game.mouse_released(x, y, button, presses),
        BackendEvent::WheelMoved { dx, dy } => game.wheel_moved(dx, dy),
        BackendEvent::TouchPressed {
            id,
            x,
            y,
            dx,
            dy,
            pressure,
        } => game.touch_pressed(id, x, y, dx, dy, pressure),
        BackendEvent::TouchMoved {
            id,
            x,
            y,
            dx,
            dy,
            pressure,
        } => game.touch_moved(id, x, y, dx, dy, pressure),
        BackendEvent::TouchReleased {
            id,
            x,
            y,
            dx,
            dy,
            pressure,
        } => game.touch_released(id, x, y, dx, dy, pressure),
        BackendEvent::JoystickAdded { id, .. } => game.joystick_added(Joystick::new(id)),
        BackendEvent::JoystickRemoved { id } => game.joystick_removed(Joystick::new(id)),
        BackendEvent::GamepadPressed { id, button } => {
            game.gamepad_pressed(Joystick::new(id), button);
        }
        BackendEvent::GamepadReleased { id, button } => {
            game.gamepad_released(Joystick::new(id), button);
        }
        BackendEvent::GamepadAxis { id, axis, value } => {
            game.gamepad_axis(Joystick::new(id), axis, value);
        }
        BackendEvent::JoystickPressed { id, button } => {
            game.joystick_pressed(Joystick::new(id), button);
        }
        BackendEvent::JoystickReleased { id, button } => {
            game.joystick_released(Joystick::new(id), button);
        }
        BackendEvent::JoystickAxis { id, axis, value } => {
            game.joystick_axis(Joystick::new(id), axis, value);
        }
        BackendEvent::JoystickHat { id, hat, direction } => {
            game.joystick_hat(Joystick::new(id), hat, direction);
        }
        BackendEvent::Focus(focused) => game.focus(focused),
        BackendEvent::MouseFocus(focused) => game.mouse_focus(focused),
        BackendEvent::Visible(visible) => game.visible(visible),
        BackendEvent::FileDropped { path, x, y } => game.file_dropped(&path, x, y),
        BackendEvent::DirectoryDropped { path, x, y } => game.directory_dropped(&path, x, y),
        BackendEvent::LowMemory => game.low_memory(),
        BackendEvent::DisplayRotated(orientation) => game.display_rotated(orientation),
    }
}

#[cfg(test)]
mod version_tests {
    #[test]
    fn the_version_numbers_match_the_crate() {
        let (major, minor, patch) = super::version();
        assert_eq!(format!("{major}.{minor}.{patch}"), super::VERSION);
    }
}
