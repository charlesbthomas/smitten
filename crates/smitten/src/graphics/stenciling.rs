//! The stencil, depth and scissor tests.

use super::*;

/// Marks the pixels covered by what `draw` draws, so that
/// [`set_stencil_test`] can limit later drawing to them, or keep it away
/// from them. Nothing `draw` draws is visible itself.
///
/// Every pixel has a stencil value from 0 to 255. This first resets them
/// all to 0, then sets the covered pixels to 1. Use [`stencil_with`] for
/// other values or to build on earlier marks.
///
/// ```no_run
/// use smitten::graphics::{self, DrawMode};
/// use smitten::CompareMode;
///
/// // Draw a picture only inside a circle.
/// graphics::stencil(|| graphics::circle(DrawMode::Fill, (200.0, 200.0), 80.0));
/// graphics::set_stencil_test(CompareMode::Greater, 0);
/// // ... draw the picture ...
/// graphics::clear_stencil_test();
/// ```
///
/// The screen and each canvas have their own stencil values. The screen's
/// are also reset at the start of every frame.
pub fn stencil(draw: impl FnOnce()) {
    stencil_with(StencilWrite::default(), draw);
}

/// Changes the stencil values of the pixels covered by what `draw` draws,
/// as `write` says, the equivalent of LÖVE's
/// `stencil(fn, action, value, keepvalues)`.
///
/// ```no_run
/// use smitten::graphics::{self, DrawMode, StencilWrite};
/// use smitten::StencilAction;
///
/// // Count how many circles cover each pixel.
/// graphics::stencil_with(
///     StencilWrite { action: StencilAction::Increment, ..StencilWrite::default() },
///     || graphics::circle(DrawMode::Fill, (200.0, 200.0), 80.0),
/// );
/// ```
pub fn stencil_with(write: StencilWrite, draw: impl FnOnce()) {
    let StencilWrite {
        action,
        value,
        keep_values,
    } = write;
    let previous = context::with(|ctx| {
        let batch = &mut ctx.gfx.batch;
        if !keep_values {
            batch.clear_stencil();
        }
        let previous = (batch.stencil, batch.stencil_value);
        batch.stencil = StencilMode::Write(action);
        batch.stencil_value = value;
        previous
    });
    draw();
    context::with(|ctx| (ctx.gfx.batch.stencil, ctx.gfx.batch.stencil_value) = previous);
}

/// How [`stencil_with`] changes the stencil values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StencilWrite {
    /// How the values change: [`StencilAction::Replace`] sets them to
    /// `value`, while the other actions count up, count down (stopping
    /// at 255 and 0, or wrapping around with the `Wrap` actions) or flip
    /// them and ignore `value`. Defaults to `Replace`.
    pub action: StencilAction,
    /// The value `Replace` sets. Defaults to 1.
    pub value: u8,
    /// Whether to build on the existing values instead of resetting them
    /// all to 0 first. Defaults to `false`.
    pub keep_values: bool,
}

impl Default for StencilWrite {
    fn default() -> Self {
        StencilWrite {
            action: StencilAction::Replace,
            value: 1,
            keep_values: false,
        }
    }
}

/// Draws only the pixels whose stencil value compares to `value` as
/// `compare` says, until [`clear_stencil_test`] is called.
///
/// `set_stencil_test(CompareMode::Greater, 0)` draws only where
/// [`stencil`] marked, and `set_stencil_test(CompareMode::Equal, 0)` only
/// where it did not.
pub fn set_stencil_test(compare: CompareMode, value: u8) {
    context::with(|ctx| {
        ctx.gfx.batch.stencil = StencilMode::Test(compare);
        ctx.gfx.batch.stencil_value = value;
    });
}

/// Goes back to drawing everywhere.
pub fn clear_stencil_test() {
    context::with(|ctx| ctx.gfx.batch.stencil = StencilMode::Off);
}

/// Returns the stencil test as `(compare, value)`, if one is set.
pub fn stencil_test() -> Option<(CompareMode, u8)> {
    context::with(|ctx| match ctx.gfx.batch.stencil {
        StencilMode::Test(compare) => Some((compare, ctx.gfx.batch.stencil_value)),
        _ => None,
    })
}
