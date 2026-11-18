//! The active shader.

use super::*;

/// Draws everything from now on with `shader`, or with the built-in
/// shader for `None`.
///
/// The shader stays set until changed, even across frames.
pub fn set_shader(shader: Option<&Shader>) {
    context::with(|ctx| ctx.gfx.set_shader(shader.cloned()));
}

/// Returns the shader in use, or `None` for the built-in one.
pub fn shader() -> Option<Shader> {
    context::with(|ctx| ctx.gfx.shader.clone())
}
