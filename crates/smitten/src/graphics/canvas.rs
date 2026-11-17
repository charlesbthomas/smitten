use std::fmt;
use std::ops::Deref;

use smitten_core::Filter;
use smitten_core::backend::Backend;

use super::stats::Kind;
use super::{Image, ImageData};
use crate::context;

/// An off-screen image that can be drawn to, the equivalent of LÖVE's
/// `Canvas`.
///
/// Draw to it between [`set_canvas`](super::set_canvas) calls or inside
/// [`Canvas::render_to`], then draw it like an [`Image`]: every `Image`
/// method and every function that takes `&Image` works on a canvas too.
///
/// A canvas keeps what was drawn to it until it is cleared, so it only
/// needs redrawing when its contents change. It starts out transparent.
///
/// Colors drawn to a canvas are stored multiplied by their alpha. A canvas
/// with see-through parts should be drawn with
/// [`BlendMode::Premultiplied`](crate::BlendMode::Premultiplied), or its
/// soft edges come out too dark. This is the same rule as in LÖVE.
///
/// Cloning is cheap and gives another handle to the same canvas.
#[derive(Clone)]
pub struct Canvas {
    /// Sized in units, so drawing the canvas covers the area it was
    /// created for whatever its sharpness.
    image: Image,
    pixels: (u32, u32),
    scale: f32,
}

impl Canvas {
    /// Creates a transparent canvas of `width` by `height` units.
    ///
    /// It is made as sharp as the screen, so on a high-DPI display it
    /// holds more pixels than units, as in LÖVE. Use
    /// [`Canvas::with_dpi_scale`] to choose the sharpness instead.
    pub fn new(width: u32, height: u32) -> Result<Canvas, String> {
        let scale = context::with(|ctx| ctx.gfx.screen.draw_scale());
        Canvas::with_dpi_scale(width, height, scale)
    }

    /// Creates a transparent canvas of `width` by `height` units that
    /// holds `dpi_scale` pixels for each unit in each direction. A scale
    /// of 1.0 gives exactly `width` by `height` pixels.
    pub fn with_dpi_scale(width: u32, height: u32, dpi_scale: f32) -> Result<Canvas, String> {
        if !(dpi_scale.is_finite() && dpi_scale > 0.0) {
            return Err(format!("the DPI scale must be positive, got {dpi_scale}"));
        }
        context::with(|ctx| {
            Canvas::create(
                &mut *ctx.backend,
                width,
                height,
                dpi_scale,
                ctx.gfx.default_filter,
            )
        })
    }

    pub(crate) fn create(
        backend: &mut dyn Backend,
        width: u32,
        height: u32,
        scale: f32,
        filter: Filter,
    ) -> Result<Canvas, String> {
        let pixels = (
            (width as f32 * scale).ceil() as u32,
            (height as f32 * scale).ceil() as u32,
        );
        let texture = backend.create_render_target(pixels.0, pixels.1)?;
        Ok(Canvas {
            image: Image::from_texture(texture, width, height, filter, Kind::Canvas),
            pixels,
            scale,
        })
    }

    /// How many pixels the canvas holds for each unit in each direction.
    pub fn dpi_scale(&self) -> f32 {
        self.scale
    }

    /// The canvas's size in pixels as `(width, height)`.
    /// [`Image::dimensions`] gives its size in units.
    pub fn pixel_dimensions(&self) -> (u32, u32) {
        self.pixels
    }

    /// Copies the canvas's pixels back from the GPU, as LÖVE's
    /// `Canvas:newImageData` does. The result is
    /// [`pixel_dimensions`](Canvas::pixel_dimensions) in size.
    ///
    /// Everything drawn to the canvas so far is included. Colors come back
    /// as the canvas stores them, multiplied by their alpha, so they only
    /// differ from what was drawn where the canvas is see-through.
    ///
    /// This waits for the GPU, so it is too slow to do every frame. It
    /// also draws what is queued for every canvas straight away: avoid it
    /// between drawing a canvas to the screen and changing that canvas
    /// later in the same frame, or the screen shows the later contents.
    pub fn new_image_data(&self) -> Result<ImageData, String> {
        context::with(|ctx| ctx.gfx.read_canvas(&mut *ctx.backend, self))
    }

    /// Runs `draw` with this canvas as the target, then goes back to
    /// whatever was the target before.
    pub fn render_to(&self, draw: impl FnOnce()) {
        let previous = super::canvas();
        super::set_canvas(Some(self));
        draw();
        super::set_canvas(previous.as_ref());
    }
}

impl Deref for Canvas {
    type Target = Image;

    fn deref(&self) -> &Image {
        &self.image
    }
}

impl fmt::Debug for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Canvas")
            .field("width", &self.image.width())
            .field("height", &self.image.height())
            .field("dpi_scale", &self.scale)
            .field("filter", &self.image.filter())
            .finish()
    }
}
