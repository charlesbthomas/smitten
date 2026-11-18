//! Touch screen state, the equivalent of `love.touch`.
//!
//! Positions are in the same coordinates as [`crate::mouse`]: units from
//! the top-left corner of the window, or of the design area when the game
//! has a design size. The `touch_pressed`, `touch_moved` and
//! `touch_released` callbacks on [`crate::Game`] report each change.

use crate::TouchId;
use crate::context;
use glam::Vec2;

/// The fingers currently on the screen, in the order they touched it.
pub fn touches() -> Vec<TouchId> {
    context::with(|ctx| ctx.input.touches.iter().map(|touch| touch.id).collect())
}

/// Where the finger `id` is, or `None` if it is not on the screen.
pub fn position(id: TouchId) -> Option<Vec2> {
    context::with(|ctx| ctx.input.touch(id).map(|touch| touch.position)).map(Vec2::from)
}

/// How hard the finger `id` is pressing, from 0.0 to 1.0, or `None` if
/// it is not on the screen. Screens that cannot measure pressure report
/// 1.0.
pub fn pressure(id: TouchId) -> Option<f32> {
    context::with(|ctx| ctx.input.touch(id).map(|touch| touch.pressure))
}
