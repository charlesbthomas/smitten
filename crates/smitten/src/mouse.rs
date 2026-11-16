//! Mouse state, the equivalent of `love.mouse`.
//!
//! Positions are in the same coordinates `graphics` draws to the screen
//! in: units from the top-left corner of the window, or of the design
//! area when the game has a design size.

use crate::MouseButton;
use crate::context;

/// The pointer's position as `(x, y)`.
pub fn position() -> (f32, f32) {
    context::with(|ctx| ctx.input.mouse_position)
}

/// The pointer's horizontal position.
pub fn x() -> f32 {
    position().0
}

/// The pointer's vertical position.
pub fn y() -> f32 {
    position().1
}

/// Returns whether `button` is currently held.
pub fn is_down(button: MouseButton) -> bool {
    context::with(|ctx| ctx.input.mouse_buttons.contains(&button))
}

/// Shows or hides the cursor while it is over the window.
pub fn set_visible(visible: bool) {
    context::with(|ctx| {
        ctx.input.mouse_visible = visible;
        ctx.backend.set_mouse_visible(visible);
    });
}

/// Returns whether the cursor is shown.
pub fn is_visible() -> bool {
    context::with(|ctx| ctx.input.mouse_visible)
}
