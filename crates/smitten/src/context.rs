//! The framework's global state.
//!
//! LÖVE lets a game call `love.graphics.draw` from anywhere. To give Rust the
//! same feel, the running framework lives in a thread-local slot that the
//! public modules reach into. It is only reachable on the thread that called
//! [`crate::run`], which keeps this safe without locks.

use std::cell::RefCell;

use smitten_core::Config;
use smitten_core::backend::Backend;

use crate::audio::AudioState;
use crate::filesystem::FilesystemState;
use crate::graphics::GraphicsState;
use crate::input::InputState;
use crate::reload::ReloadState;
use crate::timer::TimerState;

pub(crate) struct Context {
    pub backend: Box<dyn Backend>,
    pub timer: TimerState,
    pub input: InputState,
    pub gfx: GraphicsState,
    pub fs: FilesystemState,
    pub audio: AudioState,
    pub reload: ReloadState,
    pub quit_requested: bool,
}

thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

pub(crate) fn install(backend: Box<dyn Backend>, config: &Config) {
    CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(
            slot.is_none(),
            "smitten::run is already running on this thread"
        );
        *slot = Some(Context {
            backend,
            timer: TimerState::default(),
            input: InputState::default(),
            gfx: GraphicsState::default(),
            fs: FilesystemState::new(config),
            audio: AudioState::default(),
            reload: ReloadState::new(config.hot_reload),
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

/// Runs `f` with the framework state if it is available.
///
/// Used by destructors, which may run after the game has shut down or on
/// another thread, where there is nothing left to clean up.
pub(crate) fn try_with(f: impl FnOnce(&mut Context)) {
    let _ = CONTEXT.try_with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut()
            && let Some(ctx) = slot.as_mut()
        {
            f(ctx);
        }
    });
}
