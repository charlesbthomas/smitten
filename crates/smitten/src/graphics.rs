//! Drawing, the equivalent of `love.graphics`.
//!
//! Only clearing exists so far. Images, shapes and text arrive with the
//! core graphics milestone.

use crate::Color;
use crate::context;

/// Clears the screen to `color` for this frame.
pub fn clear(color: Color) {
    context::with(|ctx| ctx.frame_clear = color);
}

/// Sets the color every frame is cleared to before `draw` runs.
pub fn set_background_color(color: Color) {
    context::with(|ctx| ctx.background = color);
}

/// Returns the current background color.
pub fn background_color() -> Color {
    context::with(|ctx| ctx.background)
}

/// Drawable width in pixels.
pub fn width() -> u32 {
    dimensions().0
}

/// Drawable height in pixels.
pub fn height() -> u32 {
    dimensions().1
}

/// Drawable size in pixels as `(width, height)`.
pub fn dimensions() -> (u32, u32) {
    context::with(|ctx| ctx.backend.size_in_pixels())
}
