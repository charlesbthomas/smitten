//! Drawing, the equivalent of `love.graphics`.
//!
//! Positions are in pixels from the top-left corner of the window, with y
//! pointing down. Everything drawn is tinted by the current color and moved
//! by the current transform, both of which are set with functions here.
//!
//! Drawing normally goes to the screen. A [`Canvas`] is an off-screen
//! image to draw to instead, for effects, lighting, or rendering a
//! low-resolution game and scaling it up.
//!
//! A [`Shader`] replaces the way pixels are colored, for effects such as
//! grayscale, outlines or distortion.

mod batch;
mod canvas;
pub(crate) mod image;
mod particles;
mod shader;
mod shapes;
mod text;
mod transform;

use std::path::Path;

use smitten_core::backend::{Backend, Frame, Scissor, TextureId, Vertex};

use crate::context;
use smitten_core::StencilMode;

use crate::{BlendMode, Color, CompareMode, Filter, StencilAction};
use batch::Batch;

pub use canvas::Canvas;
pub use image::{Image, ImageError, Quad};
pub use particles::ParticleSystem;
pub use shader::{Shader, ShaderError, UniformValue};
pub use text::{Align, Font, FontError};
pub use transform::Transform;

/// Whether a shape is filled in or drawn as an outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawMode {
    Fill,
    Line,
}

pub(crate) struct GraphicsState {
    background: Color,
    color: Color,
    line_width: f32,
    pub(crate) default_filter: Filter,
    transform: Transform,
    stack: Vec<Transform>,
    batch: Batch,
    /// The font `print` uses. Created on first use so games that never
    /// print text pay nothing for it.
    font: Option<Font>,
    /// Where drawing goes, or `None` for the screen.
    canvas: Option<Canvas>,
    /// The custom shader in use, if any.
    shader: Option<Shader>,
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            background: Color::BLACK,
            color: Color::WHITE,
            line_width: 1.0,
            default_filter: Filter::default(),
            transform: Transform::IDENTITY,
            stack: Vec::new(),
            batch: Batch::default(),
            font: None,
            canvas: None,
            shader: None,
        }
    }
}

impl GraphicsState {
    /// Resets what LÖVE resets before each `draw`: the screen and the
    /// transform. The color, line width, blend mode and scissor carry over
    /// between frames.
    ///
    /// Anything queued for a canvas before now, in `load` or `update`, is
    /// kept and drawn with this frame.
    pub(crate) fn begin_frame(&mut self) {
        self.set_canvas(None);
        self.batch.clear_target(self.background);
        self.transform = Transform::IDENTITY;
        self.stack.clear();
    }

    /// Called once the frame has been handed to the backend.
    pub(crate) fn end_frame(&mut self) {
        self.batch.clear();
        self.set_canvas(None);
    }

    pub(crate) fn frame(&self) -> Frame<'_> {
        Frame {
            vertices: &self.batch.vertices,
            calls: &self.batch.calls,
            uniforms: &self.batch.uniforms,
            passes: &self.batch.passes,
        }
    }

    fn set_shader(&mut self, shader: Option<Shader>) {
        match &shader {
            Some(shader) => {
                shader.with_data(|data| self.batch.set_shader(Some((shader.id(), data))))
            }
            None => self.batch.set_shader(None),
        }
        self.shader = shader;
    }

    /// Picks up new values sent to `shader` if it is the one in use.
    pub(crate) fn refresh_shader(&mut self, shader: &Shader) {
        if self.shader.as_ref().is_some_and(|active| active.is(shader)) {
            shader.with_data(|data| self.batch.set_shader(Some((shader.id(), data))));
        }
    }

    fn set_canvas(&mut self, canvas: Option<Canvas>) {
        self.batch
            .set_target(canvas.as_ref().map(|canvas| canvas.texture()));
        self.canvas = canvas;
    }

    /// Queues untextured triangles given in local coordinates.
    fn push_shape(&mut self, triangles: &[(f32, f32)]) {
        let color = color_array(self.color);
        let vertices: Vec<Vertex> = triangles
            .iter()
            .map(|&(x, y)| {
                let (x, y) = self.transform.apply(x, y);
                Vertex {
                    position: [x, y],
                    uv: [0.0, 0.0],
                    color,
                }
            })
            .collect();
        self.batch.push(None, Filter::default(), &vertices);
    }

    /// Queues the part of `image` covered by `quad`, placed by `local`.
    fn push_image(&mut self, image: &Image, quad: &Quad, local: &Transform) {
        let transform = self.transform.then_inner(local);
        let color = color_array(self.color);
        let (u0, v0) = (quad.x / quad.sheet_width, quad.y / quad.sheet_height);
        let (u1, v1) = (
            (quad.x + quad.width) / quad.sheet_width,
            (quad.y + quad.height) / quad.sheet_height,
        );
        let corner = |x: f32, y: f32, u: f32, v: f32| {
            let (x, y) = transform.apply(x, y);
            Vertex {
                position: [x, y],
                uv: [u, v],
                color,
            }
        };
        let top_left = corner(0.0, 0.0, u0, v0);
        let top_right = corner(quad.width, 0.0, u1, v0);
        let bottom_right = corner(quad.width, quad.height, u1, v1);
        let bottom_left = corner(0.0, quad.height, u0, v1);
        self.batch.push(
            Some(image.texture()),
            image.filter(),
            &[
                top_left,
                top_right,
                bottom_right,
                top_left,
                bottom_right,
                bottom_left,
            ],
        );
    }
}

impl GraphicsState {
    /// Queues every particle of `system`, placed by `local`.
    fn push_particles(&mut self, system: &ParticleSystem, local: &Transform) {
        let transform = self.transform.then_inner(local);
        let vertices = system.vertices(&transform, self.color);
        let image = system.image();
        self.batch
            .push(Some(image.texture()), image.filter(), &vertices);
    }

    /// Queues `text` in the current font, placed by `local`.
    fn push_text(
        &mut self,
        backend: &mut dyn Backend,
        text: &str,
        wrap: Option<(f32, Align)>,
        local: &Transform,
    ) {
        let default_filter = self.default_filter;
        let font = self
            .font
            .get_or_insert_with(|| Font::fallback(default_filter))
            .clone();
        let transform = self.transform.then_inner(local);
        let color = color_array(self.color);
        let filter = font.filter();

        // Glyphs usually share one atlas page, so collect each run of
        // quads on the same page and queue it in one go.
        let mut run: Vec<Vertex> = Vec::with_capacity(text.len() * 6);
        let mut run_texture: Option<TextureId> = None;
        let batch = &mut self.batch;
        font.layout(backend, text, wrap, |texture, rect, uv| {
            if run_texture != Some(texture) {
                batch.push(run_texture, filter, &run);
                run.clear();
                run_texture = Some(texture);
            }
            let corner = |x: f32, y: f32, u: f32, v: f32| {
                let (x, y) = transform.apply(x, y);
                Vertex {
                    position: [x, y],
                    uv: [u, v],
                    color,
                }
            };
            let top_left = corner(rect[0], rect[1], uv[0], uv[1]);
            let top_right = corner(rect[2], rect[1], uv[2], uv[1]);
            let bottom_right = corner(rect[2], rect[3], uv[2], uv[3]);
            let bottom_left = corner(rect[0], rect[3], uv[0], uv[3]);
            run.extend_from_slice(&[
                top_left,
                top_right,
                bottom_right,
                top_left,
                bottom_right,
                bottom_left,
            ]);
        });
        batch.push(run_texture, filter, &run);
    }
}

fn color_array(color: Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn whole_image(image: &Image) -> Quad {
    let (width, height) = (image.width() as f32, image.height() as f32);
    Quad::new(0.0, 0.0, width, height, width, height)
}

// ---------------------------------------------------------------------------
// Screen

/// Fills the screen, or the active canvas, with `color`, replacing
/// whatever was there. The scissor does not limit it.
pub fn clear(color: Color) {
    context::with(|ctx| ctx.gfx.batch.clear_target(color));
}

/// Sets the color every frame is cleared to before `draw` runs.
pub fn set_background_color(color: Color) {
    context::with(|ctx| ctx.gfx.background = color);
}

/// Returns the current background color.
pub fn background_color() -> Color {
    context::with(|ctx| ctx.gfx.background)
}

/// Width in pixels of the screen, or of the active canvas.
pub fn width() -> u32 {
    dimensions().0
}

/// Height in pixels of the screen, or of the active canvas.
pub fn height() -> u32 {
    dimensions().1
}

/// Size in pixels of the screen, or of the active canvas, as
/// `(width, height)`.
pub fn dimensions() -> (u32, u32) {
    context::with(|ctx| match &ctx.gfx.canvas {
        Some(canvas) => canvas.dimensions(),
        None => ctx.backend.size_in_pixels(),
    })
}

// ---------------------------------------------------------------------------
// Canvases

/// Creates a transparent canvas of `width` by `height` pixels.
///
/// Panics if the GPU cannot provide it, for instance because it is larger
/// than the GPU supports. Use [`Canvas::new`] to handle the failure instead.
pub fn new_canvas(width: u32, height: u32) -> Canvas {
    Canvas::new(width, height)
        .unwrap_or_else(|message| panic!("could not create a {width}x{height} canvas: {message}"))
}

/// Sends everything drawn from now on to `canvas`, or back to the screen
/// for `None`.
///
/// Positions are then in pixels from the canvas's top-left corner. The
/// screen becomes the target again at the start of every `draw`.
pub fn set_canvas(canvas: Option<&Canvas>) {
    context::with(|ctx| ctx.gfx.set_canvas(canvas.cloned()));
}

/// Returns the canvas being drawn to, or `None` for the screen.
pub fn canvas() -> Option<Canvas> {
    context::with(|ctx| ctx.gfx.canvas.clone())
}

// ---------------------------------------------------------------------------
// Stencil

/// Marks the pixels covered by what `draw` draws, so that
/// [`set_stencil_test`] can limit later drawing to them, or keep it away
/// from them. Nothing `draw` draws is visible itself.
///
/// Every pixel has a stencil value from 0 to 255. This first resets them
/// all to 0, then sets the covered pixels to 1. Use [`stencil_ex`] for
/// other values or to build on earlier marks.
///
/// ```no_run
/// use smitten::graphics::{self, DrawMode};
/// use smitten::CompareMode;
///
/// // Draw a picture only inside a circle.
/// graphics::stencil(|| graphics::circle(DrawMode::Fill, 200.0, 200.0, 80.0));
/// graphics::set_stencil_test(CompareMode::Greater, 0);
/// // ... draw the picture ...
/// graphics::clear_stencil_test();
/// ```
///
/// The screen and each canvas have their own stencil values. The screen's
/// are also reset at the start of every frame.
pub fn stencil(draw: impl FnOnce()) {
    stencil_ex(StencilAction::Replace, 1, false, draw);
}

/// Changes the stencil values of the pixels covered by what `draw` draws.
///
/// `action` says how: [`StencilAction::Replace`] sets them to `value`,
/// while the other actions count up, count down or flip them and ignore
/// `value`. With `keep_values` the existing values are built on;
/// otherwise they are all reset to 0 first.
pub fn stencil_ex(action: StencilAction, value: u8, keep_values: bool, draw: impl FnOnce()) {
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

// ---------------------------------------------------------------------------
// Shaders

/// Compiles a shader from WGSL source. See [`Shader`] for what to write.
///
/// Panics with the compiler's message if the source is not valid. Use
/// [`Shader::new`] to handle the failure instead, or [`Shader::from_file`]
/// to load the source from a file.
pub fn new_shader(source: &str) -> Shader {
    Shader::new(source).unwrap_or_else(|error| panic!("{error}"))
}

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

// ---------------------------------------------------------------------------
// Drawing state

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

/// Sets how what is drawn combines with what is already there.
///
/// The blend mode stays set until changed, even across frames.
pub fn set_blend_mode(mode: BlendMode) {
    context::with(|ctx| ctx.gfx.batch.blend = mode);
}

/// Returns the current blend mode.
pub fn blend_mode() -> BlendMode {
    context::with(|ctx| ctx.gfx.batch.blend)
}

/// Limits drawing to a rectangle of the screen or the active canvas.
///
/// The rectangle is in pixels and is not affected by the transform. It
/// stays set until changed or removed with [`clear_scissor`].
pub fn set_scissor(x: i32, y: i32, width: u32, height: u32) {
    context::with(|ctx| {
        ctx.gfx.batch.scissor = Some(Scissor {
            x,
            y,
            width,
            height,
        });
    });
}

/// Removes the scissor so drawing can reach everywhere again.
pub fn clear_scissor() {
    context::with(|ctx| ctx.gfx.batch.scissor = None);
}

/// Returns the scissor rectangle as `(x, y, width, height)`, if one is set.
pub fn scissor() -> Option<(i32, i32, u32, u32)> {
    context::with(|ctx| {
        ctx.gfx
            .batch
            .scissor
            .map(|scissor| (scissor.x, scissor.y, scissor.width, scissor.height))
    })
}

/// Sets the filter that newly created images start with.
///
/// Call this with [`Filter::Nearest`] before loading images in a pixel art
/// game.
pub fn set_default_filter(filter: Filter) {
    context::with(|ctx| ctx.gfx.default_filter = filter);
}

/// Returns the filter that newly created images start with.
pub fn default_filter() -> Filter {
    context::with(|ctx| ctx.gfx.default_filter)
}

// ---------------------------------------------------------------------------
// Transform stack

/// Saves the current transform so [`pop`] can restore it.
pub fn push() {
    context::with(|ctx| {
        let current = ctx.gfx.transform;
        ctx.gfx.stack.push(current);
    });
}

/// Restores the transform saved by the matching [`push`].
///
/// Panics if there is no matching `push`, which is always a bug in the game.
pub fn pop() {
    context::with(|ctx| {
        ctx.gfx.transform = ctx
            .gfx
            .stack
            .pop()
            .expect("graphics::pop was called without a matching graphics::push");
    });
}

/// Moves everything drawn afterwards by `(dx, dy)`.
pub fn translate(dx: f32, dy: f32) {
    context::with(|ctx| ctx.gfx.transform.translate(dx, dy));
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

// ---------------------------------------------------------------------------
// Shapes

/// Draws a rectangle with its top-left corner at `(x, y)`.
pub fn rectangle(mode: DrawMode, x: f32, y: f32, width: f32, height: f32) {
    polygon(
        mode,
        &[
            (x, y),
            (x + width, y),
            (x + width, y + height),
            (x, y + height),
        ],
    );
}

/// Draws a circle centered on `(x, y)`.
pub fn circle(mode: DrawMode, x: f32, y: f32, radius: f32) {
    polygon(mode, &shapes::circle_points(x, y, radius));
}

/// Draws a polygon through `points`. Filling only works for convex polygons.
pub fn polygon(mode: DrawMode, points: &[(f32, f32)]) {
    context::with(|ctx| {
        let triangles = match mode {
            DrawMode::Fill => shapes::fill_convex(points),
            DrawMode::Line => shapes::outline_closed(points, ctx.gfx.line_width),
        };
        ctx.gfx.push_shape(&triangles);
    });
}

/// Draws a straight line between two points.
pub fn line(x1: f32, y1: f32, x2: f32, y2: f32) {
    context::with(|ctx| {
        let triangles = shapes::segment((x1, y1), (x2, y2), ctx.gfx.line_width);
        ctx.gfx.push_shape(&triangles);
    });
}

// ---------------------------------------------------------------------------
// Images

/// Loads an image file (PNG or JPEG).
///
/// A relative path is looked up as [`crate::filesystem`] describes, which
/// normally means inside the game's `assets` folder.
///
/// Panics with the file name if it cannot be loaded, since a missing asset
/// is usually a bug. Use [`try_new_image`] to handle the failure instead.
pub fn new_image(path: impl AsRef<Path>) -> Image {
    try_new_image(path).unwrap_or_else(|error| panic!("{error}"))
}

/// Loads an image file (PNG or JPEG), returning an error if it fails.
pub fn try_new_image(path: impl AsRef<Path>) -> Result<Image, ImageError> {
    image::load(path.as_ref())
}

/// Draws an image with its top-left corner at `(x, y)`.
pub fn draw(image: &Image, x: f32, y: f32) {
    draw_ex(image, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
}

/// Draws an image with the full set of LÖVE's `draw` arguments.
///
/// The image is scaled by `(sx, sy)` and rotated by `r` radians around the
/// point `(ox, oy)` inside it, and that point is placed at `(x, y)`. To spin
/// a sprite around its center, pass half its size as `(ox, oy)`.
#[allow(clippy::too_many_arguments)] // Mirrors love.graphics.draw.
pub fn draw_ex(image: &Image, x: f32, y: f32, r: f32, sx: f32, sy: f32, ox: f32, oy: f32) {
    draw_quad_ex(image, &whole_image(image), x, y, r, sx, sy, ox, oy);
}

/// Draws the part of an image described by `quad` at `(x, y)`.
pub fn draw_quad(image: &Image, quad: &Quad, x: f32, y: f32) {
    draw_quad_ex(image, quad, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
}

/// Draws part of an image with the full set of LÖVE's `draw` arguments.
///
/// See [`draw_ex`] for what the arguments mean. The origin `(ox, oy)` is
/// measured from the top-left corner of the quad.
#[allow(clippy::too_many_arguments)] // Mirrors love.graphics.draw.
pub fn draw_quad_ex(
    image: &Image,
    quad: &Quad,
    x: f32,
    y: f32,
    r: f32,
    sx: f32,
    sy: f32,
    ox: f32,
    oy: f32,
) {
    let local = Transform::from_draw_args(x, y, r, sx, sy, ox, oy);
    context::with(|ctx| ctx.gfx.push_image(image, quad, &local));
}

// ---------------------------------------------------------------------------
// Particles

/// Creates a particle system that draws each particle as `image` and
/// holds at most `max_particles` at once.
pub fn new_particle_system(image: &Image, max_particles: usize) -> ParticleSystem {
    ParticleSystem::new(image, max_particles)
}

/// Draws a particle system with its origin at `(x, y)`.
pub fn draw_particles(system: &ParticleSystem, x: f32, y: f32) {
    draw_particles_ex(system, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
}

/// Draws a particle system with the full set of LÖVE's `draw` arguments.
/// See [`draw_ex`] for what they mean.
#[allow(clippy::too_many_arguments)] // Mirrors love.graphics.draw.
pub fn draw_particles_ex(
    system: &ParticleSystem,
    x: f32,
    y: f32,
    r: f32,
    sx: f32,
    sy: f32,
    ox: f32,
    oy: f32,
) {
    let local = Transform::from_draw_args(x, y, r, sx, sy, ox, oy);
    context::with(|ctx| ctx.gfx.push_particles(system, &local));
}

// ---------------------------------------------------------------------------
// Text

/// Loads a TrueType or OpenType font file at `size` pixels.
///
/// A relative path is looked up as [`crate::filesystem`] describes, which
/// normally means inside the game's `assets` folder.
///
/// Panics with the file name if it cannot be loaded. Use [`try_new_font`]
/// to handle the failure instead.
pub fn new_font(path: impl AsRef<Path>, size: f32) -> Font {
    try_new_font(path, size).unwrap_or_else(|error| panic!("{error}"))
}

/// Loads a font file at `size` pixels, returning an error if it fails.
pub fn try_new_font(path: impl AsRef<Path>, size: f32) -> Result<Font, FontError> {
    let path = path.as_ref();
    let bytes = crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
    Font::from_bytes(&bytes, size)
}

/// Creates the built-in font at `size` pixels.
pub fn new_default_font(size: f32) -> Font {
    Font::default_at(size)
}

/// Sets the font used by [`print()`] and [`printf`].
pub fn set_font(font: &Font) {
    context::with(|ctx| ctx.gfx.font = Some(font.clone()));
}

/// Returns the current font. Until one is set, this is the built-in font
/// at 12 pixels, as in LÖVE.
pub fn font() -> Font {
    context::with(|ctx| {
        let default_filter = ctx.gfx.default_filter;
        ctx.gfx
            .font
            .get_or_insert_with(|| Font::fallback(default_filter))
            .clone()
    })
}

/// Draws text with its top-left corner at `(x, y)` in the current font and
/// color. Line breaks in the text start new lines.
pub fn print(text: &str, x: f32, y: f32) {
    print_ex(text, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
}

/// Draws text with the full set of LÖVE's `print` arguments. See
/// [`draw_ex`] for what they mean.
#[allow(clippy::too_many_arguments)] // Mirrors love.graphics.print.
pub fn print_ex(text: &str, x: f32, y: f32, r: f32, sx: f32, sy: f32, ox: f32, oy: f32) {
    let local = Transform::from_draw_args(x, y, r, sx, sy, ox, oy);
    context::with(|ctx| ctx.gfx.push_text(ctx.backend.as_mut(), text, None, &local));
}

/// Draws text wrapped to `limit` pixels wide and aligned within that
/// width, the equivalent of `love.graphics.printf`.
pub fn printf(text: &str, x: f32, y: f32, limit: f32, align: Align) {
    printf_ex(text, x, y, limit, align, 0.0, 1.0, 1.0, 0.0, 0.0);
}

/// Draws wrapped text with the full set of LÖVE's `printf` arguments.
#[allow(clippy::too_many_arguments)] // Mirrors love.graphics.printf.
pub fn printf_ex(
    text: &str,
    x: f32,
    y: f32,
    limit: f32,
    align: Align,
    r: f32,
    sx: f32,
    sy: f32,
    ox: f32,
    oy: f32,
) {
    let local = Transform::from_draw_args(x, y, r, sx, sy, ox, oy);
    context::with(|ctx| {
        ctx.gfx
            .push_text(ctx.backend.as_mut(), text, Some((limit, align)), &local);
    });
}
