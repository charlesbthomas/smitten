//! Starting and stopping the game, and the pieces of a frame for a game
//! that writes its own main loop.

use std::marker::PhantomData;

use crate::event::{self, Event};
use crate::{Color, Config, context, error, graphics, reload, timer, window};

/// Opens the window, runs `body`, and closes everything again. A panic in
/// `body` is shown on the error screen and then carried on.
pub(crate) fn run_with(config: Config, body: impl FnOnce(&mut MainLoop)) {
    if config.console {
        smitten_sdl::open_console();
    }
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
        if config.msaa > 1 {
            ctx.backend.set_screen_msaa(config.msaa);
        }
    });
    if let Some(icon) = &config.icon
        && let Err(error) = window::set_icon(icon)
    {
        panic!("smitten: could not load the window icon: {error}");
    }

    let outcome = error::catch(|| body(&mut MainLoop::new()));

    // A panic is shown in the window on the error screen, and then
    // carries on to end the program the way it would have.
    if let Err(panic) = outcome {
        error::show(panic.as_ref());
        context::uninstall();
        std::panic::resume_unwind(panic);
    }
    context::uninstall();
}

/// Opens the window and hands control to `main`, which runs the game's own
/// main loop instead of the one [`crate::run`] provides. The game ends when
/// `main` returns.
///
/// [`Game`](crate::Game) and [`run`](crate::run) are the usual way in. A
/// custom loop is for a game that needs to own the frame: a fixed update
/// rate, several updates per frame, or drawing only when something has
/// changed. [`MainLoop`] has the steps `run` takes each frame, so a loop
/// can keep the ones it needs and change the rest.
///
/// Must be called from the main thread. As with `run`, a panic is shown
/// on the error screen until the player presses Escape, and then carries
/// on.
///
/// ```no_run
/// use smitten::event::Event;
/// use smitten::{graphics, Color, Config, Key};
///
/// smitten::run_loop(Config::default(), |main| {
///     let mut x = 0.0;
///     // Leave out the time spent loading from the first step.
///     main.step();
///     // A fixed update rate of 120 a second, drawn as fast as vsync allows.
///     let tick = 1.0 / 120.0;
///     let mut owed = 0.0;
///     'running: loop {
///         for event in main.poll() {
///             match event {
///                 Event::Quit | Event::KeyPressed { key: Key::Escape, .. } => break 'running,
///                 _ => {}
///             }
///         }
///         owed += main.step();
///         while owed >= tick {
///             x += 60.0 * tick;
///             owed -= tick;
///         }
///         if main.is_active() {
///             main.clear(Color::rgb(0.1, 0.1, 0.2));
///             graphics::circle(graphics::DrawMode::Fill, (x % 800.0, 300.0), 20.0);
///             main.present();
///         }
///     }
/// });
/// ```
pub fn run_loop(config: Config, main: impl FnOnce(&mut MainLoop)) {
    run_with(config, |main_loop| {
        context::with(|ctx| {
            ctx.timer.reset(ctx.backend.ticks_ns());
            ctx.gfx.begin_frame(&mut *ctx.backend);
        });
        main(main_loop);
    });
}

/// The steps of one frame, handed to the closure given to [`run_loop`].
///
/// Each one is also a free function: [`event::pump`] and [`event::poll`],
/// [`timer::step`], [`graphics::clear`], [`graphics::present`] and
/// [`graphics::is_active`]. These methods add what `run` does around them,
/// such as noticing a call to [`event::quit`] and reloading changed files,
/// so prefer them in a loop.
///
/// Drawing can happen at any point between two presents. The screen starts
/// each frame cleared to [`graphics::background_color`].
#[derive(Debug)]
pub struct MainLoop {
    /// Only the main thread may use it, like the rest of the context.
    _not_send: PhantomData<*const ()>,
}

impl MainLoop {
    fn new() -> Self {
        Self {
            _not_send: PhantomData,
        }
    }

    /// Takes everything that has happened since the last call: the
    /// platform's events, then those sent with [`event::push`], as
    /// [`event::pump`] followed by [`event::poll`] would. The keyboard,
    /// mouse and joystick states are up to date when it returns.
    ///
    /// When the game called [`event::quit`], [`Event::Quit`] comes last.
    /// Nothing closes by itself in a custom loop: return from the closure
    /// on [`Event::Quit`] to end the game.
    pub fn poll(&mut self) -> Vec<Event> {
        event::pump();
        let mut events: Vec<Event> = event::poll().collect();
        if context::with(|ctx| std::mem::take(&mut ctx.quit_requested)) {
            events.push(Event::Quit);
        }
        events
    }

    /// Measures the seconds since the last step and returns them, as
    /// [`timer::step`] does. Call it once a frame. It also reloads any
    /// images, shaders, fonts and sounds whose files changed, when
    /// [`Config::hot_reload`] is on.
    ///
    /// The clock starts when the window opens, so call this once after
    /// loading to keep the loading time out of the first frame.
    pub fn step(&mut self) -> f32 {
        let dt = timer::step();
        reload::check();
        dt
    }

    /// Resets the transform and fills the screen with `color`, as
    /// [`crate::run`] does at the start of each frame.
    pub fn clear(&mut self, color: Color) {
        graphics::origin();
        graphics::clear(color);
    }

    /// Shows what was drawn since the last present, and starts the next
    /// frame. See [`graphics::present`].
    pub fn present(&mut self) {
        graphics::present();
    }

    /// Whether drawing reaches the screen. See [`graphics::is_active`].
    pub fn is_active(&self) -> bool {
        graphics::is_active()
    }
}
