//! The framework's global state.
//!
//! LÖVE lets a game call `love.graphics.draw` from anywhere. To give Rust the
//! same feel, the running framework lives in a thread-local slot that the
//! public modules reach into. It is only reachable on the thread that called
//! [`crate::run`], which keeps this safe without locks.

use std::cell::RefCell;
use std::collections::HashSet;

use crate::backend::Backend;
use crate::graphics::Color;
use crate::keyboard::Key;
use crate::timer::TimerState;

pub(crate) struct Context {
    pub backend: Box<dyn Backend>,
    pub timer: TimerState,
    pub keys_down: HashSet<Key>,
    pub background: Color,
    pub frame_clear: Color,
    pub quit_requested: bool,
}

thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

pub(crate) fn install(backend: Box<dyn Backend>) {
    CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(
            slot.is_none(),
            "smitten::run is already running on this thread"
        );
        *slot = Some(Context {
            backend,
            timer: TimerState::default(),
            keys_down: HashSet::new(),
            background: Color::BLACK,
            frame_clear: Color::BLACK,
            quit_requested: false,
        });
    });
}

pub(crate) fn uninstall() {
    CONTEXT.with(|slot| slot.borrow_mut().take());
}

/// Runs `f` with the framework state.
///
/// The borrow lasts only for the duration of `f`, and game callbacks are
/// never invoked from inside it, so framework calls made by a game cannot
/// overlap.
pub(crate) fn with<R>(f: impl FnOnce(&mut Context) -> R) -> R {
    CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let ctx = slot.as_mut().expect(
            "smitten modules can only be used on the main thread while smitten::run is active",
        );
        f(ctx)
    })
}
