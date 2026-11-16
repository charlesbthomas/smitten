use std::fmt;
use std::ops::Deref;

use super::Image;
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
    image: Image,
}

impl Canvas {
    /// Creates a transparent canvas of `width` by `height` pixels.
    pub fn new(width: u32, height: u32) -> Result<Canvas, String> {
        context::with(|ctx| {
            let texture = ctx.backend.create_render_target(width, height)?;
            Ok(Canvas {
                image: Image::from_texture(texture, width, height, ctx.gfx.default_filter),
            })
        })
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
            .field("filter", &self.image.filter())
            .finish()
    }
}
