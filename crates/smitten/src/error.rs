//! The error screen, the equivalent of LÖVE's blue screen.
//!
//! When a game panics, the window stays open and shows what went wrong and
//! where, instead of vanishing and leaving the message in a terminal that
//! may not be visible. Escape or closing the window ends the program, and
//! the panic then carries on as it would have.
//!
//! A game can show its own screen instead with [`set_error_handler`].

use std::any::Any;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;

use smitten_core::backend::BackendEvent;

use crate::graphics::Align;
use crate::{Color, Key, context};

/// The background LÖVE's error screen uses.
const BACKGROUND: Color = Color {
    r: 89.0 / 255.0,
    g: 157.0 / 255.0,
    b: 220.0 / 255.0,
    a: 1.0,
};

/// Space between the text and the window's edges, in units.
const MARGIN: f32 = 70.0;

/// The largest margin as a share of the window, so that a small window
/// still has room for the text.
const MARGIN_SHARE: f32 = 0.1;

/// Draws one frame of an error screen.
type DrawError = Box<dyn FnMut()>;

/// What a game asked to happen when it panics.
type Handler = Box<dyn FnOnce(&ErrorReport) -> Option<DrawError>>;

thread_local! {
    /// Where the most recent panic on this thread happened.
    static LOCATION: RefCell<Option<String>> = const { RefCell::new(None) };
    static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
}

/// What went wrong, as given to the handler set with
/// [`set_error_handler`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReport {
    /// The panic's message.
    pub message: String,
    /// Where the panic happened, as `file:line:column`, if known.
    pub location: Option<String>,
}

/// Replaces the error screen, the equivalent of `love.errorhandler`.
///
/// When the game panics, the game is dropped, music stops, and the
/// graphics state is reset. Then `handler` is called once with what went
/// wrong. It can log the error or save the player's progress, and then:
///
/// - return `None` to end the program straight away, or
/// - return `Some(draw)`, and `draw` is called every frame to draw the
///   screen, with the [`graphics`](crate::graphics) functions, until the
///   player presses Escape or closes the window.
///
/// If the handler or `draw` panics, the built-in screen is shown instead,
/// with the game's first error. Either way, the panic then carries on as
/// it would have. Setting a new handler replaces the last one.
///
/// ```no_run
/// use smitten::{graphics, Color};
///
/// smitten::set_error_handler(|report| {
///     eprintln!("crashed: {}", report.message);
///     let text = format!("Oops!\n\n{}", report.message);
///     Some(move || {
///         graphics::clear(Color::rgb(0.2, 0.0, 0.0));
///         graphics::print(&text, 40.0, 40.0);
///     })
/// });
/// ```
pub fn set_error_handler<D>(handler: impl FnOnce(&ErrorReport) -> Option<D> + 'static)
where
    D: FnMut() + 'static,
{
    let handler: Handler =
        Box::new(move |report| handler(report).map(|draw| Box::new(draw) as DrawError));
    HANDLER.with(|slot| *slot.borrow_mut() = Some(handler));
}

/// Goes back to the built-in error screen.
pub fn reset_error_handler() {
    HANDLER.with(|slot| *slot.borrow_mut() = None);
}

/// Starts recording where panics happen. The message is still printed to
/// the terminal as usual.
fn record_locations() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let location = info.location().map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            });
            // The slot is gone if the thread is already shutting down.
            let _ = LOCATION.try_with(|slot| *slot.borrow_mut() = location);
            previous(info);
        }));
    });
}

/// Runs the game, catching a panic so that it can be shown.
pub(crate) fn catch(game: impl FnOnce()) -> Result<(), Box<dyn Any + Send>> {
    record_locations();
    // Nothing of the game's is used after a panic: it is dropped as the
    // panic unwinds, and the framework state is reset before it is drawn
    // with again.
    catch_unwind(AssertUnwindSafe(game))
}

/// The text of a panic. `panic!` with a message gives a `String` or a
/// `&str`; anything else has no text to show.
fn message(payload: &(dyn Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<String>() {
        text
    } else if let Some(text) = payload.downcast_ref::<&str>() {
        text
    } else {
        "the game panicked without a message"
    }
}

/// What the error screen says.
fn report(message: &str, location: Option<&str>) -> String {
    let mut text = format!("Error\n\n{message}\n");
    if let Some(location) = location {
        text.push_str(&format!("\nat {location}\n"));
    }
    text.push_str("\nPress Escape to quit");
    text
}

/// The built-in screen: `text` in the corner.
fn built_in(text: String) -> DrawError {
    Box::new(move || {
        context::with(|ctx| {
            let (width, height) = ctx.gfx.screen.size();
            let margin = MARGIN.min(width.min(height) as f32 * MARGIN_SHARE);
            ctx.gfx.print_plain(
                ctx.backend.as_mut(),
                &text,
                margin,
                margin,
                Some((width as f32 - margin * 2.0, Align::Left)),
            );
        });
    })
}

/// Clears away whatever the game or a failed handler left behind.
fn reset_graphics() {
    // Whatever was left behind is freed once the framework state is no
    // longer borrowed, because freeing it needs that state.
    let leftovers = context::with(|ctx| ctx.gfx.reset_for_error_screen(BACKGROUND));
    drop(leftovers);
}

/// Shows the panic in the window until the player presses Escape or
/// closes it. Returns straight away if the screen itself cannot be drawn,
/// or if the game's error handler asks to quit.
pub(crate) fn show(payload: &(dyn Any + Send)) {
    let report = ErrorReport {
        message: message(payload).to_owned(),
        location: LOCATION.with(|slot| slot.borrow_mut().take()),
    };
    let text = self::report(&report.message, report.location.as_deref());
    let handler = HANDLER.with(|slot| slot.borrow_mut().take());

    let shown = catch_unwind(AssertUnwindSafe(|| {
        // Music should not play on over the error.
        crate::audio::stop();
        // A window the game closed is opened again to show the error.
        crate::window::open();
        reset_graphics();

        let mut draw = match handler {
            None => built_in(text.clone()),
            Some(handler) => match catch_unwind(AssertUnwindSafe(|| handler(&report))) {
                Ok(None) => return,
                Ok(Some(draw)) => draw,
                Err(_) => {
                    reset_graphics();
                    built_in(text.clone())
                }
            },
        };

        loop {
            while let Some(event) = context::with(|ctx| ctx.backend.poll_event()) {
                match event {
                    BackendEvent::Quit
                    | BackendEvent::KeyDown {
                        key: Key::Escape, ..
                    } => return,
                    _ => {}
                }
            }
            context::with(|ctx| ctx.gfx.begin_frame(&mut *ctx.backend));
            let drawn = catch_unwind(AssertUnwindSafe(&mut draw));
            context::with(|ctx| {
                ctx.gfx.finish_frame();
                ctx.backend.present(&ctx.gfx.frame());
                ctx.gfx.end_frame(&mut *ctx.backend);
            });
            if drawn.is_err() {
                reset_graphics();
                draw = built_in(text.clone());
            }
            // Nothing moves on the built-in screen, so there is no need
            // to draw as fast as a display without vsync would allow.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }));
    if shown.is_err() {
        eprintln!("smitten: the error screen could not be drawn");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_of_a_panic_is_found_whichever_way_it_was_raised() {
        let formatted = catch_unwind(|| panic!("lives: {}", 3)).unwrap_err();
        assert_eq!(message(formatted.as_ref()), "lives: 3");

        let literal = catch_unwind(|| panic!("out of hearts")).unwrap_err();
        assert_eq!(message(literal.as_ref()), "out of hearts");

        let other = catch_unwind(|| std::panic::panic_any(42)).unwrap_err();
        assert_eq!(
            message(other.as_ref()),
            "the game panicked without a message"
        );
    }

    #[test]
    fn the_report_says_what_went_wrong_and_where() {
        let text = report("could not read heart.png", Some("src/main.rs:12:5"));
        assert!(text.starts_with("Error\n\ncould not read heart.png\n"));
        assert!(text.contains("\nat src/main.rs:12:5\n"));
        assert!(text.ends_with("Press Escape to quit"));

        assert!(!report("oops", None).contains("\nat "));
    }

    #[test]
    fn a_caught_panic_remembers_where_it_happened() {
        let line = line!() + 1;
        let caught = catch(|| panic!("boom"));
        assert!(caught.is_err());
        let location = LOCATION.with(|slot| slot.borrow_mut().take()).unwrap();
        assert!(location.contains("error.rs"), "{location}");
        assert!(location.contains(&format!(":{line}:")), "{location}");
    }

    #[test]
    fn a_new_error_handler_replaces_the_last() {
        set_error_handler(|_| None::<fn()>);
        set_error_handler(|_| Some(|| {}));
        let handler = HANDLER.with(|slot| slot.borrow_mut().take()).unwrap();
        let report = ErrorReport {
            message: "boom".to_owned(),
            location: None,
        };
        assert!(handler(&report).is_some());

        set_error_handler(|_| Some(|| {}));
        reset_error_handler();
        assert!(HANDLER.with(|slot| slot.borrow().is_none()));
    }

    #[test]
    fn a_game_that_does_not_panic_is_left_alone() {
        assert!(catch(|| {}).is_ok());
    }
}
