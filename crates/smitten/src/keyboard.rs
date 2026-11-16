//! Keyboard state, the equivalent of `love.keyboard`.

use crate::Key;
use crate::context;

/// Returns whether `key` is currently held.
pub fn is_down(key: Key) -> bool {
    context::with(|ctx| ctx.keys_down.contains(&key))
}

/// Returns whether any of `keys` is currently held.
pub fn is_any_down(keys: &[Key]) -> bool {
    context::with(|ctx| keys.iter().any(|key| ctx.keys_down.contains(key)))
}
