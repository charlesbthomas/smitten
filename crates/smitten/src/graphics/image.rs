use std::cell::Cell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use smitten_core::backend::{Backend, TextureId, TextureKind};
use smitten_core::{CompareMode, Filter, PixelFormat, Wrap};

use crate::context;
use crate::reload::Asset;

use super::CompressedImageData;
use super::compressed::is_compressed_file;
use super::stats::{Counted, Kind};

/// A picture that can be drawn, the equivalent of LÖVE's `Image`.
///
/// Cloning is cheap and gives another handle to the same picture. Its GPU
/// memory is freed when the last handle is dropped.
#[derive(Clone)]
pub struct Image {
    inner: Rc<Inner>,
}

pub(crate) struct Inner {
    // In cells so a changed file can be loaded into every handle at once.
    texture: Cell<TextureId>,
    width: Cell<u32>,
    height: Cell<u32>,
    filter: Cell<Filter>,
    wrap: Cell<(Wrap, Wrap)>,
    /// How many mipmap levels the texture has, 1 for none.
    mipmap_count: Cell<u32>,
    /// Whether the pixels came from compressed data, which cannot be
    /// replaced in place.
    compressed: Cell<bool>,
    mipmap_filter: Cell<Option<(Filter, f32)>>,
    /// How the texture stores its pixels.
    format: Cell<PixelFormat>,
    /// Whether it was made to be drawn and sampled. Only a canvas can be
    /// made otherwise.
    readable: Cell<bool>,
    /// Whether the GPU decodes its colors from sRGB, which is what
    /// gamma-correct rendering does for images not made `linear`.
    srgb: Cell<bool>,
    /// The texture's shape; only a canvas can be other than flat.
    kind: Cell<TextureKind>,
    /// How a depth canvas compares when a shader samples it.
    depth_sample_mode: Cell<Option<CompareMode>>,
    _counted: Counted,
}

/// Options for making an image, the equivalent of the settings table
/// LÖVE's `newImage` takes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImageSettings {
    /// Whether to make mipmaps: smaller copies of the image that the GPU
    /// uses when it is drawn small, so it does not shimmer or look grainy.
    /// They take a third more memory. See [`Image::set_mipmap_filter`].
    pub mipmaps: bool,
    /// With gamma-correct rendering, whether the pixels are used as they
    /// are rather than decoded from sRGB, LÖVE's `linear` setting. Use it
    /// for pictures that hold data, such as normal maps, rather than
    /// colors. It changes nothing without gamma-correct rendering.
    pub linear: bool,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // If the game has already shut down, the backend freed everything.
        context::try_with(|ctx| ctx.backend.destroy_texture(self.texture.get()));
    }
}

impl Image {
    /// Creates an image from raw pixels: `width * height` of them, four
    /// bytes each (red, green, blue, alpha), in rows from the top.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Image, ImageError> {
        Image::from_rgba_with(width, height, rgba, ImageSettings::default())
    }

    /// As [`Image::from_rgba`], with options such as mipmaps.
    pub fn from_rgba_with(
        width: u32,
        height: u32,
        rgba: &[u8],
        settings: ImageSettings,
    ) -> Result<Image, ImageError> {
        Image::from_pixels(PixelFormat::Rgba8, width, height, rgba, settings)
    }

    /// Creates an image from `pixels` stored in `format`, as
    /// [`ImageData::from_raw`](super::ImageData::from_raw) takes them.
    pub(crate) fn from_pixels(
        format: PixelFormat,
        width: u32,
        height: u32,
        pixels: &[u8],
        settings: ImageSettings,
    ) -> Result<Image, ImageError> {
        context::with(|ctx| {
            // Gamma-correct rendering stores ordinary colors sRGB-encoded.
            let format = if format == PixelFormat::Rgba8
                && ctx.backend.is_gamma_correct()
                && !settings.linear
            {
                PixelFormat::Srgba8
            } else {
                format
            };
            let texture = match (format, settings.mipmaps) {
                (PixelFormat::Rgba8, true) => {
                    ctx.backend.create_mipmapped_texture(width, height, pixels)
                }
                (PixelFormat::Rgba8, false) => ctx.backend.create_texture(width, height, pixels),
                (format, mipmaps) => ctx
                    .backend
                    .create_texture_in(format, width, height, pixels, mipmaps),
            }
            .map_err(ImageError::Upload)?;
            let mipmap_count = if settings.mipmaps {
                32 - width.max(height).max(1).leading_zeros()
            } else {
                1
            };
            Ok(Image {
                inner: Rc::new(Inner {
                    texture: Cell::new(texture),
                    width: Cell::new(width),
                    height: Cell::new(height),
                    filter: Cell::new(ctx.gfx.default_filter),
                    wrap: Cell::default(),
                    mipmap_count: Cell::new(mipmap_count),
                    compressed: Cell::new(false),
                    mipmap_filter: Cell::new(settings.mipmaps.then_some((Filter::Linear, 0.0))),
                    format: Cell::new(format),
                    readable: Cell::new(true),
                    srgb: Cell::new(format.is_srgb()),
                    kind: Cell::new(TextureKind::Flat),
                    depth_sample_mode: Cell::new(None),
                    _counted: Counted::new(Kind::Image),
                }),
            })
        })
    }

    /// Wraps a texture the backend already holds. The image takes over
    /// freeing it. `kind` says what to count it as in the statistics.
    pub(crate) fn from_texture(
        texture: TextureId,
        width: u32,
        height: u32,
        filter: Filter,
        kind: Kind,
    ) -> Image {
        Image {
            inner: Rc::new(Inner {
                texture: Cell::new(texture),
                width: Cell::new(width),
                height: Cell::new(height),
                filter: Cell::new(filter),
                wrap: Cell::default(),
                mipmap_count: Cell::new(1),
                compressed: Cell::new(false),
                mipmap_filter: Cell::new(None),
                format: Cell::new(PixelFormat::Rgba8),
                readable: Cell::new(true),
                srgb: Cell::new(false),
                kind: Cell::new(TextureKind::Flat),
                depth_sample_mode: Cell::new(None),
                _counted: Counted::new(kind),
            }),
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.inner.width.get()
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.inner.height.get()
    }

    /// Size in pixels as `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.inner.width.get(), self.inner.height.get())
    }

    /// Whether the pixels came from compressed data, such as a DDS file,
    /// the equivalent of `Image:isCompressed`. A compressed image cannot
    /// have its pixels replaced.
    pub fn is_compressed(&self) -> bool {
        self.inner.compressed.get()
    }

    /// How the image stores its pixels, the equivalent of
    /// `Texture:getFormat`. An image loaded from a file, and a compressed
    /// one, report [`PixelFormat::Rgba8`]; see [`Image::is_compressed`].
    pub fn format(&self) -> PixelFormat {
        self.inner.format.get()
    }

    /// Whether the image can be drawn and sampled by shaders, the
    /// equivalent of `Texture:isReadable`. Every image is; a canvas made
    /// with `readable: Some(false)` is not.
    pub fn is_readable(&self) -> bool {
        self.inner.readable.get()
    }

    /// Records how a texture made by someone else stores its pixels.
    pub(crate) fn set_storage(&self, format: PixelFormat, readable: bool) {
        self.inner.format.set(format);
        self.inner.readable.set(readable);
        self.inner.srgb.set(format.is_srgb());
    }

    /// Whether the image's pixels are used as they are, rather than
    /// decoded from sRGB, while rendering is gamma-correct, the
    /// equivalent of `Image:isFormatLinear`. Always `false` without
    /// gamma-correct rendering, as in LÖVE.
    pub fn is_format_linear(&self) -> bool {
        super::is_gamma_correct() && !self.inner.srgb.get()
    }

    /// How the image is sampled when scaled or rotated.
    pub fn filter(&self) -> Filter {
        self.inner.filter.get()
    }

    /// Sets the filter for every handle to this image.
    pub fn set_filter(&self, filter: Filter) {
        self.inner.filter.set(filter);
    }

    /// What the image shows beyond its edges, horizontally and vertically.
    pub fn wrap(&self) -> (Wrap, Wrap) {
        self.inner.wrap.get()
    }

    /// Sets what the image shows beyond its edges, for every handle to it.
    /// With [`Wrap::Repeat`], a [`Quad`] larger than the image draws it
    /// tiled, which is the cheap way to fill a background.
    ///
    /// [`Wrap::ClampZero`] becomes [`Wrap::Clamp`] on GPUs that cannot do
    /// it, which today is all of them, and [`Image::wrap`] then says so.
    pub fn set_wrap(&self, horizontal: Wrap, vertical: Wrap) {
        let supported = |wrap: Wrap| match wrap {
            Wrap::ClampZero if !super::is_supported(super::GraphicsFeature::ClampZero) => {
                Wrap::Clamp
            }
            other => other,
        };
        let (horizontal, vertical) = (supported(horizontal), supported(vertical));
        self.inner.wrap.set((horizontal, vertical));
        context::with(|ctx| {
            ctx.backend
                .set_texture_wrap(self.texture(), horizontal, vertical);
        });
    }

    /// Creates an image from compressed pixels, the equivalent of
    /// `love.graphics.newImage(compresseddata)`. Mipmaps in the data are
    /// used, blended as [`Image::set_mipmap_filter`] describes. Fails if
    /// the GPU cannot read the format, which
    /// [`graphics::is_compressed_format_supported`](super::is_compressed_format_supported)
    /// tells beforehand.
    ///
    /// Its pixels cannot be replaced.
    pub fn from_compressed_data(data: &CompressedImageData) -> Result<Image, ImageError> {
        Image::from_compressed_data_with(data, ImageSettings::default())
    }

    /// As [`Image::from_compressed_data`], with options. Only `linear`
    /// applies; the mipmaps are the ones in the data.
    pub fn from_compressed_data_with(
        data: &CompressedImageData,
        settings: ImageSettings,
    ) -> Result<Image, ImageError> {
        context::with(|ctx| {
            let (width, height) = data.dimensions();
            let srgb = ctx.backend.is_gamma_correct() && !settings.linear;
            let texture = ctx
                .backend
                .create_compressed_texture(data.format(), srgb, width, height, &data.levels())
                .map_err(ImageError::Upload)?;
            let mipmap_count = data.mipmap_count();
            Ok(Image {
                inner: Rc::new(Inner {
                    texture: Cell::new(texture),
                    width: Cell::new(width),
                    height: Cell::new(height),
                    filter: Cell::new(ctx.gfx.default_filter),
                    wrap: Cell::default(),
                    mipmap_count: Cell::new(mipmap_count),
                    compressed: Cell::new(true),
                    mipmap_filter: Cell::new((mipmap_count > 1).then_some((Filter::Linear, 0.0))),
                    format: Cell::new(PixelFormat::Rgba8),
                    readable: Cell::new(true),
                    srgb: Cell::new(srgb),
                    kind: Cell::new(TextureKind::Flat),
                    depth_sample_mode: Cell::new(None),
                    _counted: Counted::new(Kind::Image),
                }),
            })
        })
    }

    /// How many mipmap levels the image has, counting the full-size image:
    /// 1 for an image made without mipmaps.
    pub fn mipmap_count(&self) -> u32 {
        self.inner.mipmap_count.get()
    }

    /// How the image blends between its mipmaps, and their sharpness, or
    /// `None` if it draws only the full-size image.
    pub fn mipmap_filter(&self) -> Option<(Filter, f32)> {
        self.inner.mipmap_filter.get()
    }

    /// Sets how the image blends between its mipmaps, the equivalent of
    /// `Image:setMipmapFilter`. [`Filter::Linear`] blends smoothly from one
    /// to the next, and is how it starts out. `None` draws only the
    /// full-size image. A positive `sharpness` picks larger mipmaps than
    /// the GPU would, for a crisper image, and a negative one smaller.
    ///
    /// Does nothing to an image made without mipmaps.
    pub fn set_mipmap_filter(&self, filter: Option<Filter>, sharpness: f32) {
        if self.inner.mipmap_count.get() <= 1 {
            return;
        }
        self.inner
            .mipmap_filter
            .set(filter.map(|filter| (filter, sharpness)));
        context::with(|ctx| {
            ctx.backend
                .set_texture_mipmap_filter(self.texture(), filter, sharpness);
        });
    }

    /// Copies pixels from memory over part of the image, with their
    /// top-left corner at `(x, y)`. The equivalent of
    /// `Image:replacePixels`. Every handle to the image changes.
    ///
    /// The pixels must fit inside the image. They are converted if `data`
    /// is in another format than the image.
    pub fn replace_pixels(
        &self,
        data: &super::ImageData,
        x: u32,
        y: u32,
    ) -> Result<(), ImageError> {
        let (width, height) = data.dimensions();
        if x + width > self.width() || y + height > self.height() {
            return Err(ImageError::Invalid(format!(
                "{width} by {height} pixels at ({x}, {y}) do not fit in a {} by {} image",
                self.width(),
                self.height()
            )));
        }
        context::with(|ctx| {
            ctx.backend
                .update_texture(
                    self.texture(),
                    x,
                    y,
                    width,
                    height,
                    &data.bytes_in(self.format()),
                )
                .map_err(ImageError::Upload)
        })
    }

    pub(crate) fn texture(&self) -> TextureId {
        self.inner.texture.get()
    }

    /// Records that the texture was made with `levels` mipmap levels,
    /// blended as a new image's are.
    /// The texture's shape, the equivalent of `Texture:getTextureType`.
    /// Only a [`Canvas`](super::Canvas) can be other than flat.
    pub fn texture_kind(&self) -> TextureKind {
        self.inner.kind.get()
    }

    pub(crate) fn set_texture_kind(&self, kind: TextureKind) {
        self.inner.kind.set(kind);
    }

    /// How a depth canvas is compared when a shader samples it, the
    /// equivalent of `Texture:getDepthSampleMode`. `None`, as a depth
    /// canvas starts out, reads plain depth values. Always `None` for
    /// anything other than a depth canvas.
    pub fn depth_sample_mode(&self) -> Option<CompareMode> {
        self.inner.depth_sample_mode.get()
    }

    /// Sets how a readable depth canvas is sampled, the equivalent of
    /// `Texture:setDepthSampleMode`. With a compare mode, a shader reads
    /// it as a `texture_depth_2d` with `textureSampleCompare`, which
    /// compares the depth it gives with the one stored and returns 1
    /// where the comparison passes and 0 where it fails, blended between
    /// neighbouring pixels with the linear filter. That is how shadow maps
    /// are read. With `None`, a shader reads it as a `texture_2d<f32>`
    /// whose red channel holds the depth.
    ///
    /// Fails for anything other than a readable depth canvas.
    pub fn set_depth_sample_mode(&self, mode: Option<CompareMode>) -> Result<(), String> {
        if !self.format().is_depth() {
            return Err("only a depth canvas has a depth sample mode".to_string());
        }
        if !self.is_readable() {
            return Err("only a readable depth canvas can be sampled".to_string());
        }
        context::with(|ctx| ctx.backend.set_texture_depth_compare(self.texture(), mode))?;
        self.inner.depth_sample_mode.set(mode);
        Ok(())
    }

    pub(crate) fn mark_mipmapped(&self, levels: u32) {
        self.inner.mipmap_count.set(levels);
        if levels > 1 {
            self.inner.mipmap_filter.set(Some((Filter::Linear, 0.0)));
        }
    }

    pub(crate) fn keep_alive(&self) -> Rc<dyn std::any::Any> {
        self.inner.clone()
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.inner.width.get())
            .field("height", &self.inner.height.get())
            .field("filter", &self.inner.filter.get())
            .finish()
    }
}

/// Why an image could not be created.
#[derive(Debug)]
pub enum ImageError {
    /// The file could not be read.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The file is not an image Smitten can decode.
    Decode { path: PathBuf, message: String },
    /// The pixels could not be sent to the GPU.
    Upload(String),
    /// Data that did not come from a file could not be decoded or encoded.
    Invalid(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Read { path, source } => {
                write!(f, "could not read image {}: {source}", path.display())
            }
            ImageError::Decode { path, message } => {
                write!(f, "could not decode image {}: {message}", path.display())
            }
            ImageError::Upload(message) => write!(f, "could not create image: {message}"),
            ImageError::Invalid(message) => write!(f, "could not use image data: {message}"),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ImageError::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Result<::image::RgbaImage, String> {
    decode_any(bytes).map(::image::DynamicImage::into_rgba8)
}

/// Decodes an image file, keeping the precision of its pixels.
pub(crate) fn decode_any(bytes: &[u8]) -> Result<::image::DynamicImage, String> {
    ::image::load_from_memory(bytes)
        // TGA files have no signature to recognize them by, so they are
        // tried last, and the first error is the one reported.
        .or_else(|error| {
            ::image::load_from_memory_with_format(bytes, ::image::ImageFormat::Tga)
                .map_err(|_| error)
        })
        .map_err(|error| error.to_string())
}

/// Reads an image file into raw pixels: width, height, and four bytes
/// (red, green, blue, alpha) for each pixel.
pub(crate) fn load_rgba(path: &Path) -> Result<(u32, u32, Vec<u8>), ImageError> {
    let bytes = crate::filesystem::read(path).map_err(|source| ImageError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let decoded = decode(&bytes).map_err(|message| ImageError::Decode {
        path: path.to_path_buf(),
        message,
    })?;
    Ok((decoded.width(), decoded.height(), decoded.into_raw()))
}

pub(crate) fn load(path: &Path, settings: ImageSettings) -> Result<Image, ImageError> {
    let bytes = crate::filesystem::read(path).map_err(|source| ImageError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    // A compressed file is used as it is, mipmaps and all.
    let image = if is_compressed_file(&bytes) {
        let data = CompressedImageData::from_bytes(&bytes).map_err(|error| match error {
            ImageError::Invalid(message) => ImageError::Decode {
                path: path.to_path_buf(),
                message,
            },
            other => other,
        })?;
        Image::from_compressed_data_with(&data, settings)?
    } else {
        let decoded = decode(&bytes).map_err(|message| ImageError::Decode {
            path: path.to_path_buf(),
            message,
        })?;
        Image::from_rgba_with(
            decoded.width(),
            decoded.height(),
            decoded.as_raw(),
            settings,
        )?
    };
    let file = crate::filesystem::resolve(path);
    context::with(|ctx| {
        ctx.reload
            .watch(file, Asset::Image(Rc::downgrade(&image.inner)));
    });
    Ok(image)
}

/// Replaces an image's pixels with a newer version of its file, which
/// may be compressed.
pub(crate) fn reload(inner: &Inner, backend: &mut dyn Backend, bytes: &[u8]) -> Result<(), String> {
    if is_compressed_file(bytes) {
        let data = CompressedImageData::from_bytes(bytes).map_err(|error| error.to_string())?;
        let (width, height) = data.dimensions();
        let texture = backend.create_compressed_texture(
            data.format(),
            inner.srgb.get(),
            width,
            height,
            &data.levels(),
        )?;
        replace_texture(
            inner,
            backend,
            texture,
            width,
            height,
            data.mipmap_count(),
            true,
        );
        return Ok(());
    }
    let decoded = decode(bytes)?;
    let (width, height) = (decoded.width(), decoded.height());
    if (width, height) == (inner.width.get(), inner.height.get()) && !inner.compressed.get() {
        return backend.update_texture(inner.texture.get(), 0, 0, width, height, decoded.as_raw());
    }
    // A different size, or an image that was compressed, needs a new
    // texture. Quads made for the old size keep their old numbers until
    // the game makes them again. An image with mipmaps keeps having them,
    // as many as the new size allows.
    // It stays sRGB-encoded or linear, as it was made.
    let format = if inner.srgb.get() {
        PixelFormat::Srgba8
    } else {
        PixelFormat::Rgba8
    };
    let mipmapped = inner.mipmap_count.get() > 1;
    let texture = backend.create_texture_in(format, width, height, decoded.as_raw(), mipmapped)?;
    let levels = if mipmapped {
        32 - width.max(height).leading_zeros()
    } else {
        1
    };
    inner.format.set(format);
    replace_texture(inner, backend, texture, width, height, levels, false);
    Ok(())
}

/// Puts a new texture in place of an image's old one, carrying over how
/// it wraps and blends its mipmaps.
fn replace_texture(
    inner: &Inner,
    backend: &mut dyn Backend,
    texture: TextureId,
    width: u32,
    height: u32,
    levels: u32,
    compressed: bool,
) {
    let (horizontal, vertical) = inner.wrap.get();
    backend.set_texture_wrap(texture, horizontal, vertical);
    if levels > 1 {
        // An image that gains mipmaps starts out blending them, as a new
        // one would.
        let (filter, sharpness) = if inner.mipmap_count.get() > 1 {
            inner.mipmap_filter.get().unzip()
        } else {
            (Some(Filter::Linear), Some(0.0))
        };
        let sharpness = sharpness.unwrap_or(0.0);
        backend.set_texture_mipmap_filter(texture, filter, sharpness);
        inner
            .mipmap_filter
            .set(filter.map(|filter| (filter, sharpness)));
    } else {
        inner.mipmap_filter.set(None);
    }
    backend.destroy_texture(inner.texture.replace(texture));
    inner.width.set(width);
    inner.height.set(height);
    inner.mipmap_count.set(levels);
    inner.compressed.set(compressed);
    if compressed {
        inner.format.set(PixelFormat::Rgba8);
    }
}

/// A rectangular part of an image, the equivalent of LÖVE's `Quad`.
///
/// Use it with [`crate::graphics::draw_quad`] to draw one sprite out of a
/// sprite sheet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) sheet_width: f32,
    pub(crate) sheet_height: f32,
}

impl Quad {
    /// Describes the `width` by `height` region whose top-left corner is at
    /// `(x, y)` in an image of `sheet_width` by `sheet_height` pixels.
    pub fn new(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        sheet_width: f32,
        sheet_height: f32,
    ) -> Self {
        Self {
            x,
            y,
            width,
            height,
            sheet_width,
            sheet_height,
        }
    }

    /// Width of the region in pixels.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Height of the region in pixels.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// The region as `(x, y, width, height)`, the equivalent of
    /// `Quad:getViewport`.
    pub fn viewport(&self) -> (f32, f32, f32, f32) {
        (self.x, self.y, self.width, self.height)
    }

    /// Moves the region to `width` by `height` pixels at `(x, y)` in the
    /// same image, the equivalent of `Quad:setViewport`. Moving one quad
    /// along a sprite sheet animates whatever draws it.
    pub fn set_viewport(&mut self, x: f32, y: f32, width: f32, height: f32) {
        (self.x, self.y, self.width, self.height) = (x, y, width, height);
    }

    /// The size of the image the region is in, as `(width, height)`, the
    /// equivalent of `Quad:getTextureDimensions`.
    pub fn texture_dimensions(&self) -> (f32, f32) {
        (self.sheet_width, self.sheet_height)
    }

    /// Changes the size of the image the region is in, the last two
    /// arguments of LÖVE's `Quad:setViewport`.
    pub fn set_texture_dimensions(&mut self, width: f32, height: f32) {
        (self.sheet_width, self.sheet_height) = (width, height);
    }
}

#[cfg(test)]
mod tests {
    use super::Quad;

    #[test]
    fn a_quad_can_be_read_back_and_moved() {
        let mut quad = Quad::new(0.0, 0.0, 16.0, 16.0, 64.0, 32.0);
        quad.set_viewport(16.0, 0.0, 16.0, 8.0);
        assert_eq!(quad.viewport(), (16.0, 0.0, 16.0, 8.0));
        assert_eq!(quad.texture_dimensions(), (64.0, 32.0));
        quad.set_texture_dimensions(128.0, 64.0);
        assert_eq!(quad.texture_dimensions(), (128.0, 64.0));
    }
}
