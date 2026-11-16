//! Game-level events, the equivalent of `love.event`.

use crate::context;

/// Asks the game to close. [`crate::Game::quit`] gets a chance to refuse.
pub fn quit() {
    context::with(|ctx| ctx.quit_requested = true);
}
