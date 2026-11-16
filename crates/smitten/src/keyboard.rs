//! Keyboard state, the equivalent of `love.keyboard`.

use crate::Key;
use crate::context;

/// Returns whether `key` is currently held.
pub fn is_down(key: Key) -> bool {
    context::with(|ctx| ctx.input.keys.contains(&key))
}

/// Returns whether any of `keys` is currently held.
pub fn is_any_down(keys: &[Key]) -> bool {
    context::with(|ctx| keys.iter().any(|key| ctx.input.keys.contains(key)))
}
