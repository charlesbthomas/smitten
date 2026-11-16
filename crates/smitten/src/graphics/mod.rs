//! Drawing, the equivalent of `love.graphics`.
//!
//! Positions are in pixels from the top-left corner of the window, with y
//! pointing down. Everything drawn is tinted by the current color and moved
//! by the current transform, both of which are set with functions here.
//!
//! Text, canvases and shaders are not implemented yet.

mod batch;
mod image;
mod shapes;
mod transform;

use std::path::Path;

use smitten_core::backend::{Frame, Vertex};

use crate::context;
use crate::{Color, Filter};
use batch::Batch;

pub use image::{Image, ImageError, Quad};
pub use transform::Transform;

/// Whether a shape is filled in or drawn as an outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawMode {
    Fill,
    Line,
}

pub(crate) struct GraphicsState {
    background: Color,
    clear: Color,
    color: Color,
    line_width: f32,
    pub(crate) default_filter: Filter,
    transform: Transform,
    stack: Vec<Transform>,
    batch: Batch,
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            background: Color::BLACK,
            clear: Color::BLACK,
            color: Color::WHITE,
            line_width: 1.0,
            default_filter: Filter::default(),
            transform: Transform::IDENTITY,
            stack: Vec::new(),
            batch: Batch::default(),
        }
    }
}

impl GraphicsState {
    /// Resets what LÖVE resets before each `draw`: the screen and the
    /// transform. The color and line width carry over between frames.
    pub(crate) fn begin_frame(&mut self) {
        self.clear = self.background;
        self.batch.clear();
        self.transform = Transform::IDENTITY;
        self.stack.clear();
    }

    pub(crate) fn frame(&self) -> Frame<'_> {
        Frame {
            clear: self.clear,
            vertices: &self.batch.vertices,
            calls: &self.batch.calls,
        }
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

fn color_array(color: Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn whole_image(image: &Image) -> Quad {
    let (width, height) = (image.width() as f32, image.height() as f32);
    Quad::new(0.0, 0.0, width, height, width, height)
}

// ---------------------------------------------------------------------------
// Screen

/// Clears the screen to `color`, discarding anything drawn so far this frame.
pub fn clear(color: Color) {
    context::with(|ctx| {
        ctx.gfx.clear = color;
        ctx.gfx.batch.clear();
    });
}

/// Sets the color every frame is cleared to before `draw` runs.
pub fn set_background_color(color: Color) {
    context::with(|ctx| ctx.gfx.background = color);
}

/// Returns the current background color.
pub fn background_color() -> Color {
    context::with(|ctx| ctx.gfx.background)
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
