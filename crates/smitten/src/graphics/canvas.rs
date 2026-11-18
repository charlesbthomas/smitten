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
    mipmaps: MipmapMode,
    /// How many samples each pixel holds, 1 for none.
    msaa: u32,
}

/// Whether a canvas has mipmaps and when they are made, the equivalent of
/// the `mipmaps` setting of LÖVE's `newCanvas`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MipmapMode {
    /// No mipmaps, as a canvas starts out.
    #[default]
    None,
    /// Mipmaps made again every frame the canvas is drawn to.
    Auto,
    /// Mipmaps made only when [`Canvas::generate_mipmaps`] is called.
    Manual,
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
                MipmapMode::None,
            )
        })
    }

    /// Creates a transparent canvas of `width` by `height` units, as sharp
    /// as the screen, with mipmaps: smaller copies of what is drawn to it,
    /// used when the canvas is drawn small so it does not shimmer. See
    /// [`MipmapMode`] for when they are made, and
    /// [`Image::set_mipmap_filter`] for how they blend.
    pub fn with_mipmaps(width: u32, height: u32, mode: MipmapMode) -> Result<Canvas, String> {
        context::with(|ctx| {
            Canvas::create(
                &mut *ctx.backend,
                width,
                height,
                ctx.gfx.screen.draw_scale(),
                ctx.gfx.default_filter,
                mode,
            )
        })
    }

    /// Creates a transparent canvas of `width` by `height` units, as sharp
    /// as the screen, that holds `samples` samples per pixel, the `msaa`
    /// setting of LÖVE's `newCanvas`. Shapes and images drawn to it get
    /// smooth edges; 4 is a common choice.
    ///
    /// The GPU may give fewer samples than asked for; [`Canvas::msaa`]
    /// says how many. A multisampled canvas has no mipmaps.
    pub fn with_msaa(width: u32, height: u32, samples: u32) -> Result<Canvas, String> {
        context::with(|ctx| {
            let scale = ctx.gfx.screen.draw_scale();
            let pixels = Canvas::pixels_for(width, height, scale);
            let texture = ctx
                .backend
                .create_msaa_render_target(pixels.0, pixels.1, samples)?;
            let msaa = ctx.backend.texture_msaa(texture);
            let image =
                Image::from_texture(texture, width, height, ctx.gfx.default_filter, Kind::Canvas);
            Ok(Canvas {
                image,
                pixels,
                scale,
                mipmaps: MipmapMode::None,
                msaa,
            })
        })
    }

    /// How many samples each pixel of the canvas holds, 1 if it is not
    /// multisampled, the equivalent of `Canvas:getMSAA`.
    pub fn msaa(&self) -> u32 {
        self.msaa
    }

    fn pixels_for(width: u32, height: u32, scale: f32) -> (u32, u32) {
        (
            (width as f32 * scale).ceil() as u32,
            (height as f32 * scale).ceil() as u32,
        )
    }

    pub(crate) fn create(
        backend: &mut dyn Backend,
        width: u32,
        height: u32,
        scale: f32,
        filter: Filter,
        mipmaps: MipmapMode,
    ) -> Result<Canvas, String> {
        let pixels = Canvas::pixels_for(width, height, scale);
        let texture = match mipmaps {
            MipmapMode::None => backend.create_render_target(pixels.0, pixels.1)?,
            MipmapMode::Auto => backend.create_mipmapped_render_target(pixels.0, pixels.1, true)?,
            MipmapMode::Manual => {
                backend.create_mipmapped_render_target(pixels.0, pixels.1, false)?
            }
        };
        let image = Image::from_texture(texture, width, height, filter, Kind::Canvas);
        if mipmaps != MipmapMode::None {
            image.mark_mipmapped(32 - pixels.0.max(pixels.1).max(1).leading_zeros());
        }
        Ok(Canvas {
            image,
            pixels,
            scale,
            mipmaps,
            msaa: 1,
        })
    }

    /// Whether the canvas has mipmaps and when they are made.
    pub fn mipmap_mode(&self) -> MipmapMode {
        self.mipmaps
    }

    /// Makes the canvas's mipmaps again from what has been drawn to it so
    /// far, the equivalent of `Canvas:generateMipmaps`. Only needed with
    /// [`MipmapMode::Manual`].
    ///
    /// Like [`Canvas::new_image_data`], this draws what is queued for every
    /// canvas straight away.
    pub fn generate_mipmaps(&self) -> Result<(), String> {
        if self.mipmaps == MipmapMode::None {
            return Err("the canvas was made without mipmaps".to_string());
        }
        context::with(|ctx| {
            ctx.gfx.flush_canvases(&mut *ctx.backend);
            ctx.backend.generate_mipmaps(self.texture())
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
