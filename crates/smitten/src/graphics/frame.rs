//! Clearing, presenting and the screen.

use super::*;

/// Clears the screen, or the active canvas. The scissor does not limit it.
///
/// A [`Color`] fills the picture with it, replacing whatever was there,
/// resets the stencil values to 0 and the depth values to 1.
/// [`ClearValues`] picks which of those to clear and to what.
///
/// ```no_run
/// use smitten::{graphics, Color};
/// use smitten::graphics::ClearValues;
///
/// graphics::clear(Color::BLACK);
/// // Reset only the stencil, keeping the picture.
/// graphics::clear(ClearValues::NONE.stencil(0));
/// ```
pub fn clear(values: impl Into<ClearValues>) {
    let ClearValues {
        color,
        stencil,
        depth,
    } = values.into();
    context::with(|ctx| ctx.gfx.batch.clear_target_ex(color, stencil, depth));
}

/// What [`clear`] clears: the picture, the stencil values and the depth
/// values, each to its own value, or left as they are when `None`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ClearValues {
    /// The color to fill the picture with.
    pub color: Option<Color>,
    /// The value to set every stencil value to.
    pub stencil: Option<u8>,
    /// The value to set every depth value to, from 0 to 1.
    pub depth: Option<f32>,
}

impl ClearValues {
    /// Clears nothing, to add to with the other methods.
    pub const NONE: ClearValues = ClearValues {
        color: None,
        stencil: None,
        depth: None,
    };

    /// Also fills the picture with `color`.
    #[must_use]
    pub fn color(self, color: Color) -> Self {
        Self {
            color: Some(color),
            ..self
        }
    }

    /// Also sets every stencil value to `value`.
    #[must_use]
    pub fn stencil(self, value: u8) -> Self {
        Self {
            stencil: Some(value),
            ..self
        }
    }

    /// Also sets every depth value to `value`, from 0 to 1.
    #[must_use]
    pub fn depth(self, value: f32) -> Self {
        Self {
            depth: Some(value),
            ..self
        }
    }
}

impl From<Color> for ClearValues {
    fn from(color: Color) -> Self {
        ClearValues::NONE.color(color).stencil(0).depth(1.0)
    }
}

/// Fills each canvas being drawn to with its own color, when several
/// canvases are set. `colors[0]` is for the first canvas; a canvas with
/// `None`, or with no entry, keeps its picture. `stencil` sets every
/// stencil value and `depth` every depth value, if given. The scissor does
/// not limit it.
pub fn clear_canvases(colors: &[Option<Color>], stencil: Option<u8>, depth: Option<f32>) {
    let mut each = [None; MAX_CANVASES];
    for (slot, color) in each.iter_mut().zip(colors) {
        *slot = *color;
    }
    context::with(|ctx| ctx.gfx.batch.clear_targets(each, stencil, depth));
}

/// Sets the color every frame is cleared to before `draw` runs.
pub fn set_background_color(color: Color) {
    context::with(|ctx| ctx.gfx.background = color);
}

/// Returns the current background color.
pub fn background_color() -> Color {
    context::with(|ctx| ctx.gfx.background)
}

/// Width of the screen, or of the active canvas.
pub fn width() -> u32 {
    dimensions().0
}

/// Height of the screen, or of the active canvas.
pub fn height() -> u32 {
    dimensions().1
}

/// Size of the screen, or of the active canvas, as `(width, height)`.
///
/// This is the space the game draws in. It is measured in units, which
/// are pixels on an ordinary display and larger on a high-DPI one (see
/// [`Config::dpi_scaling`](crate::Config::dpi_scaling)). With a
/// [`Config::design_size`](crate::Config::design_size) it is always that
/// size. [`pixel_dimensions`] gives the real pixels.
pub fn dimensions() -> (u32, u32) {
    context::with(|ctx| ctx.gfx.target_size())
}

/// Width in real pixels of the screen, or of the active canvas.
pub fn pixel_width() -> u32 {
    pixel_dimensions().0
}

/// Height in real pixels of the screen, or of the active canvas.
pub fn pixel_height() -> u32 {
    pixel_dimensions().1
}

/// Size in real pixels of the screen, or of the active canvas. Shaders see
/// positions in these pixels.
pub fn pixel_dimensions() -> (u32, u32) {
    context::with(|ctx| ctx.gfx.target_pixel_size())
}
