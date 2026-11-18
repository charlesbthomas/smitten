use std::fmt;
use std::ops::Deref;

use smitten_core::backend::{Backend, RenderTargetDesc};
use smitten_core::{Filter, PixelFormat};

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

/// Options for making a canvas, the equivalent of the settings table
/// LÖVE's `newCanvas` takes. Start from the default and change what is
/// needed:
///
/// ```no_run
/// # use smitten::graphics::{Canvas, CanvasSettings};
/// # use smitten::PixelFormat;
/// let hdr = Canvas::with_settings(320, 180, CanvasSettings {
///     format: Some(PixelFormat::Rgba16f),
///     ..CanvasSettings::default()
/// });
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasSettings {
    /// How many pixels the canvas holds for each unit in each direction.
    /// `None` makes it as sharp as the screen.
    pub dpi_scale: Option<f32>,
    /// How the canvas stores its pixels. `None` is LÖVE's `normal`:
    /// [`PixelFormat::Rgba8`], or [`PixelFormat::Srgba8`] with
    /// gamma-correct rendering. A floating-point format such as
    /// [`PixelFormat::Rgba16f`] holds colors brighter than white, for HDR
    /// effects. [`graphics::canvas_formats`](super::canvas_formats) lists
    /// what the GPU can draw to.
    pub format: Option<PixelFormat>,
    /// Whether the canvas has mipmaps and when they are made.
    pub mipmaps: MipmapMode,
    /// How many samples each pixel holds; 0 or 1 for no multisampling.
    pub msaa: u32,
    /// Whether the canvas will be drawn or sampled, as well as drawn to.
    /// `None` means yes. LÖVE draws a canvas made with `Some(false)` faster
    /// on some GPUs; smitten records the setting for
    /// [`Image::is_readable`] but draws it the same way.
    pub readable: Option<bool>,
}

impl Default for CanvasSettings {
    fn default() -> Self {
        Self {
            dpi_scale: None,
            format: None,
            mipmaps: MipmapMode::None,
            msaa: 0,
            readable: None,
        }
    }
}

impl Canvas {
    /// Creates a transparent canvas of `width` by `height` units.
    ///
    /// It is made as sharp as the screen, so on a high-DPI display it
    /// holds more pixels than units, as in LÖVE. Use
    /// [`Canvas::with_dpi_scale`] to choose the sharpness instead.
    pub fn new(width: u32, height: u32) -> Result<Canvas, String> {
        Canvas::with_settings(width, height, CanvasSettings::default())
    }

    /// Creates a transparent canvas of `width` by `height` units that
    /// holds `dpi_scale` pixels for each unit in each direction. A scale
    /// of 1.0 gives exactly `width` by `height` pixels.
    pub fn with_dpi_scale(width: u32, height: u32, dpi_scale: f32) -> Result<Canvas, String> {
        Canvas::with_settings(
            width,
            height,
            CanvasSettings {
                dpi_scale: Some(dpi_scale),
                ..CanvasSettings::default()
            },
        )
    }

    /// Creates a transparent canvas of `width` by `height` units, as sharp
    /// as the screen, with mipmaps: smaller copies of what is drawn to it,
    /// used when the canvas is drawn small so it does not shimmer. See
    /// [`MipmapMode`] for when they are made, and
    /// [`Image::set_mipmap_filter`] for how they blend.
    pub fn with_mipmaps(width: u32, height: u32, mode: MipmapMode) -> Result<Canvas, String> {
        Canvas::with_settings(
            width,
            height,
            CanvasSettings {
                mipmaps: mode,
                ..CanvasSettings::default()
            },
        )
    }

    /// Creates a transparent canvas of `width` by `height` units, as sharp
    /// as the screen, that holds `samples` samples per pixel, the `msaa`
    /// setting of LÖVE's `newCanvas`. Shapes and images drawn to it get
    /// smooth edges; 4 is a common choice.
    ///
    /// The GPU may give fewer samples than asked for; [`Canvas::msaa`]
    /// says how many. A multisampled canvas has no mipmaps.
    pub fn with_msaa(width: u32, height: u32, samples: u32) -> Result<Canvas, String> {
        Canvas::with_settings(
            width,
            height,
            CanvasSettings {
                msaa: samples,
                ..CanvasSettings::default()
            },
        )
    }

    /// Creates a transparent canvas of `width` by `height` units as
    /// `settings` describe, the equivalent of
    /// `love.graphics.newCanvas(width, height, settings)`.
    ///
    /// Fails for a depth format, which is not yet supported, for a format
    /// the GPU cannot draw to, and for multisampling with mipmaps.
    pub fn with_settings(
        width: u32,
        height: u32,
        settings: CanvasSettings,
    ) -> Result<Canvas, String> {
        if let Some(scale) = settings.dpi_scale
            && !(scale.is_finite() && scale > 0.0)
        {
            return Err(format!("the DPI scale must be positive, got {scale}"));
        }
        context::with(|ctx| {
            let settings = CanvasSettings {
                dpi_scale: Some(
                    settings
                        .dpi_scale
                        .unwrap_or_else(|| ctx.gfx.screen.draw_scale()),
                ),
                ..settings
            };
            Canvas::create(
                &mut *ctx.backend,
                width,
                height,
                ctx.gfx.default_filter,
                settings,
            )
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

    /// Makes a canvas. `settings.dpi_scale` must be set.
    pub(crate) fn create(
        backend: &mut dyn Backend,
        width: u32,
        height: u32,
        filter: Filter,
        settings: CanvasSettings,
    ) -> Result<Canvas, String> {
        let scale = settings.dpi_scale.unwrap_or(1.0);
        let samples = settings.msaa.max(1);
        let format = settings
            .format
            .unwrap_or_else(|| super::normal_format(backend));
        if samples > 1 && settings.mipmaps != MipmapMode::None {
            return Err("a multisampled canvas cannot have mipmaps".to_string());
        }
        if format.is_depth() {
            return Err(format!(
                "{format} canvases are not supported yet; only color formats are"
            ));
        }
        let pixels = Canvas::pixels_for(width, height, scale);
        let texture = backend.create_render_target_with(&RenderTargetDesc {
            width: pixels.0,
            height: pixels.1,
            format,
            mipmaps: match settings.mipmaps {
                MipmapMode::None => None,
                MipmapMode::Auto => Some(true),
                MipmapMode::Manual => Some(false),
            },
            samples,
        })?;
        let msaa = backend.texture_msaa(texture);
        let image = Image::from_texture(texture, width, height, filter, Kind::Canvas);
        image.set_storage(format, settings.readable.unwrap_or(true));
        if settings.mipmaps != MipmapMode::None {
            image.mark_mipmapped(32 - pixels.0.max(pixels.1).max(1).leading_zeros());
        }
        Ok(Canvas {
            image,
            pixels,
            scale,
            mipmaps: settings.mipmaps,
            msaa,
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
    /// [`pixel_dimensions`](Canvas::pixel_dimensions) in size, in the
    /// canvas's [`format`](Image::format).
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
