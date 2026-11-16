//! Mouse state, the equivalent of `love.mouse`.
//!
//! Positions are in the same coordinates `graphics` draws to the screen
//! in: units from the top-left corner of the window, or of the design
//! area when the game has a design size.

use crate::context;
use crate::{Cursor, MouseButton};

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

/// Moves the pointer to `(x, y)`, in the same coordinates as
/// [`position`]. A `mouse_moved` callback follows.
pub fn set_position(x: f32, y: f32) {
    context::with(|ctx| {
        let view = ctx.gfx.screen.view();
        ctx.backend
            .warp_mouse(view.x + x * view.scale, view.y + y * view.scale);
    });
}

/// Moves the pointer horizontally, keeping its vertical position.
pub fn set_x(x: f32) {
    set_position(x, self::y());
}

/// Moves the pointer vertically, keeping its horizontal position.
pub fn set_y(y: f32) {
    set_position(self::x(), y);
}

/// Chooses one of the operating system's cursors, the equivalent of
/// `love.mouse.setCursor(love.mouse.getSystemCursor(name))`.
pub fn set_cursor(cursor: Cursor) {
    context::with(|ctx| match ctx.backend.set_cursor(cursor) {
        Ok(()) => ctx.input.cursor = cursor,
        // A system without that cursor keeps the one it had.
        Err(message) => eprintln!("smitten: could not set the cursor: {message}"),
    });
}

/// Returns the cursor last chosen with [`set_cursor`].
pub fn cursor() -> Cursor {
    context::with(|ctx| ctx.input.cursor)
}

/// Turns relative mode on or off. While it is on the cursor is hidden and
/// kept in the window, and `mouse_moved` keeps reporting movement however
/// far the mouse travels. Use it for a camera steered by the mouse.
pub fn set_relative_mode(relative: bool) {
    context::with(|ctx| match ctx.backend.set_relative_mouse(relative) {
        Ok(()) => ctx.input.mouse_relative = relative,
        Err(message) => eprintln!("smitten: could not change relative mouse mode: {message}"),
    });
}

/// Returns whether relative mode is on.
pub fn relative_mode() -> bool {
    context::with(|ctx| ctx.input.mouse_relative)
}

/// Keeps the pointer inside the window, or lets it leave again.
pub fn set_grabbed(grabbed: bool) {
    context::with(|ctx| match ctx.backend.set_mouse_grabbed(grabbed) {
        Ok(()) => ctx.input.mouse_grabbed = grabbed,
        Err(message) => eprintln!("smitten: could not grab the mouse: {message}"),
    });
}

/// Returns whether the pointer is kept inside the window.
pub fn is_grabbed() -> bool {
    context::with(|ctx| ctx.input.mouse_grabbed)
}
