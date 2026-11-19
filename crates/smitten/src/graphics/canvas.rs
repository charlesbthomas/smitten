use std::fmt;
use std::ops::Deref;

use smitten_core::backend::{Backend, RenderTargetDesc, TargetSlice, TextureKind};
use smitten_core::{Filter, PixelFormat};

use super::stats::Kind;
use super::{GraphicsError, Image, ImageData};
use crate::context;

/// An off-screen image that can be drawn to.
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
/// soft edges come out too dark.
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
    kind: TextureKind,
    /// How many layers, faces or depth slices it has, 1 when flat.
    layers: u32,
}

/// Whether a canvas has mipmaps and when they are made.
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

/// Options for making a canvas. Start from the default and change what is
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
    /// How the canvas stores its pixels. `None` is the usual format:
    /// [`PixelFormat::Rgba8`], or [`PixelFormat::Srgba8`] with
    /// gamma-correct rendering. A floating-point format such as
    /// [`PixelFormat::Rgba16f`] holds colors brighter than white, for HDR
    /// effects. [`graphics::canvas_formats`](super::canvas_formats) lists
    /// what the GPU can draw to.
    ///
    /// A depth format, such as [`PixelFormat::Depth24Stencil8`], makes a
    /// depth canvas: one that holds depth, and maybe stencil, values for
    /// the color canvases drawn to with it, given to
    /// [`graphics::set_canvas_with_depth`](super::set_canvas_with_depth).
    /// It is flat and has no mipmaps. Made readable, shaders can sample
    /// it, for instance as a shadow map; see
    /// [`Image::set_depth_sample_mode`].
    pub format: Option<PixelFormat>,
    /// Whether the canvas has mipmaps and when they are made.
    pub mipmaps: MipmapMode,
    /// How many samples each pixel holds; 0 or 1 for no multisampling.
    pub msaa: u32,
    /// Whether the canvas will be drawn or sampled, as well as drawn to.
    /// `None` means yes for a color canvas and no for a depth one. For a
    /// color canvas, smitten records the setting for [`Image::is_readable`]
    /// but draws it the same way. A depth canvas that is not readable
    /// cannot be drawn or sent to a shader, but some GPUs only offer some
    /// depth formats that way, and only it can be multisampled.
    pub readable: Option<bool>,
    /// The canvas's shape: flat, an array of layers, the six faces of a
    /// cube, or a volume. Draw to one layer, face or depth slice with
    /// [`graphics::set_canvas_targets`](super::set_canvas_targets), and
    /// sample a non-flat canvas in a shader with
    /// [`Shader::send_texture`](super::Shader::send_texture).
    pub texture_type: TextureKind,
    /// How many layers an array canvas has, or how deep a volume canvas
    /// is. Ignored for a flat canvas, which has 1, and a cube, which has 6.
    pub layers: u32,
}

impl Default for CanvasSettings {
    fn default() -> Self {
        Self {
            dpi_scale: None,
            format: None,
            mipmaps: MipmapMode::None,
            msaa: 0,
            readable: None,
            texture_type: TextureKind::Flat,
            layers: 1,
        }
    }
}

impl Canvas {
    /// Creates a transparent canvas of `width` by `height` units.
    ///
    /// It is made as sharp as the screen, so on a high-DPI display it holds
    /// more pixels than units. Use [`Canvas::with_settings`] to choose the
    /// sharpness instead.
    pub fn new(width: u32, height: u32) -> Result<Canvas, GraphicsError> {
        Canvas::with_settings(width, height, CanvasSettings::default())
    }

    /// Creates a transparent canvas of `width` by `height` units as
    /// `settings` describe.
    ///
    /// Fails for a format the GPU cannot draw to, for multisampling with
    /// mipmaps, and for a depth canvas that has mipmaps, is not flat, or
    /// is both readable and multisampled.
    pub fn with_settings(
        width: u32,
        height: u32,
        settings: CanvasSettings,
    ) -> Result<Canvas, GraphicsError> {
        if let Some(scale) = settings.dpi_scale
            && !(scale.is_finite() && scale > 0.0)
        {
            return Err(format!("the DPI scale must be positive, got {scale}").into());
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
    /// multisampled.
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
    ) -> Result<Canvas, GraphicsError> {
        let scale = settings.dpi_scale.unwrap_or(1.0);
        let samples = settings.msaa.max(1);
        let format = settings
            .format
            .unwrap_or_else(|| super::normal_format(backend));
        if samples > 1 && settings.mipmaps != MipmapMode::None {
            return Err(GraphicsError::new(
                "a multisampled canvas cannot have mipmaps",
            ));
        }
        let readable = settings.readable.unwrap_or(format.is_color());
        if format.is_depth() {
            if settings.mipmaps != MipmapMode::None {
                return Err(format!("a {format} canvas cannot have mipmaps").into());
            }
            if settings.texture_type != TextureKind::Flat {
                return Err(format!("a {format} canvas must be flat").into());
            }
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
            kind: settings.texture_type,
            layers: settings.layers,
            // A color canvas can always be sampled; only the setting is
            // kept.
            readable: readable || format.is_color(),
        })?;
        let msaa = backend.texture_msaa(texture);
        let image = Image::from_texture(texture, width, height, filter, Kind::Canvas);
        image.set_storage(format, readable);
        image.set_texture_kind(settings.texture_type);
        if settings.mipmaps != MipmapMode::None {
            image.mark_mipmapped(32 - pixels.0.max(pixels.1).max(1).leading_zeros());
        }
        Ok(Canvas {
            image,
            pixels,
            scale,
            mipmaps: settings.mipmaps,
            msaa,
            kind: settings.texture_type,
            layers: match settings.texture_type {
                TextureKind::Flat => 1,
                TextureKind::Cube => 6,
                TextureKind::Array | TextureKind::Volume => settings.layers,
            },
        })
    }

    /// The canvas's shape.
    pub fn texture_type(&self) -> TextureKind {
        self.kind
    }

    /// How many layers, cube faces or depth slices the canvas has: 1 for a
    /// flat canvas, 6 for a cube.
    pub fn layer_count(&self) -> u32 {
        self.layers
    }

    /// Whether the canvas has mipmaps and when they are made.
    pub fn mipmap_mode(&self) -> MipmapMode {
        self.mipmaps
    }

    /// Makes the canvas's mipmaps again from what has been drawn to it so
    /// far. Only needed with [`MipmapMode::Manual`].
    ///
    /// Like [`Canvas::to_image_data`], this draws what is queued for every
    /// canvas straight away.
    pub fn generate_mipmaps(&self) -> Result<(), GraphicsError> {
        if self.mipmaps == MipmapMode::None {
            return Err(GraphicsError::new("the canvas was made without mipmaps"));
        }
        context::with(|ctx| {
            ctx.gfx.flush_canvases(&mut *ctx.backend);
            ctx.backend.generate_mipmaps(self.texture())
        })?;
        Ok(())
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

    /// Copies the canvas's pixels back from the GPU. The result is
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
    pub fn to_image_data(&self) -> Result<ImageData, GraphicsError> {
        self.to_image_data_slice(0, 0)
    }

    /// As [`Canvas::to_image_data`], copying one layer, cube face or depth
    /// slice at one mipmap level. The result is that level's size.
    pub fn to_image_data_slice(&self, layer: u32, mipmap: u32) -> Result<ImageData, GraphicsError> {
        if layer >= self.layers {
            return Err(format!(
                "layer {layer} does not exist: the canvas has {}",
                self.layers
            )
            .into());
        }
        if mipmap >= self.mipmap_count() {
            return Err(format!(
                "mipmap level {mipmap} does not exist: the canvas has {}",
                self.mipmap_count()
            )
            .into());
        }
        let slice = TargetSlice { layer, mipmap };
        context::with(|ctx| ctx.gfx.read_canvas(&mut *ctx.backend, self, slice))
    }

    /// Runs `draw` with this canvas as the target, then goes back to
    /// whatever was the target before.
    pub fn render_to(&self, draw: impl FnOnce()) {
        let previous = context::with(|ctx| ctx.gfx.canvas_list());
        super::set_canvas(Some(self));
        draw();
        context::with(|ctx| ctx.gfx.set_canvas_list(previous));
    }
}

/// Two canvases are equal when they are the same canvas, as for
/// [`Image`].
impl PartialEq for Canvas {
    fn eq(&self, other: &Self) -> bool {
        self.image == other.image
    }
}

impl Eq for Canvas {}

impl std::hash::Hash for Canvas {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.image.hash(state);
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
            .field("type", &self.kind)
            .finish()
    }
}
