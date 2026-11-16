//! Smitten is a LÖVE-style 2D game framework for Rust.
//!
//! A game implements [`Game`] and hands it to [`run`]. While the game is
//! running, the framework modules ([`graphics`], [`keyboard`], [`timer`],
//! [`window`], [`event`]) can be called from anywhere on the main thread,
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

pub mod event;
pub mod graphics;
pub mod keyboard;
pub mod timer;
pub mod window;

pub use smitten_core::{Color, Config, Filter, Key};

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

    /// Called when the window's drawable size changes, in pixels.
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
    context::install(Box::new(backend));

    let mut game = G::load();
    context::with(|ctx| ctx.timer.reset(ctx.backend.ticks_ns()));

    'running: loop {
        while let Some(ev) = context::with(|ctx| ctx.backend.poll_event()) {
            match ev {
                BackendEvent::Quit => event::quit(),
                BackendEvent::Resized { width, height } => game.resized(width, height),
                BackendEvent::KeyDown { key, is_repeat } => {
                    context::with(|ctx| ctx.keys_down.insert(key));
                    game.key_pressed(key, is_repeat);
                }
                BackendEvent::KeyUp { key } => {
                    context::with(|ctx| ctx.keys_down.remove(&key));
                    game.key_released(key);
                }
            }
        }

        if context::with(|ctx| std::mem::take(&mut ctx.quit_requested)) && !game.quit() {
            break 'running;
        }

        let dt = context::with(|ctx| {
            let now = ctx.backend.ticks_ns();
            ctx.timer.step(now)
        });
        game.update(dt);

        context::with(|ctx| ctx.gfx.begin_frame());
        game.draw();
        context::with(|ctx| ctx.backend.present(&ctx.gfx.frame()));
    }

    drop(game);
    context::uninstall();
}
