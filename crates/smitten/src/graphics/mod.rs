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
mod screen;
mod shader;
mod shapes;
mod sprite_batch;
mod text;
mod transform;

use std::path::Path;

use smitten_core::backend::{Backend, Frame, Scissor, TextureId, Vertex};

use crate::context;
use smitten_core::StencilMode;

use crate::{BlendMode, Color, CompareMode, Filter, StencilAction};
use batch::Batch;
pub(crate) use screen::Screen;

/// What [`crate::reload`] needs from the asset types.
pub(crate) mod reloading {
    pub(crate) use super::image::{Inner as ImageInner, reload as reload_image};
    pub(crate) use super::shader::{Inner as ShaderInner, reload as reload_shader};
    pub(crate) use super::text::{Inner as FontInner, reload as reload_font};
}

pub use canvas::Canvas;
pub use image::{Image, ImageError, Quad};
pub use particles::ParticleSystem;
pub use shader::{Shader, ShaderError, UniformValue};
pub use sprite_batch::SpriteBatch;
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
    /// Reused between draws of large things so they do not allocate.
    scratch: Vec<Vertex>,
    /// How units map onto the window.
    pub(crate) screen: Screen,
    /// Where the game's picture is drawn before it is enlarged to the
    /// window, when the design size asks for pixel-perfect scaling.
    surface: Option<Canvas>,
    /// The scissor the game asked for, in units of the current target.
    scissor: Option<(i32, i32, u32, u32)>,
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
            scratch: Vec::new(),
            screen: Screen::default(),
            surface: None,
            scissor: None,
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
    pub(crate) fn begin_frame(&mut self, backend: &mut dyn Backend) {
        self.screen
            .update(backend.size_in_pixels(), backend.dpi_scale());
        if let Some((width, height)) = self.screen.pixel_perfect()
            && self.surface.is_none()
        {
            // If the GPU refuses, the game draws straight to the window.
            self.surface = Canvas::create(backend, width, height, 1.0, Filter::Nearest).ok();
        }
        if self.surface.is_some() {
            // The window itself only shows the surface, between black bars.
            self.batch.set_target(None);
            self.batch.clear_target(Color::BLACK);
        }
        self.set_canvas(None);
        self.batch.clear_target(self.background);
        self.transform = Transform::IDENTITY;
        self.stack.clear();
    }

    /// Called after `draw`: puts the game's picture in the window when a
    /// design size is in use.
    pub(crate) fn finish_frame(&mut self) {
        if self.screen.design.is_none() {
            return;
        }
        self.batch.set_target(None);
        let quad = |[left, top, right, bottom]: [f32; 4], color: [f32; 4]| {
            let corner = |x: f32, y: f32, u: f32, v: f32| Vertex {
                position: [x, y],
                uv: [u, v],
                color,
            };
            [
                corner(left, top, 0.0, 0.0),
                corner(right, top, 1.0, 0.0),
                corner(right, bottom, 1.0, 1.0),
                corner(left, top, 0.0, 0.0),
                corner(right, bottom, 1.0, 1.0),
                corner(left, bottom, 0.0, 1.0),
            ]
        };
        if let Some(surface) = &self.surface {
            let view = self.screen.view();
            let (width, height) = surface.dimensions();
            let area = [
                view.x,
                view.y,
                view.x + width as f32 * view.scale,
                view.y + height as f32 * view.scale,
            ];
            self.batch.push_plain(
                Some(surface.texture()),
                Filter::Nearest,
                BlendMode::Replace,
                &quad(area, [1.0; 4]),
            );
        } else {
            // The game drew at full resolution and may have spilled past
            // its area, so cover the rest of the window.
            for bar in self.screen.bars() {
                self.batch.push_plain(
                    None,
                    Filter::default(),
                    BlendMode::Replace,
                    &quad(bar, [0.0, 0.0, 0.0, 1.0]),
                );
            }
        }
    }

    /// Puts drawing back to how it starts out, for the error screen: the
    /// game may have panicked half way through a frame, with a canvas, a
    /// shader or a scissor in force. A design size is dropped too, since
    /// it may be too small to read text in.
    ///
    /// Returns what the game left in use. Drop it once the framework state
    /// is no longer borrowed.
    pub(crate) fn reset_for_error_screen(&mut self, background: Color) -> impl Sized + use<> {
        self.batch.clear();
        self.batch.set_shader(None);
        self.batch.blend = BlendMode::default();
        self.batch.stencil = StencilMode::Off;
        self.scissor = None;
        self.screen.design = None;
        let leftovers = (
            self.surface.take(),
            self.canvas.take(),
            self.shader.take(),
            self.font.take(),
        );
        self.set_canvas(None);
        self.background = background;
        self.color = Color::WHITE;
        self.font = Some(Font::fallback_at(
            text::ERROR_FONT_SIZE,
            self.default_filter,
            self.screen.draw_scale(),
        ));
        leftovers
    }

    /// Queues text at `(x, y)` in the current font, for the framework's
    /// own messages.
    pub(crate) fn print_plain(
        &mut self,
        backend: &mut dyn Backend,
        text: &str,
        x: f32,
        y: f32,
        wrap: Option<(f32, Align)>,
    ) {
        let local = Transform::from_draw_args(x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
        self.push_text(backend, text, wrap, &local);
    }

    /// Called once the frame has been handed to the backend.
    pub(crate) fn end_frame(&mut self) {
        self.batch.clear();
        self.set_canvas(None);
    }

    /// How units of the current target become its pixels.
    fn base(&self) -> Transform {
        match &self.canvas {
            Some(canvas) => {
                let mut transform = Transform::IDENTITY;
                transform.scale(canvas.dpi_scale(), canvas.dpi_scale());
                transform
            }
            None if self.surface.is_some() => Transform::IDENTITY,
            None => self.screen.view().transform(),
        }
    }

    /// Pixels per unit on the current target.
    fn target_scale(&self) -> f32 {
        match &self.canvas {
            Some(canvas) => canvas.dpi_scale(),
            None if self.surface.is_some() => 1.0,
            None => self.screen.view().scale,
        }
    }

    /// The game's transform followed by the step from units to pixels.
    fn device(&self) -> Transform {
        self.base().then_inner(&self.transform)
    }

    /// Hands the game's scissor to the batch in pixels of the target.
    fn apply_scissor(&mut self) {
        let base = self.base();
        self.batch.scissor = self.scissor.map(|(x, y, width, height)| {
            let (left, top) = base.apply(x as f32, y as f32);
            let (right, bottom) = base.apply(x as f32 + width as f32, y as f32 + height as f32);
            let (left, top) = (left.round(), top.round());
            Scissor {
                x: left as i32,
                y: top as i32,
                width: (right.round() - left).max(0.0) as u32,
                height: (bottom.round() - top).max(0.0) as u32,
            }
        });
    }

    /// The size in units of what is being drawn to.
    fn target_size(&self) -> (u32, u32) {
        match &self.canvas {
            Some(canvas) => canvas.dimensions(),
            None => self.screen.size(),
        }
    }

    /// The size in pixels of what is being drawn to.
    fn target_pixel_size(&self) -> (u32, u32) {
        match &self.canvas {
            Some(canvas) => canvas.pixel_dimensions(),
            None => self.screen.pixel_size(),
        }
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
        let target = canvas.as_ref().or(self.surface.as_ref());
        self.batch.set_target(target.map(|canvas| canvas.texture()));
        self.canvas = canvas;
        self.apply_scissor();
    }

    /// Queues untextured triangles given in local coordinates.
    fn push_shape(&mut self, triangles: &[(f32, f32)]) {
        let color = color_array(self.color);
        let device = self.device();
        let vertices: Vec<Vertex> = triangles
            .iter()
            .map(|&(x, y)| {
                let (x, y) = device.apply(x, y);
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
        let transform = self.device().then_inner(local);
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
        let transform = self.device().then_inner(local);
        let vertices = system.vertices(&transform, self.color);
        let image = system.image();
        self.batch
            .push(Some(image.texture()), image.filter(), &vertices);
    }

    /// Queues every sprite of `sprites`, placed by `local`.
    fn push_sprite_batch(&mut self, sprites: &SpriteBatch, local: &Transform) {
        let transform = self.device().then_inner(local);
        let mut vertices = std::mem::take(&mut self.scratch);
        vertices.clear();
        sprites.write_vertices(&transform, self.color, &mut vertices);
        let image = sprites.image();
        self.batch
            .push(Some(image.texture()), image.filter(), &vertices);
        self.scratch = vertices;
    }

    /// Queues `text` in the current font, placed by `local`.
    fn push_text(
        &mut self,
        backend: &mut dyn Backend,
        text: &str,
        wrap: Option<(f32, Align)>,
        local: &Transform,
    ) {
        let (default_filter, scale) = (self.default_filter, self.screen.draw_scale());
        let font = self
            .font
            .get_or_insert_with(|| Font::fallback(default_filter, scale))
            .clone();
        let transform = self.device().then_inner(local);
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

/// Size in real pixels of the screen, or of the active canvas, the
/// equivalent of `love.graphics.getPixelDimensions`. Shaders see
/// positions in these pixels.
pub fn pixel_dimensions() -> (u32, u32) {
    context::with(|ctx| ctx.gfx.target_pixel_size())
}

// ---------------------------------------------------------------------------
// Canvases

/// Creates a transparent canvas of `width` by `height` units, as sharp as
/// the screen. See [`Canvas::with_dpi_scale`] to choose its sharpness.
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
/// Positions are then in units from the canvas's top-left corner. The
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
    // More pixels per unit need more corners to look round.
    let scale = context::with(|ctx| ctx.gfx.target_scale());
    polygon(mode, &shapes::circle_points(x, y, radius, scale));
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
// Sprite batches

/// Creates an empty sprite batch that draws `image`, with room reserved
/// for `capacity` sprites. See [`SpriteBatch`].
pub fn new_sprite_batch(image: &Image, capacity: usize) -> SpriteBatch {
    SpriteBatch::new(image, capacity)
}

/// Draws every sprite in a batch, with the batch's origin at `(x, y)`.
pub fn draw_sprite_batch(sprites: &SpriteBatch, x: f32, y: f32) {
    draw_sprite_batch_ex(sprites, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
}

/// Draws a sprite batch with the full set of LÖVE's `draw` arguments,
/// which move, turn and scale the batch as a whole. See [`draw_ex`].
#[allow(clippy::too_many_arguments)] // Mirrors love.graphics.draw.
pub fn draw_sprite_batch_ex(
    sprites: &SpriteBatch,
    x: f32,
    y: f32,
    r: f32,
    sx: f32,
    sy: f32,
    ox: f32,
    oy: f32,
) {
    let local = Transform::from_draw_args(x, y, r, sx, sy, ox, oy);
    context::with(|ctx| ctx.gfx.push_sprite_batch(sprites, &local));
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
    let font = Font::from_bytes(&bytes, size)?;
    font.watch(path);
    Ok(font)
}

/// Loads a bitmap font: an image holding the characters of `glyphs` in a
/// row, separated by columns in the color of its top-left pixel. The
/// equivalent of `love.graphics.newImageFont`. See [`Font::from_image`].
///
/// Pixel fonts usually want `font.set_filter(Filter::Nearest)` so they
/// stay crisp when enlarged.
///
/// Panics with the file name if it cannot be loaded. Use
/// [`try_new_image_font`] to handle the failure instead.
pub fn new_image_font(path: impl AsRef<Path>, glyphs: &str) -> Font {
    try_new_image_font(path, glyphs).unwrap_or_else(|error| panic!("{error}"))
}

/// Loads a bitmap font, returning an error if it fails.
pub fn try_new_image_font(path: impl AsRef<Path>, glyphs: &str) -> Result<Font, FontError> {
    let path = path.as_ref();
    let bytes = crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
    let font = Font::from_image(&bytes, glyphs, 0.0).map_err(|error| match error {
        FontError::Parse(message) => FontError::Parse(format!("{}: {message}", path.display())),
        other => other,
    })?;
    font.watch(path);
    Ok(font)
}

/// Loads a bitmap font: an image holding the characters of `glyphs` in a
/// row, separated by columns in the color of its top-left pixel. The
/// equivalent of `love.graphics.newImageFont`. See [`Font::from_image`].
///
/// Pixel fonts usually want `font.set_filter(Filter::Nearest)` so they
/// stay crisp when enlarged.
///
/// Panics with the file name if it cannot be loaded. Use
/// [`try_new_image_font`] to handle the failure instead.
pub fn new_image_font(path: impl AsRef<Path>, glyphs: &str) -> Font {
    try_new_image_font(path, glyphs).unwrap_or_else(|error| panic!("{error}"))
}

/// Loads a bitmap font, returning an error if it fails.
pub fn try_new_image_font(path: impl AsRef<Path>, glyphs: &str) -> Result<Font, FontError> {
    let path = path.as_ref();
    let bytes = crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
    Font::from_image(&bytes, glyphs, 0.0).map_err(|error| match error {
        FontError::Parse(message) => FontError::Parse(format!("{}: {message}", path.display())),
        other => other,
    })
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
        let (default_filter, scale) = (ctx.gfx.default_filter, ctx.gfx.screen.draw_scale());
        ctx.gfx
            .font
            .get_or_insert_with(|| Font::fallback(default_filter, scale))
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
