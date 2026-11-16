//! Drawing, the equivalent of `love.graphics`.
//!
//! Only clearing exists so far. Images, shapes and text arrive with the
//! core graphics milestone.

use crate::context;

/// An RGBA color with components from 0.0 to 1.0, as in LÖVE 11.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);

    /// An opaque color.
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

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
