//! The transform stack: push, pop, translate and the rest.

use super::*;

/// The most [`push`]es that can wait for a [`pop`] at once, the size of
/// LÖVE's stack.
pub const MAX_STACK_DEPTH: usize = 64;

/// Saves the current transform so [`pop`] can restore it.
///
/// Panics if more than [`MAX_STACK_DEPTH`] pushes are waiting for a
/// [`pop`], which is almost always a missing `pop` in the game.
pub fn push() {
    push_kind(StackType::Transform);
}

/// Saves the transform and every other drawing setting so [`pop`] can
/// restore them, the equivalent of `love.graphics.push("all")`.
///
/// That is the color, the background color, the font, the blend mode,
/// the shader, the canvas, the scissor, the stencil test, the line width,
/// join and style, the point size, the color mask, wireframe drawing, the
/// default filter, and the mesh cull mode and front face winding. A helper
/// can then change any of them without the caller having to put them
/// back.
///
/// ```no_run
/// use smitten::{graphics, BlendMode, Color};
///
/// fn draw_glow() {
///     graphics::push_all();
///     graphics::set_blend_mode(BlendMode::Add);
///     graphics::set_color(Color::rgb(1.0, 0.8, 0.3));
///     // ... draw ...
///     graphics::pop();
/// }
/// ```
#[doc(alias = "love.graphics.push")]
pub fn push_all() {
    push_kind(StackType::All);
}

/// Saves what `kind` says, for [`pop`] to restore.
fn push_kind(kind: StackType) {
    context::with(|ctx| {
        assert!(
            ctx.gfx.stack.len() < MAX_STACK_DEPTH,
            "graphics::push was called more than {MAX_STACK_DEPTH} times without a graphics::pop"
        );
        ctx.gfx.push_state(kind);
    });
}

/// Restores what the matching [`push`] or [`push_all`] saved.
///
/// Panics if there is no matching `push`, which is always a bug in the game.
pub fn pop() {
    context::with(|ctx| ctx.gfx.pop_state());
}

/// Runs `draw` between a [`push`] and its [`pop`], so changes it makes to
/// the transform are undone afterwards, and returns what `draw` returns.
///
/// ```no_run
/// use smitten::graphics::{self, DrawMode};
///
/// graphics::scoped(|| {
///     graphics::translate((100.0, 100.0));
///     graphics::rotate(0.5);
///     graphics::rectangle(DrawMode::Fill, (-10.0, -10.0), (20.0, 20.0));
/// });
/// // Back to the transform from before.
/// ```
pub fn scoped<R>(draw: impl FnOnce() -> R) -> R {
    push();
    let result = draw();
    pop();
    result
}

/// Runs `draw` between a [`push_all`] and its [`pop`], so every drawing
/// setting it changes is put back afterwards, and returns what `draw`
/// returns.
pub fn scoped_all<R>(draw: impl FnOnce() -> R) -> R {
    push_all();
    let result = draw();
    pop();
    result
}

/// How many [`push`]es are waiting for a [`pop`], the equivalent of
/// `love.graphics.getStackDepth`. It is 0 at the start of every `draw`.
#[doc(alias = "love.graphics.getStackDepth")]
pub fn stack_depth() -> usize {
    context::with(|ctx| ctx.gfx.stack.len())
}

/// Puts every drawing setting back to how a game starts, the equivalent
/// of `love.graphics.reset`.
///
/// The color becomes white and the background black, drawing goes back to
/// the screen with the built-in shader and font, the scissor and stencil
/// test are removed, the blend mode, line and point settings, color mask,
/// wireframe drawing and default filter go back to their defaults, and the
/// transform to the origin. What [`push`] saved is kept.
#[doc(alias = "love.graphics.reset")]
pub fn reset() {
    context::with(|ctx| ctx.gfx.reset_state());
}

/// Moves everything drawn afterwards by `(dx, dy)`.
pub fn translate(delta: impl Into<Vec2>) {
    let Vec2 { x: dx, y: dy } = delta.into();
    context::with(|ctx| ctx.gfx.transform.translate((dx, dy)));
}

/// Rotates everything drawn afterwards by `angle` radians around the
/// current origin, clockwise on screen.
pub fn rotate(angle: f32) {
    context::with(|ctx| ctx.gfx.transform.rotate(angle));
}

/// Scales everything drawn afterwards.
pub fn scale(sx: f32, sy: f32) {
    context::with(|ctx| ctx.gfx.transform.scale(sx, sy));
}

/// Resets the transform so positions are plain window pixels again.
pub fn origin() {
    context::with(|ctx| ctx.gfx.transform = Transform::IDENTITY);
}

/// Shears everything drawn afterwards: x gains `kx` times y, and y gains
/// `ky` times x.
pub fn shear(shear: impl Into<Vec2>) {
    let Vec2 { x: kx, y: ky } = shear.into();
    context::with(|ctx| ctx.gfx.transform.shear((kx, ky)));
}

/// Applies `transform` on top of the current transform, as if its calls
/// had been made here.
pub fn apply_transform(transform: &Transform) {
    context::with(|ctx| ctx.gfx.transform.apply_transform(transform));
}

/// Replaces the current transform with `transform`.
pub fn replace_transform(transform: &Transform) {
    context::with(|ctx| ctx.gfx.transform = *transform);
}

/// The current transform, as set by [`translate`], [`apply_transform`]
/// and the rest. [`push`] and [`pop`] save and restore it.
pub fn transform() -> Transform {
    context::with(|ctx| ctx.gfx.transform)
}

/// Where the point `(x, y)` lands on the screen, in units, under the
/// current transform.
pub fn transform_point(position: impl Into<Vec2>) -> Vec2 {
    let Vec2 { x, y } = position.into();
    context::with(|ctx| ctx.gfx.transform.transform_point((x, y)))
}

/// Which point the current transform puts at `(x, y)` on the screen, such
/// as where the mouse is in a scrolled and zoomed world.
pub fn inverse_transform_point(position: impl Into<Vec2>) -> Vec2 {
    let Vec2 { x, y } = position.into();
    context::with(|ctx| ctx.gfx.transform.inverse_transform_point((x, y)))
}
