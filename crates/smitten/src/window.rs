//! The game window, the equivalent of `love.window`.

use crate::context;

/// Sets the window title.
pub fn set_title(title: &str) {
    context::with(|ctx| ctx.backend.set_title(title));
}
