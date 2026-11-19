//! Colors, blending, lines, points and other drawing settings.

use super::*;

/// Sets the color used to draw shapes and to tint images.
///
/// White draws images unchanged. The color stays set until changed, even
/// across frames.
pub fn set_color(color: Color) {
    context::with(|ctx| ctx.gfx.color = color);
}

/// Returns the current drawing color.
pub fn color() -> Color {
    context::with(|ctx| ctx.gfx.color)
}

/// Sets how wide outlines and lines are, in pixels.
pub fn set_line_width(width: f32) {
    context::with(|ctx| ctx.gfx.line_width = width);
}

/// Returns the current line width.
pub fn line_width() -> f32 {
    context::with(|ctx| ctx.gfx.line_width)
}

/// Sets how wide and tall each point drawn by [`points`] is, in units.
pub fn set_point_size(size: f32) {
    context::with(|ctx| ctx.gfx.point_size = size);
}

/// Returns the current point size.
pub fn point_size() -> f32 {
    context::with(|ctx| ctx.gfx.point_size)
}

/// Sets how what is drawn is combined with what is already there. A
/// [`BlendMode`] on its own uses its usual alpha mode;
/// [`BlendMode::with_alpha`] chooses another.
///
/// ```no_run
/// use smitten::{graphics, BlendAlphaMode, BlendMode};
///
/// # let canvas = smitten::graphics::Canvas::new(64, 64).unwrap();
/// graphics::set_blend_mode(BlendMode::Add);
/// // Add a canvas full of soft glows onto the screen.
/// graphics::set_blend_mode(BlendMode::Add.with_alpha(BlendAlphaMode::Premultiplied));
/// graphics::draw(&canvas, (0.0, 0.0));
/// ```
///
/// Panics for [`BlendMode::Multiply`], [`BlendMode::Lighten`] or
/// [`BlendMode::Darken`] with [`BlendAlphaMode::AlphaMultiply`]: those
/// modes only work premultiplied. [`BlendMode::Premultiplied`] is always
/// premultiplied whatever the alpha mode says.
pub fn set_blend_mode(blend: impl Into<Blend>) {
    let Blend { mode, alpha } = blend.into();
    assert!(
        !(mode.needs_premultiplied()
            && mode != BlendMode::Premultiplied
            && alpha == BlendAlphaMode::AlphaMultiply),
        "the {mode:?} blend mode only works with BlendAlphaMode::Premultiplied"
    );
    context::with(|ctx| {
        ctx.gfx.batch.blend = mode;
        ctx.gfx.batch.blend_alpha = alpha;
    });
}

/// Returns the current blend mode and alpha mode.
/// [`BlendMode::Premultiplied`] comes back as itself, with
/// [`BlendAlphaMode::Premultiplied`].
pub fn blend_mode() -> Blend {
    context::with(|ctx| {
        let mode = ctx.gfx.batch.blend;
        let alpha = if mode == BlendMode::Premultiplied {
            BlendAlphaMode::Premultiplied
        } else {
            ctx.gfx.batch.blend_alpha
        };
        mode.with_alpha(alpha)
    })
}

/// Limits drawing to a rectangle of the screen or the active canvas.
///
/// The rectangle is in units and is not affected by the transform. It
/// stays set until changed or removed with [`clear_scissor`].
pub fn set_scissor(x: i32, y: i32, width: u32, height: u32) {
    context::with(|ctx| {
        ctx.gfx.scissor = Some((x, y, width, height));
        ctx.gfx.apply_scissor();
    });
}

/// Removes the scissor so drawing can reach everywhere again.
pub fn clear_scissor() {
    context::with(|ctx| {
        ctx.gfx.scissor = None;
        ctx.gfx.apply_scissor();
    });
}

/// Limits drawing to the part of the current scissor that is also inside
/// the given rectangle. With no scissor set, it is the same as
/// [`set_scissor`].
///
/// This suits nested panels: each can narrow where it draws without
/// knowing where its parent allows.
pub fn intersect_scissor(x: i32, y: i32, width: u32, height: u32) {
    context::with(|ctx| {
        let rect = (x, y, width, height);
        ctx.gfx.scissor = Some(match ctx.gfx.scissor {
            Some(current) => intersect(current, rect),
            None => rect,
        });
        ctx.gfx.apply_scissor();
    });
}

/// The overlap of two rectangles, empty at the corner of the first one's
/// overlap if they do not meet.
pub(super) fn intersect(a: (i32, i32, u32, u32), b: (i32, i32, u32, u32)) -> (i32, i32, u32, u32) {
    let right = |(x, _, w, _): (i32, i32, u32, u32)| i64::from(x) + i64::from(w);
    let bottom = |(_, y, _, h): (i32, i32, u32, u32)| i64::from(y) + i64::from(h);
    let left = a.0.max(b.0);
    let top = a.1.max(b.1);
    let width = (right(a).min(right(b)) - i64::from(left)).max(0) as u32;
    let height = (bottom(a).min(bottom(b)) - i64::from(top)).max(0) as u32;
    (left, top, width, height)
}

/// Returns the scissor rectangle as `(x, y, width, height)`, if one is set.
pub fn scissor() -> Option<(i32, i32, u32, u32)> {
    context::with(|ctx| ctx.gfx.scissor)
}

/// Sets the filter that newly created images start with.
///
/// Call this with [`Filter::Nearest`] before loading images in a pixel art
/// game.
pub fn set_default_filter(filter: Filter) {
    context::with(|ctx| ctx.gfx.default_filter = filter);
}

/// Sets how the corners of lines and outlines are drawn. They start out
/// mitered.
pub fn set_line_join(join: LineJoin) {
    context::with(|ctx| ctx.gfx.line_join = join);
}

/// Returns how the corners of lines and outlines are drawn.
pub fn line_join() -> LineJoin {
    context::with(|ctx| ctx.gfx.line_join)
}

/// Sets whether the edges of lines and outlines are softened. They start
/// out smooth. Pixel art usually wants [`LineStyle::Rough`].
///
/// Filled shapes are not affected: their edges are always hard.
pub fn set_line_style(style: LineStyle) {
    context::with(|ctx| ctx.gfx.line_style = style);
}

/// Returns whether the edges of lines and outlines are softened.
pub fn line_style() -> LineStyle {
    context::with(|ctx| ctx.gfx.line_style)
}

/// Chooses which color channels drawing may change. A channel left out
/// keeps what is already there. Clearing the screen or a canvas ignores the
/// mask.
pub fn set_color_mask(r: bool, g: bool, b: bool, a: bool) {
    context::with(|ctx| ctx.gfx.batch.color_mask = ColorMask { r, g, b, a });
}

/// Which color channels drawing may change, as `(r, g, b, a)`.
pub fn color_mask() -> (bool, bool, bool, bool) {
    context::with(|ctx| {
        let mask = ctx.gfx.batch.color_mask;
        (mask.r, mask.g, mask.b, mask.a)
    })
}

/// Chooses how drawing uses depth. Each pixel drawn is kept only where its
/// depth compares with the depth already at that pixel as `compare` says,
/// and with `write` its depth replaces it. `(CompareMode::Always, false)`,
/// the default, leaves depth out.
///
/// Depth comes from the z of a mesh's 3-component `VertexPosition`, or from
/// a [`Shader`]'s `position`: z from -10 to 10 maps to depth 1 to 0, so a
/// larger z is nearer. Everything else is drawn at z 0, depth 0.5. The
/// screen and every canvas have depth values of their own, which [`clear`]
/// resets to 1.
///
/// ```no_run
/// use smitten::{CompareMode, graphics};
///
/// graphics::set_depth_mode(CompareMode::Less, true);
/// // Draw meshes in any order; nearer pixels win.
/// graphics::set_depth_mode(CompareMode::Always, false);
/// ```
pub fn set_depth_mode(compare: CompareMode, write: bool) {
    context::with(|ctx| ctx.gfx.batch.depth = DepthMode { compare, write });
}

/// How drawing uses depth, as `(compare, write)`. See [`set_depth_mode`].
pub fn depth_mode() -> (CompareMode, bool) {
    context::with(|ctx| {
        let depth = ctx.gfx.batch.depth;
        (depth.compare, depth.write)
    })
}

/// Draws only the edges of every triangle from now on, one pixel wide, to
/// see how shapes, images and text are built. For debugging.
pub fn set_wireframe(enabled: bool) {
    context::with(|ctx| ctx.gfx.batch.wireframe = enabled);
}

/// Whether wireframe drawing is on.
pub fn is_wireframe() -> bool {
    context::with(|ctx| ctx.gfx.batch.wireframe)
}

/// What the frame has drawn so far and how many images, canvases and fonts
/// the game holds. Call it at the end of `draw` to see the whole frame.
pub fn stats() -> Stats {
    use stats::{Kind, alive};
    context::with(|ctx| {
        let counts = ctx.gfx.batch.counts;
        Stats {
            draw_calls: counts.draw_calls,
            draw_calls_batched: counts.draw_calls_batched,
            canvas_switches: counts.canvas_switches,
            shader_switches: counts.shader_switches,
            texture_memory: ctx.backend.texture_memory(),
            images: alive(Kind::Image),
            canvases: alive(Kind::Canvas),
            fonts: alive(Kind::Font),
        }
    })
}

/// Returns the filter that newly created images start with.
pub fn default_filter() -> Filter {
    context::with(|ctx| ctx.gfx.default_filter)
}

/// Sets which triangles of meshes are left out. Only [meshes](Mesh) are
/// affected.
///
/// Leaving out the back faces of a closed 3D-looking shape skips the
/// triangles that face away. [`set_front_face_winding`] says which faces
/// are the front.
pub fn set_mesh_cull_mode(mode: CullMode) {
    context::with(|ctx| ctx.gfx.mesh_cull = mode);
}

/// Which triangles of meshes are left out.
pub fn mesh_cull_mode() -> CullMode {
    context::with(|ctx| ctx.gfx.mesh_cull)
}

/// Sets which way round a triangle's corners go, on screen, when it faces
/// the viewer. It starts out counter-clockwise. A transform that flips the
/// picture flips the winding too.
pub fn set_front_face_winding(winding: VertexWinding) {
    context::with(|ctx| ctx.gfx.front_face = winding);
}

/// Which way round a triangle's corners go when it faces the viewer.
pub fn front_face_winding() -> VertexWinding {
    context::with(|ctx| ctx.gfx.front_face)
}

/// The graphics API, driver and GPU doing the drawing. Useful in bug
/// reports.
pub fn renderer_info() -> RendererInfo {
    context::with(|ctx| ctx.backend.renderer_info())
}

/// Checks WGSL shader source without creating a shader. The error is the
/// message [`Shader::new`] would give.
pub fn validate_shader(source: &str) -> Result<(), ShaderError> {
    context::with(|ctx| ctx.backend.validate_shader(source)).map_err(ShaderError::Compile)
}

/// Shows what has been drawn since the last present, then starts the next
/// frame: the screen is cleared to [`background_color`] and the transform
/// is reset.
///
/// A custom main loop made with [`crate::run_loop`] calls this once a
/// frame. With [`crate::Game`], Smitten presents after `draw` by itself, so
/// calling it there shows a half-drawn frame.
pub fn present() {
    present_frame();
    context::with(|ctx| ctx.gfx.begin_frame(&mut *ctx.backend));
}

/// Hands the frame to the window, or draws it offscreen while the window
/// is closed, and runs the screenshot callbacks it asked for.
pub(crate) fn present_frame() {
    let screenshots = context::with(|ctx| {
        ctx.gfx.finish_frame();
        let screenshots = ctx.gfx.take_screenshots(&mut *ctx.backend);
        if ctx.window_closed {
            ctx.backend.render_offscreen(&ctx.gfx.frame());
        } else {
            ctx.backend.present(&ctx.gfx.frame());
        }
        ctx.gfx.end_frame(&mut *ctx.backend);
        screenshots
    });
    for (callback, picture) in screenshots {
        callback(picture);
    }
}

/// Whether drawing reaches the screen. It is `false` while the window is
/// closed with [`crate::window::close`]; drawing still works then, but only
/// canvases are worth drawing to. A custom main loop can skip drawing the
/// screen while it is `false`.
pub fn is_active() -> bool {
    context::with(|ctx| !ctx.window_closed)
}

/// Whether colors are blended in linear light. True when
/// [`Config::gamma_correct`](crate::Config::gamma_correct) was set and the
/// display allows it.
pub fn is_gamma_correct() -> bool {
    context::with(|ctx| ctx.backend.is_gamma_correct())
}

/// The format of an ordinary image or canvas: sRGB-encoded with
/// gamma-correct rendering.
pub(crate) fn normal_format(backend: &dyn Backend) -> PixelFormat {
    if backend.is_gamma_correct() {
        PixelFormat::Srgba8
    } else {
        PixelFormat::Rgba8
    }
}
