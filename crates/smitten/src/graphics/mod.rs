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
mod compressed;
pub(crate) mod image;
mod image_data;
mod layered;
mod mesh;
mod particles;
mod screen;
mod shader;
mod shapes;
mod sprite_batch;
mod state;
mod stats;
mod text;
mod text_object;
mod transform;
#[cfg(feature = "video")]
mod video;

use std::path::Path;

use smitten_core::backend::{Backend, ColorMask, Frame, Pass, Scissor, TextureId, Vertex};

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

pub use canvas::{Canvas, MipmapMode};
pub use compressed::CompressedImageData;
pub use image::{Image, ImageError, ImageSettings, Quad};
pub use image_data::ImageData;
pub use layered::{ArrayImage, CubeImage};
pub use mesh::{Mesh, MeshDrawMode, MeshVertex};
pub use particles::ParticleSystem;
pub use shader::{Shader, ShaderError, Texture, UniformValue};
pub use shapes::{LineJoin, LineStyle};
pub use smitten_core::backend::CompressedFormat;
pub use sprite_batch::SpriteBatch;
pub use state::StackType;
pub use stats::Stats;
pub use text::{Align, Font, FontError};
pub use text_object::Text;
pub use transform::Transform;
#[cfg(feature = "video")]
pub use video::{Video, new_video, try_new_video};

/// Whether a shape is filled in or drawn as an outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawMode {
    Fill,
    Line,
}

/// How the ends of an arc are joined, as LÖVE's `ArcType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ArcType {
    /// Both ends are joined to the center, like a slice of pie.
    #[default]
    Pie,
    /// The ends are left apart. Filled, it looks the same as `Closed`.
    Open,
    /// The ends are joined to each other by a straight line.
    Closed,
}

pub(crate) struct GraphicsState {
    background: Color,
    color: Color,
    line_width: f32,
    line_join: LineJoin,
    line_style: LineStyle,
    point_size: f32,
    pub(crate) default_filter: Filter,
    transform: Transform,
    stack: Vec<state::Saved>,
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
    /// Who wants a copy of the frame being drawn.
    screenshots: Vec<ScreenshotCallback>,
}

/// Called with the finished frame by [`capture_screenshot`].
pub(crate) type ScreenshotCallback = Box<dyn FnOnce(ImageData)>;

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            background: Color::BLACK,
            color: Color::WHITE,
            line_width: 1.0,
            line_join: LineJoin::default(),
            line_style: LineStyle::default(),
            point_size: 1.0,
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
            screenshots: Vec::new(),
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
            self.surface = Canvas::create(
                backend,
                width,
                height,
                1.0,
                Filter::Nearest,
                MipmapMode::None,
            )
            .ok();
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
        self.batch.color_mask = ColorMask::ALL;
        self.batch.wireframe = false;
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

    /// Draws what is queued for canvases now, ahead of the rest of the
    /// frame, so that their pixels are up to date.
    fn flush_canvases(&mut self, backend: &mut dyn Backend) {
        if self.batch.passes.iter().any(|pass| pass.target.is_some()) {
            backend.render_offscreen(&self.frame());
            self.batch.forget_canvas_passes();
        }
    }

    /// Copies a canvas's pixels back from the GPU.
    pub(crate) fn read_canvas(
        &mut self,
        backend: &mut dyn Backend,
        canvas: &Canvas,
    ) -> Result<ImageData, String> {
        self.flush_canvases(backend);
        let (width, height, pixels) = backend.read_texture(canvas.texture())?;
        Ok(ImageData::from_rgba(width, height, pixels))
    }

    /// Called after `finish_frame`: hands a copy of the frame to everyone
    /// who asked for a screenshot. Returns the callbacks to call, each
    /// with its copy, once the framework state is no longer borrowed.
    pub(crate) fn take_screenshots(
        &mut self,
        backend: &mut dyn Backend,
    ) -> Vec<(ScreenshotCallback, ImageData)> {
        if self.screenshots.is_empty() {
            return Vec::new();
        }
        let callbacks = std::mem::take(&mut self.screenshots);
        match self.read_screen(backend) {
            Ok(data) => callbacks
                .into_iter()
                .map(|callback| (callback, data.clone()))
                .collect(),
            Err(message) => {
                eprintln!("smitten: could not take a screenshot: {message}");
                Vec::new()
            }
        }
    }

    /// Draws the queued frame into a texture instead of the window and
    /// copies it back. The window's own pixels cannot be read, and this
    /// way the picture is the same whatever is covering the window.
    fn read_screen(&mut self, backend: &mut dyn Backend) -> Result<ImageData, String> {
        let (width, height) = backend.size_in_pixels();
        let target = backend.create_render_target(width, height)?;
        let passes: Vec<Pass> = self
            .batch
            .passes
            .iter()
            .map(|pass| Pass {
                target: pass.target.or(Some(target)),
                ..*pass
            })
            .collect();
        backend.render_offscreen(&Frame {
            passes: &passes,
            ..self.frame()
        });
        let result = backend.read_texture(target);
        backend.destroy_texture(target);
        // The canvases have had their draws; only the screen's are left
        // for the window.
        self.batch.forget_canvas_passes();
        let (width, height, mut pixels) = result?;
        // The window is opaque whatever was drawn to it.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        Ok(ImageData::from_rgba(width, height, pixels))
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
            textures: &self.batch.textures,
            passes: &self.batch.passes,
        }
    }

    fn set_shader(&mut self, shader: Option<Shader>) {
        match &shader {
            Some(shader) => shader.with_data(|data, textures| {
                self.batch.set_shader(Some((shader.id(), data, textures)))
            }),
            None => self.batch.set_shader(None),
        }
        self.shader = shader;
    }

    /// Picks up new values sent to `shader` if it is the one in use.
    pub(crate) fn refresh_shader(&mut self, shader: &Shader) {
        if self.shader.as_ref().is_some_and(|active| active.is(shader)) {
            shader.with_data(|data, textures| {
                self.batch.set_shader(Some((shader.id(), data, textures)))
            });
        }
    }

    fn set_canvas(&mut self, canvas: Option<Canvas>) {
        let target = canvas.as_ref().or(self.surface.as_ref());
        self.batch.set_target(target.map(|canvas| canvas.texture()));
        self.canvas = canvas;
        self.apply_scissor();
    }

    /// Queues untextured triangles given in local coordinates.
    /// How lines are drawn right now.
    fn stroke(&self) -> shapes::Stroke {
        let fringe = match self.line_style {
            LineStyle::Rough => 0.0,
            LineStyle::Smooth => {
                // The size of one pixel of the target in the game's
                // coordinates, taken from how much the transform enlarges
                // areas.
                let device = self.device();
                let (ox, oy) = device.apply(0.0, 0.0);
                let (ax, ay) = device.apply(1.0, 0.0);
                let (bx, by) = device.apply(0.0, 1.0);
                let area = ((ax - ox) * (by - oy) - (bx - ox) * (ay - oy)).abs();
                if area > 1e-12 { 1.0 / area.sqrt() } else { 0.0 }
            }
        };
        shapes::Stroke {
            width: self.line_width,
            join: self.line_join,
            fringe,
        }
    }

    /// Queues triangles in the current color, faded by each corner's
    /// coverage.
    fn push_covered(&mut self, triangles: &[shapes::Covered]) {
        let color = color_array(self.color);
        let device = self.device();
        let vertices: Vec<Vertex> = triangles
            .iter()
            .map(|&(x, y, coverage)| {
                let (x, y) = device.apply(x, y);
                Vertex {
                    position: [x, y],
                    uv: [0.0, 0.0],
                    color: [color[0], color[1], color[2], color[3] * coverage],
                }
            })
            .collect();
        self.batch.push(None, Filter::default(), &vertices);
    }

    /// Queues a filled shape in the current color. Its texture
    /// coordinates run from 0 to 1 across the box around it, which only
    /// a custom shader can tell.
    fn push_shape(&mut self, triangles: &[(f32, f32)]) {
        let color = color_array(self.color);
        let device = self.device();
        let uv_of = shapes::spread_uv(triangles);
        let vertices: Vec<Vertex> = triangles
            .iter()
            .map(|&(x, y)| {
                let uv = uv_of(x, y);
                let (x, y) = device.apply(x, y);
                Vertex {
                    position: [x, y],
                    uv,
                    color,
                }
            })
            .collect();
        self.batch.push(None, Filter::default(), &vertices);
    }

    /// Queues the part of `image` covered by `quad`, placed by `local`.
    fn push_image(&mut self, image: &Image, quad: &Quad, local: &Transform) {
        self.push_texture(image.texture(), 0, image.filter(), quad, local);
    }

    /// Queues the part of layer `layer` of `texture` covered by `quad`,
    /// placed by `local`.
    fn push_texture(
        &mut self,
        texture: TextureId,
        layer: u32,
        filter: Filter,
        quad: &Quad,
        local: &Transform,
    ) {
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
        self.batch.push_layer(
            Some(texture),
            layer,
            filter,
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

    /// Queues the triangles of `mesh`, placed by `local`.
    fn push_mesh(&mut self, mesh: &Mesh, local: &Transform) {
        let transform = self.device().then_inner(local);
        let mut vertices = std::mem::take(&mut self.scratch);
        vertices.clear();
        mesh.write_vertices(&transform, self.color, &mut vertices);
        match mesh.texture() {
            Some(image) => self
                .batch
                .push(Some(image.texture()), image.filter(), &vertices),
            None => self.batch.push(None, Filter::default(), &vertices),
        }
        self.scratch = vertices;
    }

    /// Queues a laid-out [`Text`], placed by `local`.
    fn push_text_object(&mut self, backend: &mut dyn Backend, text: &Text, local: &Transform) {
        let transform = self.device().then_inner(local);
        let tint = color_array(self.color);
        let filter = text.font().filter();
        let mut vertices = std::mem::take(&mut self.scratch);
        let batch = &mut self.batch;
        text.with_runs(backend, |texture, run| {
            vertices.clear();
            vertices.extend(run.iter().map(|vertex| {
                let (x, y) = transform.apply(vertex.position[0], vertex.position[1]);
                let [r, g, b, a] = vertex.color;
                Vertex {
                    position: [x, y],
                    uv: vertex.uv,
                    color: [r * tint[0], g * tint[1], b * tint[2], a * tint[3]],
                }
            }));
            batch.push(Some(texture), filter, &vertices);
        });
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
        self.push_colored_text(backend, &[(Color::WHITE, text)], wrap, local);
    }

    /// Queues text made of `segments`, each tinted by its color as well as
    /// the current one, in the current font, placed by `local`.
    fn push_colored_text(
        &mut self,
        backend: &mut dyn Backend,
        segments: &[(Color, &str)],
        wrap: Option<(f32, Align)>,
        local: &Transform,
    ) {
        let text = text::joined(segments);
        let mut colors = text::GlyphColors::new(segments);
        let (default_filter, scale) = (self.default_filter, self.screen.draw_scale());
        let font = self
            .font
            .get_or_insert_with(|| Font::fallback(default_filter, scale))
            .clone();
        let transform = self.device().then_inner(local);
        let tint = self.color;
        let filter = font.filter();

        // Glyphs usually share one atlas page, so collect each run of
        // quads on the same page and queue it in one go.
        let mut run: Vec<Vertex> = Vec::with_capacity(text.len() * 6);
        let mut run_texture: Option<TextureId> = None;
        let batch = &mut self.batch;
        font.layout(backend, &text, wrap, |character, texture, rect, uv| {
            let color = color_array(multiply(tint, colors.color(character)));
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

fn multiply(a: Color, b: Color) -> Color {
    Color::rgba(a.r * b.r, a.g * b.g, a.b * b.b, a.a * b.a)
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

/// Takes a picture of the window once the current frame has been drawn,
/// and calls `callback` with it, as LÖVE's `captureScreenshot` does.
///
/// The picture is the whole window in pixels, including any bars around a
/// design size, and is fully opaque. `callback` runs at the end of the
/// frame, after `draw` returns. Called outside `draw`, the picture is of
/// the next frame.
///
/// Reading pixels back waits for the GPU, so this is too slow to do
/// every frame.
///
/// ```no_run
/// use smitten::graphics;
///
/// graphics::capture_screenshot(|picture| {
///     picture.save("screenshot.png").expect("could not save the screenshot");
/// });
/// ```
pub fn capture_screenshot(callback: impl FnOnce(ImageData) + 'static) {
    context::with(|ctx| ctx.gfx.screenshots.push(Box::new(callback)));
}

/// Takes a picture of the window once the current frame has been drawn
/// and writes it as a PNG file in the save directory. A failure to write
/// the file is reported on the terminal. See [`capture_screenshot`].
pub fn save_screenshot(path: impl AsRef<Path>) {
    let path = path.as_ref().to_path_buf();
    capture_screenshot(move |picture| {
        if let Err(error) = picture.save(&path) {
            eprintln!(
                "smitten: could not save the screenshot {}: {error}",
                path.display()
            );
        }
    });
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

/// Sets how wide and tall each point drawn by [`points`] is, in units.
pub fn set_point_size(size: f32) {
    context::with(|ctx| ctx.gfx.point_size = size);
}

/// Returns the current point size.
pub fn point_size() -> f32 {
    context::with(|ctx| ctx.gfx.point_size)
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

/// Limits drawing to the part of the current scissor that is also inside
/// the given rectangle, the equivalent of `love.graphics.intersectScissor`.
/// With no scissor set, it is the same as [`set_scissor`].
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
fn intersect(a: (i32, i32, u32, u32), b: (i32, i32, u32, u32)) -> (i32, i32, u32, u32) {
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
/// out smooth, as in LÖVE. Pixel art usually wants [`LineStyle::Rough`].
///
/// Filled shapes are not affected: their edges are always hard.
pub fn set_line_style(style: LineStyle) {
    context::with(|ctx| ctx.gfx.line_style = style);
}

/// Returns whether the edges of lines and outlines are softened.
pub fn line_style() -> LineStyle {
    context::with(|ctx| ctx.gfx.line_style)
}

/// Chooses which color channels drawing may change, the equivalent of
/// `love.graphics.setColorMask`. A channel left out keeps what is already
/// there. Clearing the screen or a canvas ignores the mask.
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

/// Draws only the edges of every triangle from now on, one pixel wide,
/// to see how shapes, images and text are built. For debugging, as in
/// LÖVE.
pub fn set_wireframe(enabled: bool) {
    context::with(|ctx| ctx.gfx.batch.wireframe = enabled);
}

/// Whether wireframe drawing is on.
pub fn is_wireframe() -> bool {
    context::with(|ctx| ctx.gfx.batch.wireframe)
}

/// What the frame has drawn so far and how many images, canvases and
/// fonts the game holds, the equivalent of `love.graphics.getStats`. Call
/// it at the end of `draw` to see the whole frame.
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

// ---------------------------------------------------------------------------
// Transform stack

/// The most [`push`]es that can wait for a [`pop`] at once, the size of
/// LÖVE's stack.
pub const MAX_STACK_DEPTH: usize = 64;

/// Saves the current transform so [`pop`] can restore it.
///
/// Panics if more than [`MAX_STACK_DEPTH`] pushes are waiting for a
/// [`pop`], which is almost always a missing `pop` in the game.
pub fn push() {
    push_ex(StackType::Transform);
}

/// Saves the transform and every other drawing setting so [`pop`] can
/// restore them, the equivalent of `love.graphics.push("all")`.
///
/// That is the color, the background color, the font, the blend mode,
/// the shader, the canvas, the scissor, the stencil test, the line width,
/// join and style, the point size, the color mask, wireframe drawing and
/// the default filter. A helper can then change any of them without the
/// caller having to put them back.
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
pub fn push_all() {
    push_ex(StackType::All);
}

/// Saves what `kind` says, for [`pop`] to restore: [`push`] and
/// [`push_all`] in one function, as LÖVE's `push(stacktype)`.
pub fn push_ex(kind: StackType) {
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

/// How many [`push`]es are waiting for a [`pop`], the equivalent of
/// `love.graphics.getStackDepth`. It is 0 at the start of every `draw`.
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
pub fn reset() {
    context::with(|ctx| ctx.gfx.reset_state());
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

/// Shears everything drawn afterwards: x gains `kx` times y, and y gains
/// `ky` times x.
pub fn shear(kx: f32, ky: f32) {
    context::with(|ctx| ctx.gfx.transform.shear(kx, ky));
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
pub fn transform_point(x: f32, y: f32) -> (f32, f32) {
    context::with(|ctx| ctx.gfx.transform.transform_point(x, y))
}

/// Which point the current transform puts at `(x, y)` on the screen, such
/// as where the mouse is in a scrolled and zoomed world.
pub fn inverse_transform_point(x: f32, y: f32) -> (f32, f32) {
    context::with(|ctx| ctx.gfx.transform.inverse_transform_point(x, y))
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
///
/// Under a custom shader, a filled shape's `uv` runs from `(0, 0)` at the
/// top-left of the box around the shape to `(1, 1)` at its bottom-right,
/// as it does across an image. So a shader can paint a rectangle, circle
/// or any other filled shape with no image at all. This holds for every
/// filled shape, not only polygons.
pub fn polygon(mode: DrawMode, points: &[(f32, f32)]) {
    context::with(|ctx| {
        let triangles = match mode {
            DrawMode::Fill => shapes::fill_convex(points),
            DrawMode::Line => {
                let triangles = shapes::stroke(points, true, &ctx.gfx.stroke());
                return ctx.gfx.push_covered(&triangles);
            }
        };
        ctx.gfx.push_shape(&triangles);
    });
}

/// Draws a rectangle with rounded corners. `rx` and `ry` are the corner
/// radii along each axis, as in the long form of `love.graphics.rectangle`.
pub fn rounded_rectangle(
    mode: DrawMode,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    rx: f32,
    ry: f32,
) {
    let scale = context::with(|ctx| ctx.gfx.target_scale());
    polygon(
        mode,
        &shapes::rounded_rectangle_points(x, y, width, height, rx, ry, scale),
    );
}

/// Draws an ellipse centered on `(x, y)` with radii `rx` and `ry`.
pub fn ellipse(mode: DrawMode, x: f32, y: f32, rx: f32, ry: f32) {
    let scale = context::with(|ctx| ctx.gfx.target_scale());
    polygon(mode, &shapes::ellipse_points(x, y, rx, ry, scale));
}

/// Draws part of a circle centered on `(x, y)`, from `angle1` to `angle2`
/// in radians. Angles grow clockwise from the right, as in LÖVE.
pub fn arc(
    mode: DrawMode,
    arc_type: ArcType,
    x: f32,
    y: f32,
    radius: f32,
    angle1: f32,
    angle2: f32,
) {
    context::with(|ctx| {
        let scale = ctx.gfx.target_scale();
        let mut path = shapes::arc_points(x, y, radius, radius, angle1, angle2, scale);
        if arc_type == ArcType::Pie {
            // The center comes first so that filling fans out from it.
            path.insert(0, (x, y));
        }
        match mode {
            DrawMode::Fill => ctx.gfx.push_shape(&shapes::fill_convex(&path)),
            DrawMode::Line => {
                let closed = arc_type != ArcType::Open;
                let triangles = shapes::stroke(&path, closed, &ctx.gfx.stroke());
                ctx.gfx.push_covered(&triangles);
            }
        }
    });
}

/// Draws a square of the current point size centered on each point.
pub fn points(points: &[(f32, f32)]) {
    context::with(|ctx| {
        let half = ctx.gfx.point_size / 2.0;
        let mut triangles = Vec::with_capacity(points.len() * 6);
        for &(x, y) in points {
            let (left, top, right, bottom) = (x - half, y - half, x + half, y + half);
            triangles.extend_from_slice(&[
                (left, top),
                (right, top),
                (right, bottom),
                (left, top),
                (right, bottom),
                (left, bottom),
            ]);
        }
        ctx.gfx.push_shape(&triangles);
    });
}

/// Draws the shadow that a convex shape casts away from a light at
/// `(light_x, light_y)`, in the current color: every edge of `polygon`
/// that faces away from the light, stretched `length` further from it.
///
/// The shape itself is not covered, as if light falls on its top. The
/// corners may be listed clockwise or counter-clockwise. A `length`
/// longer than the screen is wide gives shadows that run off its edge.
///
/// For lighting, draw the shadows of everything solid inside
/// [`stencil`], then draw the light's glow only where the stencil is
/// still zero. The `lighting` example does exactly this.
///
/// LÖVE has no such call; games there work the corners out by hand.
pub fn shadow(light_x: f32, light_y: f32, polygon: &[(f32, f32)], length: f32) {
    context::with(|ctx| {
        let triangles = shapes::shadow((light_x, light_y), polygon, length);
        ctx.gfx.push_shape(&triangles);
    });
}

/// Draws connected lines through `points` in order, with joined corners.
/// The equivalent of `love.graphics.line` with more than two points.
pub fn polyline(points: &[(f32, f32)]) {
    context::with(|ctx| {
        let triangles = shapes::stroke(points, false, &ctx.gfx.stroke());
        ctx.gfx.push_covered(&triangles);
    });
}

/// Draws a dashed line through `points` in order: dashes `dash` long
/// with gaps of `gap` between them, both measured along the line. The
/// dashes use the current line width, join and style.
///
/// LÖVE has no dashed lines; this saves working the pieces out by hand.
pub fn dashed_polyline(points: &[(f32, f32)], dash: f32, gap: f32) {
    context::with(|ctx| {
        let stroke = ctx.gfx.stroke();
        let mut triangles = Vec::new();
        for piece in shapes::dashes(points, dash, gap) {
            triangles.extend(shapes::stroke(&piece, false, &stroke));
        }
        ctx.gfx.push_covered(&triangles);
    });
}

/// Draws a straight dashed line between two points. See
/// [`dashed_polyline`].
pub fn dashed_line(x1: f32, y1: f32, x2: f32, y2: f32, dash: f32, gap: f32) {
    dashed_polyline(&[(x1, y1), (x2, y2)], dash, gap);
}

/// Draws a straight line between two points.
pub fn line(x1: f32, y1: f32, x2: f32, y2: f32) {
    context::with(|ctx| {
        let triangles = shapes::stroke(&[(x1, y1), (x2, y2)], false, &ctx.gfx.stroke());
        ctx.gfx.push_covered(&triangles);
    });
}

// ---------------------------------------------------------------------------
// Images

/// Loads an image file: PNG, JPEG, or DDS for compressed pixels.
///
/// A relative path is looked up as [`crate::filesystem`] describes, which
/// normally means inside the game's `assets` folder.
///
/// Panics with the file name if it cannot be loaded, since a missing asset
/// is usually a bug. Use [`try_new_image`] to handle the failure instead.
pub fn new_image(path: impl AsRef<Path>) -> Image {
    try_new_image(path).unwrap_or_else(|error| panic!("{error}"))
}

/// Loads an image file (PNG, JPEG, or DDS for compressed pixels),
/// returning an error if it fails.
pub fn try_new_image(path: impl AsRef<Path>) -> Result<Image, ImageError> {
    image::load(path.as_ref(), ImageSettings::default())
}

/// Loads an image file with options such as mipmaps, the equivalent of
/// `love.graphics.newImage(path, settings)`.
///
/// Panics with the file name if it cannot be loaded. Use
/// [`try_new_image_with`] to handle the failure instead.
pub fn new_image_with(path: impl AsRef<Path>, settings: ImageSettings) -> Image {
    try_new_image_with(path, settings).unwrap_or_else(|error| panic!("{error}"))
}

/// Loads an image file with options, returning an error if it fails.
pub fn try_new_image_with(
    path: impl AsRef<Path>,
    settings: ImageSettings,
) -> Result<Image, ImageError> {
    image::load(path.as_ref(), settings)
}

/// Loads an image file into memory, where its pixels can be read and
/// changed. See [`ImageData`].
///
/// Panics with the file name if it cannot be loaded. Use
/// [`try_new_image_data`] to handle the failure instead.
pub fn new_image_data(path: impl AsRef<Path>) -> ImageData {
    try_new_image_data(path).unwrap_or_else(|error| panic!("{error}"))
}

/// Loads an image file into memory, returning an error if it fails.
pub fn try_new_image_data(path: impl AsRef<Path>) -> Result<ImageData, ImageError> {
    let (width, height, rgba) = image::load_rgba(path.as_ref())?;
    Ok(ImageData::from_rgba(width, height, rgba))
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

/// Draws a video's current picture at `(x, y)`, the equivalent of
/// `love.graphics.draw(video, x, y)`. Drawing moves the picture on to
/// the video's position.
#[cfg(feature = "video")]
pub fn draw_video(video: &Video, x: f32, y: f32) {
    draw_video_ex(video, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
}

/// Draws a video with the full set of `draw` arguments, as [`draw_ex`]
/// describes.
#[cfg(feature = "video")]
#[allow(clippy::too_many_arguments)] // Mirrors love.graphics.draw.
pub fn draw_video_ex(video: &Video, x: f32, y: f32, r: f32, sx: f32, sy: f32, ox: f32, oy: f32) {
    draw_ex(video.image(), x, y, r, sx, sy, ox, oy);
}

/// Draws an image placed by `transform` instead of a position, rotation
/// and scale, the equivalent of `love.graphics.draw(image, transform)`.
pub fn draw_with(image: &Image, transform: &Transform) {
    let quad = whole_image(image);
    context::with(|ctx| ctx.gfx.push_image(image, &quad, transform));
}

/// Draws part of an image placed by `transform`.
pub fn draw_quad_with(image: &Image, quad: &Quad, transform: &Transform) {
    context::with(|ctx| ctx.gfx.push_image(image, quad, transform));
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
// Meshes

/// Creates a mesh from its vertices. See [`Mesh`].
pub fn new_mesh(vertices: &[MeshVertex], mode: MeshDrawMode) -> Mesh {
    Mesh::new(vertices, mode)
}

/// Draws a mesh with its origin at `(x, y)`, tinted by the current color.
pub fn draw_mesh(mesh: &Mesh, x: f32, y: f32) {
    draw_mesh_ex(mesh, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
}

/// Draws a mesh with the full set of LÖVE's `draw` arguments. See
/// [`draw_ex`].
#[allow(clippy::too_many_arguments)] // Mirrors love.graphics.draw.
pub fn draw_mesh_ex(mesh: &Mesh, x: f32, y: f32, r: f32, sx: f32, sy: f32, ox: f32, oy: f32) {
    let local = Transform::from_draw_args(x, y, r, sx, sy, ox, oy);
    context::with(|ctx| ctx.gfx.push_mesh(mesh, &local));
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

/// Draws text placed by `transform` instead of a position, rotation and
/// scale.
pub fn print_with(text: &str, transform: &Transform) {
    context::with(|ctx| {
        ctx.gfx
            .push_text(ctx.backend.as_mut(), text, None, transform)
    });
}

/// Draws wrapped text placed by `transform`.
pub fn printf_with(text: &str, limit: f32, align: Align, transform: &Transform) {
    context::with(|ctx| {
        ctx.gfx
            .push_text(ctx.backend.as_mut(), text, Some((limit, align)), transform);
    });
}

/// Makes a [`Text`] in `font` holding `text`, the equivalent of
/// `love.graphics.newText`. Pass an empty string to start with nothing.
pub fn new_text(font: &Font, text: &str) -> Text {
    Text::new(font, text)
}

/// Draws a [`Text`] with its top-left corner at `(x, y)` in the current
/// color.
pub fn draw_text(text: &Text, x: f32, y: f32) {
    draw_text_ex(text, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
}

/// Draws a [`Text`] with the full set of LÖVE's `draw` arguments. See
/// [`draw_ex`] for what they mean.
#[allow(clippy::too_many_arguments)] // Mirrors love.graphics.draw.
pub fn draw_text_ex(text: &Text, x: f32, y: f32, r: f32, sx: f32, sy: f32, ox: f32, oy: f32) {
    draw_text_with(text, &Transform::from_draw_args(x, y, r, sx, sy, ox, oy));
}

/// Draws a [`Text`] placed by `transform`.
pub fn draw_text_with(text: &Text, transform: &Transform) {
    context::with(|ctx| {
        ctx.gfx
            .push_text_object(ctx.backend.as_mut(), text, transform)
    });
}

/// Draws layer `layer` of an [`ArrayImage`], counting from 0, with its
/// top-left corner at `(x, y)`. The equivalent of
/// `love.graphics.drawLayer`.
///
/// Panics if there is no such layer, or if a custom shader is active: a
/// shader reads an array image by declaring a `texture_2d_array<f32>` and
/// being sent it with [`Shader::send_texture`].
pub fn draw_layer(image: &ArrayImage, layer: u32, x: f32, y: f32) {
    draw_layer_ex(image, layer, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
}

/// Draws a layer of an [`ArrayImage`] with the full set of LÖVE's `draw`
/// arguments. See [`draw_ex`] for what they mean.
#[allow(clippy::too_many_arguments)] // Mirrors love.graphics.drawLayer.
pub fn draw_layer_ex(
    image: &ArrayImage,
    layer: u32,
    x: f32,
    y: f32,
    r: f32,
    sx: f32,
    sy: f32,
    ox: f32,
    oy: f32,
) {
    let (width, height) = (image.width() as f32, image.height() as f32);
    let quad = Quad::new(0.0, 0.0, width, height, width, height);
    draw_layer_quad_with(
        image,
        layer,
        &quad,
        &Transform::from_draw_args(x, y, r, sx, sy, ox, oy),
    );
}

/// Draws part of a layer of an [`ArrayImage`], placed by `transform`.
pub fn draw_layer_quad_with(image: &ArrayImage, layer: u32, quad: &Quad, transform: &Transform) {
    assert!(
        layer < image.layer_count(),
        "layer {layer} does not exist: the array image has {} layers",
        image.layer_count()
    );
    context::with(|ctx| {
        assert!(
            ctx.gfx.shader.is_none(),
            "graphics::draw_layer cannot be used with a custom shader active; \
             send the array image to the shader with Shader::send_texture instead"
        );
        ctx.gfx
            .push_texture(image.texture(), layer, image.filter(), quad, transform);
    });
}

/// Loads an image file for each layer of an [`ArrayImage`], the
/// equivalent of `love.graphics.newArrayImage`. They must all be the same
/// size.
///
/// Panics with the reason if it cannot be made. Use
/// [`ArrayImage::from_files`] to handle the failure instead.
pub fn new_array_image(paths: &[impl AsRef<Path>]) -> ArrayImage {
    ArrayImage::from_files(paths).unwrap_or_else(|error| panic!("{error}"))
}

/// Loads the six faces of a [`CubeImage`] from image files, in the order
/// +x, -x, +y, -y, +z, -z. The equivalent of `love.graphics.newCubeImage`
/// with six files.
///
/// Panics with the reason if it cannot be made. Use
/// [`CubeImage::from_files`] to handle the failure instead.
pub fn new_cube_image(paths: &[impl AsRef<Path>; 6]) -> CubeImage {
    CubeImage::from_files(paths).unwrap_or_else(|error| panic!("{error}"))
}

/// Loads a DDS file of compressed pixels, the equivalent of
/// `love.image.newCompressedData`. Draw it with
/// [`Image::from_compressed_data`], or load it straight into an image
/// with [`new_image`].
///
/// Panics with the file name if it cannot be loaded. Use
/// [`CompressedImageData::from_file`] to handle the failure instead.
pub fn new_compressed_data(path: impl AsRef<Path>) -> CompressedImageData {
    CompressedImageData::from_file(path).unwrap_or_else(|error| panic!("{error}"))
}

/// Whether the GPU can draw images compressed in `format`, the
/// equivalent of `love.graphics.getImageFormats`.
pub fn is_compressed_format_supported(format: CompressedFormat) -> bool {
    context::with(|ctx| ctx.backend.supports_compressed_format(format))
}

/// Loads a [`CubeImage`] from one image file holding all six faces, the
/// equivalent of `love.graphics.newCubeImage(path)`. The layouts it takes
/// are listed at [`CubeImage::from_layout`].
///
/// Panics with the reason if it cannot be made. Use
/// [`CubeImage::from_layout_file`] to handle the failure instead.
pub fn new_cube_image_from_layout(path: impl AsRef<Path>) -> CubeImage {
    CubeImage::from_layout_file(path).unwrap_or_else(|error| panic!("{error}"))
}

/// Draws text made of colored parts, the equivalent of
/// `love.graphics.print(coloredtext, x, y)`: each part is tinted by its
/// color as well as the current one.
///
/// ```no_run
/// use smitten::{Color, graphics};
///
/// graphics::print_colored(&[(Color::WHITE, "Score: "), (Color::rgb(1.0, 0.8, 0.2), "120")], 10.0, 10.0);
/// ```
pub fn print_colored(segments: &[(Color, &str)], x: f32, y: f32) {
    let local = Transform::from_draw_args(x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
    print_colored_with(segments, &local);
}

/// Draws colored text placed by `transform`.
pub fn print_colored_with(segments: &[(Color, &str)], transform: &Transform) {
    context::with(|ctx| {
        ctx.gfx
            .push_colored_text(ctx.backend.as_mut(), segments, None, transform);
    });
}

/// Draws text made of colored parts, wrapped to `limit` units and aligned
/// within that width, the equivalent of `love.graphics.printf(coloredtext,
/// ...)`.
pub fn printf_colored(segments: &[(Color, &str)], x: f32, y: f32, limit: f32, align: Align) {
    let local = Transform::from_draw_args(x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
    printf_colored_with(segments, limit, align, &local);
}

/// Draws wrapped colored text placed by `transform`.
pub fn printf_colored_with(
    segments: &[(Color, &str)],
    limit: f32,
    align: Align,
    transform: &Transform,
) {
    context::with(|ctx| {
        ctx.gfx.push_colored_text(
            ctx.backend.as_mut(),
            segments,
            Some((limit, align)),
            transform,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::intersect;

    #[test]
    fn scissors_intersect_to_their_overlap() {
        assert_eq!(
            intersect((0, 0, 100, 100), (50, 20, 100, 30)),
            (50, 20, 50, 30)
        );
        assert_eq!(
            intersect((10, 10, 20, 20), (0, 0, 100, 100)),
            (10, 10, 20, 20)
        );
    }

    #[test]
    fn scissors_that_do_not_meet_leave_nothing() {
        let (_, _, width, height) = intersect((0, 0, 10, 10), (20, 20, 10, 10));
        assert_eq!((width, height), (0, 0));
    }
}
