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

/// Reads two sets of keys as one direction: -1.0 if any of `negative`
/// is held, 1.0 if any of `positive` is, and 0.0 for neither or both.
///
/// ```no_run
/// # use smitten::{Key, keyboard};
/// let steer = keyboard::axis(&[Key::Left, Key::A], &[Key::Right, Key::D]);
/// ```
pub fn axis(negative: &[Key], positive: &[Key]) -> f32 {
    f32::from(i8::from(is_any_down(positive)) - i8::from(is_any_down(negative)))
}

/// The direction the arrow keys and W, A, S, D are asking for, with
/// right and down positive as on screen. It is never longer than 1, so
/// walking diagonally is no faster than walking straight.
///
/// LÖVE has no such call; nearly every game writes it out by hand.
pub fn direction() -> (f32, f32) {
    let x = axis(&[Key::Left, Key::A], &[Key::Right, Key::D]);
    let y = axis(&[Key::Up, Key::W], &[Key::Down, Key::S]);
    crate::input::limit_length(x, y)
}
