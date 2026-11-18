//! Fonts and printing text.

use super::*;

impl Font {
    /// Loads a TrueType or OpenType font file at `size` pixels, or an
    /// AngelCode BMFont if the file name ends in `.fnt` (whose own size is
    /// used then, and whose page images are found next to it).
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes, which
    /// normally means inside the game's `assets` folder.
    pub fn load(path: impl AsRef<Path>, size: f32) -> Result<Font, FontError> {
        Font::load_with(path, size, FontSettings::default())
    }
}

impl Font {
    /// Loads a font file with the full set of LÖVE's `newFont` arguments:
    /// its size, and how its outlines are fitted to pixels and how sharp
    /// its glyphs are, as [`FontSettings`] says.
    pub fn load_with(
        path: impl AsRef<Path>,
        size: f32,
        settings: FontSettings,
    ) -> Result<Font, FontError> {
        let path = path.as_ref();
        let is_bmfont = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fnt"));
        let font = if is_bmfont {
            Font::read_bmfont(path)?
        } else {
            let bytes =
                crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
            Font::from_bytes_with(&bytes, size, settings)?
        };
        font.watch(path);
        Ok(font)
    }
}

impl Font {
    /// Loads an AngelCode BMFont from its `.fnt` file, whose page images are
    /// found next to it. Only the text form of `.fnt` files is read, as in
    /// LÖVE. [`Font::load`] does the same for a file ending in `.fnt`.
    pub fn load_bmfont(path: impl AsRef<Path>) -> Result<Font, FontError> {
        let path = path.as_ref();
        let font = Font::read_bmfont(path)?;
        font.watch(path);
        Ok(font)
    }
}

impl Font {
    /// Loads a bitmap font: an image holding the characters of `glyphs` in a
    /// row, separated by columns in the color of its top-left pixel. The
    /// equivalent of `love.graphics.newImageFont`. See [`Font::from_image`].
    ///
    /// Pixel fonts usually want `font.set_filter(Filter::Nearest)` so they
    /// stay crisp when enlarged.
    #[doc(alias = "love.graphics.newImageFont")]
    pub fn load_image_font(path: impl AsRef<Path>, glyphs: &str) -> Result<Font, FontError> {
        Font::load_image_font_with(path, glyphs, 0.0)
    }
}

impl Font {
    /// Loads a bitmap font with `extra_spacing` units added after each
    /// character, which may be negative: LÖVE's `newImageFont` with its
    /// `extraspacing` argument.
    pub fn load_image_font_with(
        path: impl AsRef<Path>,
        glyphs: &str,
        extra_spacing: f32,
    ) -> Result<Font, FontError> {
        let path = path.as_ref();
        let bytes =
            crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
        let font =
            Font::from_image(&bytes, glyphs, extra_spacing).map_err(|error| match error {
                FontError::Parse(message) => {
                    FontError::Parse(format!("{}: {message}", path.display()))
                }
                other => other,
            })?;
        font.watch(path);
        Ok(font)
    }
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

/// Draws text in the current font and color, placed by `params`, the
/// equivalent of `love.graphics.print`. Line breaks in the text start new
/// lines. The text can be a string or colored parts, as [`TextSpans`]
/// describes.
#[doc(alias = "love.graphics.print")]
pub fn print<'a>(text: impl Into<TextSpans<'a>>, params: impl Into<DrawParams>) {
    let local = params.into().to_transform();
    text.into().with_segments(|segments| {
        context::with(|ctx| {
            ctx.gfx
                .push_colored_text(ctx.backend.as_mut(), segments, None, &local);
        });
    });
}

/// Draws text wrapped to `limit` pixels wide and aligned within that
/// width, placed by `params`, the equivalent of `love.graphics.printf`.
#[doc(alias = "love.graphics.printf")]
pub fn printf<'a>(
    text: impl Into<TextSpans<'a>>,
    params: impl Into<DrawParams>,
    limit: f32,
    align: Align,
) {
    let local = params.into().to_transform();
    text.into().with_segments(|segments| {
        context::with(|ctx| {
            ctx.gfx
                .push_colored_text(ctx.backend.as_mut(), segments, Some((limit, align)), &local);
        });
    });
}

/// One layer of an [`ArrayImage`] or array [`Canvas`], drawn by [`draw`]
/// as LÖVE's `drawLayer` does. Made with [`ArrayImage::layer`] or
/// [`Canvas::layer`].
///
/// Drawing panics if a custom shader is active: a shader reads an array
/// image by declaring a `texture_2d_array<f32>` and being sent it with
/// [`Shader::send_texture`].
#[derive(Debug, Clone, Copy)]
pub struct Layer<'a> {
    texture: TextureId,
    filter: Filter,
    layer: u32,
    quad: Option<&'a Quad>,
    size: (u32, u32),
}

impl<'a> Layer<'a> {
    fn of(image: &impl Layered, layer: u32) -> Self {
        assert!(
            layer < image.layers(),
            "layer {layer} does not exist: there are {} layers",
            image.layers()
        );
        Layer {
            texture: image.layer_texture(),
            filter: image.layer_filter(),
            layer,
            quad: None,
            size: image.layer_size(),
        }
    }

    /// Just the part of the layer that `quad` describes.
    #[must_use]
    pub fn region(self, quad: &'a Quad) -> Layer<'a> {
        Layer {
            quad: Some(quad),
            ..self
        }
    }
}

impl ArrayImage {
    /// Layer `layer` of the image, counting from 0, to draw with [`draw`].
    ///
    /// Panics if there is no such layer.
    #[doc(alias = "drawLayer")]
    pub fn layer(&self, layer: u32) -> Layer<'_> {
        Layer::of(self, layer)
    }
}

impl Canvas {
    /// Layer `layer` of an array canvas, counting from 0, to draw with
    /// [`draw`].
    ///
    /// Panics if the canvas was not made with [`TextureKind::Array`], or
    /// there is no such layer.
    pub fn layer(&self, layer: u32) -> Layer<'_> {
        Layer::of(self, layer)
    }
}

impl draw::sealed::Sealed for Layer<'_> {
    fn draw_transformed(&self, local: &Transform) {
        let whole;
        let quad = match self.quad {
            Some(quad) => quad,
            None => {
                let (width, height) = (self.size.0 as f32, self.size.1 as f32);
                whole = Quad::new((0.0, 0.0), (width, height), (width, height));
                &whole
            }
        };
        context::with(|ctx| {
            assert!(
                ctx.gfx.shader.is_none(),
                "an array image layer cannot be drawn with a custom shader active; \
                 send the array image to the shader with Shader::send_texture instead"
            );
            ctx.gfx
                .push_texture(self.texture, self.layer, self.filter, quad, local);
        });
    }
}
impl Drawable for Layer<'_> {}

/// Something with layers that [`Layer`] can draw one of: an
/// [`ArrayImage`], or a [`Canvas`] made with [`TextureKind::Array`].
trait Layered {
    fn layer_texture(&self) -> TextureId;
    fn layer_filter(&self) -> Filter;
    fn layer_size(&self) -> (u32, u32);
    fn layers(&self) -> u32;
}

impl Layered for ArrayImage {
    fn layer_texture(&self) -> TextureId {
        self.texture()
    }
    fn layer_filter(&self) -> Filter {
        self.filter()
    }
    fn layer_size(&self) -> (u32, u32) {
        (self.width(), self.height())
    }
    fn layers(&self) -> u32 {
        self.layer_count()
    }
}

impl Layered for Canvas {
    fn layer_texture(&self) -> TextureId {
        assert!(
            self.texture_type() == TextureKind::Array,
            "Canvas::layer needs an array canvas, not a {:?} one",
            self.texture_type()
        );
        self.texture()
    }
    fn layer_filter(&self) -> Filter {
        self.filter()
    }
    fn layer_size(&self) -> (u32, u32) {
        self.dimensions()
    }
    fn layers(&self) -> u32 {
        self.layer_count()
    }
}

/// Whether the GPU can draw images compressed in `format`, the
/// equivalent of `love.graphics.getImageFormats`.
#[doc(alias = "love.graphics.getImageFormats")]
pub fn is_compressed_format_supported(format: CompressedFormat) -> bool {
    context::with(|ctx| ctx.backend.supports_compressed_format(format))
}

/// Whether the GPU can draw images in `format`, made with
/// [`Image::from_image_data`]. The uncompressed half of
/// `love.graphics.getImageFormats`.
pub fn is_image_format_supported(format: PixelFormat) -> bool {
    format.is_color() && context::with(|ctx| ctx.backend.supports_pixel_format(format, false))
}

/// The uncompressed formats images can have on this GPU, in the order
/// [`PixelFormat::ALL`] lists them.
pub fn image_formats() -> Vec<PixelFormat> {
    PixelFormat::ALL
        .into_iter()
        .filter(|&format| is_image_format_supported(format))
        .collect()
}

/// Whether a canvas can be made in `format` on this GPU, the equivalent
/// of looking `format` up in `love.graphics.getCanvasFormats`. With
/// `readable`, it must also be drawable and readable by shaders, as a
/// canvas usually is.
///
/// A depth canvas is readable when shaders can sample it; LÖVE's depth
/// canvases are not by default.
pub fn is_canvas_format_supported(format: PixelFormat, readable: bool) -> bool {
    context::with(|ctx| {
        ctx.backend.supports_pixel_format(format, true)
            && (!readable || ctx.backend.supports_pixel_format(format, false))
    })
}

/// The texture shapes this GPU supports, the equivalent of
/// `love.graphics.getTextureTypes`. Every SDL GPU backend supports all
/// four, for images and canvases alike.
#[doc(alias = "love.graphics.getTextureTypes")]
pub fn texture_types() -> [(TextureKind, bool); 4] {
    [
        (TextureKind::Flat, true),
        (TextureKind::Array, true),
        (TextureKind::Cube, true),
        (TextureKind::Volume, true),
    ]
}

/// The formats canvases can have on this GPU, the equivalent of
/// `love.graphics.getCanvasFormats(readable)`, in the order
/// [`PixelFormat::ALL`] lists them.
#[doc(alias = "love.graphics.getCanvasFormats")]
pub fn canvas_formats(readable: bool) -> Vec<PixelFormat> {
    PixelFormat::ALL
        .into_iter()
        .filter(|&format| is_canvas_format_supported(format, readable))
        .collect()
}

/// An optional graphics feature, the equivalent of LÖVE's
/// `GraphicsFeature`. Ask about one with [`is_supported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GraphicsFeature {
    /// [`Wrap::ClampZero`](crate::Wrap::ClampZero) as its own mode, rather
    /// than falling back to [`Wrap::Clamp`](crate::Wrap::Clamp).
    ClampZero,
    /// The [`BlendMode::Lighten`] and
    /// [`BlendMode::Darken`] blend modes.
    Lighten,
    /// Drawing to several canvases at once that have different formats.
    MultiCanvasFormats,
    /// Textures whose sides are not powers of two, with mipmaps and every
    /// wrap mode.
    FullNpot,
    /// Full-precision floats in fragment shaders.
    PixelShaderHighp,
    /// The derivative functions in shaders, such as `dpdx`.
    ShaderDerivatives,
    /// What LÖVE allows only with GLSL 3: integer types in shaders and
    /// between [`Shader`] stages, sampling textures in `position`, and
    /// `instance_index`. WGSL always has them.
    Glsl3,
    /// Drawing many copies of a mesh at once with [`draw_instanced`].
    Instancing,
}

/// Whether the GPU supports `feature`, the equivalent of the table
/// `love.graphics.getSupported` returns.
pub fn is_supported(feature: GraphicsFeature) -> bool {
    match feature {
        // SDL's GPU API has no border colors to build it from.
        GraphicsFeature::ClampZero => false,
        // Every GPU SDL's GPU API runs on has these.
        GraphicsFeature::MultiCanvasFormats
        | GraphicsFeature::Lighten
        | GraphicsFeature::FullNpot
        | GraphicsFeature::PixelShaderHighp
        | GraphicsFeature::ShaderDerivatives
        | GraphicsFeature::Glsl3
        | GraphicsFeature::Instancing => true,
    }
}

/// The largest sizes and counts the GPU allows, the equivalent of the
/// table `love.graphics.getSystemLimits` returns. See [`system_limits`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct SystemLimits {
    /// The widest point [`set_point_size`] can make. Points are drawn as
    /// squares, so there is no limit.
    pub point_size: f32,
    /// The widest and tallest an [`Image`] or [`Canvas`] can be.
    pub texture_size: u32,
    /// The widest, tallest and deepest a [`VolumeImage`] can be.
    pub volume_texture_size: u32,
    /// The widest and tallest each face of a [`CubeImage`] can be.
    pub cube_texture_size: u32,
    /// The most layers an [`ArrayImage`] can have.
    pub texture_layers: u32,
    /// The most canvases that can be drawn to at once with
    /// [`set_canvases`].
    pub multi_canvas: u32,
    /// The most samples per pixel a multisampled canvas can have, 1 if
    /// there is no multisampling.
    pub canvas_msaa: u32,
    /// The most anisotropic filtering a texture can have. Smitten does
    /// not filter anisotropically yet, so this is 1.
    pub anisotropy: f32,
}

/// The largest sizes and counts the GPU allows, the equivalent of
/// `love.graphics.getSystemLimits`.
///
/// SDL's GPU API does not report most limits, so the texture sizes are
/// the ones every GPU it supports on desktops has: Direct3D 12 feature
/// level 11_0 promises them, and Vulkan and Metal desktop GPUs match. A
/// phone's GPU may allow less. `canvas_msaa` is asked of the GPU.
#[doc(alias = "love.graphics.getSystemLimits")]
pub fn system_limits() -> SystemLimits {
    SystemLimits {
        point_size: f32::INFINITY,
        texture_size: 16384,
        volume_texture_size: 2048,
        cube_texture_size: 16384,
        texture_layers: 2048,
        multi_canvas: MAX_CANVASES as u32,
        canvas_msaa: context::with(|ctx| ctx.backend.max_msaa()),
        anisotropy: 1.0,
    }
}
