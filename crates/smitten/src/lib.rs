//! Smitten is a LÖVE-style 2D game framework for Rust.
//!
//! A game implements [`Game`] and hands it to [`run`]. While the game is
//! running, the framework modules ([`graphics`], [`audio`], [`keyboard`], [`mouse`],
//! [`joystick`], [`filesystem`], [`timer`], [`window`], [`event`], [`math`]) can be called from anywhere on the main thread,
//! the same way `love.graphics` and friends work in LÖVE.
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
mod input;
mod reload;

pub mod audio;
pub mod event;
pub mod filesystem;
pub mod graphics;
pub mod joystick;
pub mod keyboard;
pub mod math;
pub mod mouse;
#[cfg(feature = "physics")]
pub mod physics;
pub mod timer;
pub mod window;

pub use joystick::Joystick;
pub use smitten_core::{
    BlendMode, Color, CompareMode, Config, Filter, GamepadAxis, GamepadButton, Key, MouseButton,
    Scaling, StencilAction,
};

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

    /// Called when a gamepad is plugged in, and once at startup for each
    /// gamepad that was already connected.
    fn joystick_added(&mut self, _joystick: Joystick) {}

    /// Called when a gamepad is unplugged.
    fn joystick_removed(&mut self, _joystick: Joystick) {}

    /// Called when a gamepad button goes down.
    fn gamepad_pressed(&mut self, _joystick: Joystick, _button: GamepadButton) {}

    /// Called when a gamepad button is released.
    fn gamepad_released(&mut self, _joystick: Joystick, _button: GamepadButton) {}

    /// Called when a gamepad stick or trigger moves.
    fn gamepad_axis(&mut self, _joystick: Joystick, _axis: GamepadAxis, _value: f32) {}

    /// Called when the window's size changes, with its new size in units
    /// (see [`window::dimensions`]).
    fn resized(&mut self, _width: u32, _height: u32) {}

    /// Called when the game is asked to close. Return `true` to keep running.
    fn quit(&mut self) -> bool {
        false
    }
}

/// Opens the window and runs the game until it quits.
///
/// Must be called from the main thread. Panics if the platform cannot be
/// initialized, because there is nothing a game can do without a window.
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

    let mut game = G::load();
    context::with(|ctx| ctx.timer.reset(ctx.backend.ticks_ns()));

    'running: loop {
        while let Some(event) = context::with(|ctx| ctx.backend.poll_event()) {
            dispatch(&mut game, event);
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
        context::with(|ctx| {
            ctx.gfx.finish_frame();
            ctx.backend.present(&ctx.gfx.frame());
            ctx.gfx.end_frame();
        });
    }

    drop(game);
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
        BackendEvent::KeyDown { key, is_repeat } => game.key_pressed(key, is_repeat),
        BackendEvent::KeyUp { key } => game.key_released(key),
        BackendEvent::TextInput(text) => game.text_input(&text),
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
    }
}
