//! Pixels held in memory, the equivalent of LÖVE's `ImageData`.
//!
//! An [`Image`] lives on the GPU and can only be drawn. An [`ImageData`]
//! lives in ordinary memory, so its pixels can be read and changed: to
//! generate a texture, to read a level out of a picture, or to check what
//! color is under a point. Turn one into an image with
//! [`Image::from_image_data`] when it is ready to draw.

use std::borrow::Cow;
use std::fmt;
use std::path::Path;

use smitten_core::PixelFormat;

use super::image::{Image, ImageError, decode_any};
use super::pixels;
use crate::Color;

/// A grid of pixels that can be read and changed.
///
/// Its pixels are stored in a [`PixelFormat`], [`PixelFormat::Rgba8`]
/// unless it was made with another. [`ImageData::pixel`] and
/// [`ImageData::set_pixel`] work the same in every format.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageData {
    width: u32,
    height: u32,
    format: PixelFormat,
    /// [`PixelFormat::bytes_per_pixel`] bytes a pixel, in rows from the top.
    pixels: Vec<u8>,
}

impl ImageData {
    /// Creates `width` by `height` transparent black pixels.
    pub fn new(width: u32, height: u32) -> ImageData {
        ImageData {
            width,
            height,
            format: PixelFormat::Rgba8,
            pixels: vec![0; width as usize * height as usize * 4],
        }
    }

    /// Creates `width` by `height` pixels in `format`, all zero, the
    /// equivalent of `love.image.newImageData(width, height, format)`.
    /// For a format without alpha, they read back as opaque black.
    ///
    /// Fails for depth and stencil formats, which only canvases can have.
    pub fn with_format(
        width: u32,
        height: u32,
        format: PixelFormat,
    ) -> Result<ImageData, ImageError> {
        check_format(format)?;
        Ok(ImageData {
            width,
            height,
            format,
            pixels: vec![0; width as usize * height as usize * format.bytes_per_pixel()],
        })
    }

    /// Wraps raw pixels in `format`: `width * height` of them, each
    /// [`PixelFormat::bytes_per_pixel`] long, in rows from the top. The
    /// equivalent of `love.image.newImageData(width, height, format,
    /// rawdata)`. Multi-byte channels are little-endian, and packed
    /// formats such as `rgb10a2` are one little-endian 32-bit number with
    /// red in the lowest bits, as the GPU stores them.
    pub fn from_raw(
        width: u32,
        height: u32,
        format: PixelFormat,
        bytes: Vec<u8>,
    ) -> Result<ImageData, ImageError> {
        check_format(format)?;
        let expected = width as usize * height as usize * format.bytes_per_pixel();
        if bytes.len() != expected {
            return Err(ImageError::Invalid(format!(
                "{width} by {height} {format} pixels need {expected} bytes, not {}",
                bytes.len()
            )));
        }
        Ok(ImageData {
            width,
            height,
            format,
            pixels: bytes,
        })
    }

    /// How the pixels are stored, the equivalent of `ImageData:getFormat`.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Wraps raw pixels: `width * height` of them, four bytes each (red,
    /// green, blue, alpha), in rows from the top.
    ///
    /// Panics if `rgba` is not exactly that long.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> ImageData {
        assert_eq!(
            rgba.len(),
            width as usize * height as usize * 4,
            "{width} by {height} pixels need {} bytes",
            width as usize * height as usize * 4
        );
        ImageData {
            width,
            height,
            format: PixelFormat::Rgba8,
            pixels: rgba,
        }
    }

    /// Decodes the contents of a PNG, JPEG or TGA file. A 16-bit PNG
    /// keeps its precision as [`PixelFormat::Rgba16`], as in LÖVE; every
    /// other file gives [`PixelFormat::Rgba8`].
    pub fn from_bytes(bytes: &[u8]) -> Result<ImageData, ImageError> {
        let decoded = decode_any(bytes).map_err(ImageError::Invalid)?;
        let (width, height) = (decoded.width(), decoded.height());
        if decoded.color().bytes_per_pixel() / decoded.color().channel_count() == 2 {
            let pixels = decoded
                .into_rgba16()
                .into_raw()
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect();
            return Ok(ImageData {
                width,
                height,
                format: PixelFormat::Rgba16,
                pixels,
            });
        }
        Ok(ImageData {
            width,
            height,
            format: PixelFormat::Rgba8,
            pixels: decoded.into_rgba8().into_raw(),
        })
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Size in pixels as `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Where the pixel at `(x, y)` starts in `pixels`.
    fn offset(&self, x: u32, y: u32) -> usize {
        assert!(
            x < self.width && y < self.height,
            "pixel ({x}, {y}) is outside the {} by {} image",
            self.width,
            self.height
        );
        (y as usize * self.width as usize + x as usize) * self.format.bytes_per_pixel()
    }

    /// The color of the pixel at `(x, y)`, counted from the top-left.
    /// Channels the format lacks read as LÖVE's do: green and blue as 0,
    /// alpha as 1.
    ///
    /// Panics if the pixel is outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Color {
        let at = self.offset(x, y);
        pixels::decode(
            self.format,
            &self.pixels[at..at + self.format.bytes_per_pixel()],
        )
    }

    /// Sets the color of the pixel at `(x, y)`. Channels the format lacks
    /// are dropped. In a floating-point format, values outside 0 to 1 are
    /// kept; in others they are clamped.
    ///
    /// Panics if the pixel is outside the image.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Color) {
        let at = self.offset(x, y);
        let size = self.format.bytes_per_pixel();
        pixels::encode(self.format, color, &mut self.pixels[at..at + size]);
    }

    /// Replaces every pixel with what `change` returns for its position
    /// and current color, the equivalent of `ImageData:mapPixel`.
    pub fn map_pixels(&mut self, mut change: impl FnMut(u32, u32, Color) -> Color) {
        for y in 0..self.height {
            for x in 0..self.width {
                let color = change(x, y, self.pixel(x, y));
                self.set_pixel(x, y, color);
            }
        }
    }

    /// Copies a `width` by `height` rectangle of `source`, whose top-left
    /// corner is at `(source_x, source_y)`, to `(x, y)` in this image.
    /// Whatever falls outside either image is left out. Pixels are
    /// converted if the two images have different formats.
    #[allow(clippy::too_many_arguments)] // Mirrors ImageData:paste.
    pub fn paste(
        &mut self,
        source: &ImageData,
        x: i32,
        y: i32,
        source_x: u32,
        source_y: u32,
        width: u32,
        height: u32,
    ) {
        for row in 0..height {
            for column in 0..width {
                let (from_x, from_y) = (source_x + column, source_y + row);
                let to_x = i64::from(x) + i64::from(column);
                let to_y = i64::from(y) + i64::from(row);
                let inside_source = from_x < source.width && from_y < source.height;
                let inside_self = (0..i64::from(self.width)).contains(&to_x)
                    && (0..i64::from(self.height)).contains(&to_y);
                if inside_source && inside_self {
                    let (to_x, to_y) = (to_x as u32, to_y as u32);
                    if source.format == self.format {
                        let size = self.format.bytes_per_pixel();
                        let from = source.offset(from_x, from_y);
                        let to = self.offset(to_x, to_y);
                        self.pixels[to..to + size]
                            .copy_from_slice(&source.pixels[from..from + size]);
                    } else {
                        self.set_pixel(to_x, to_y, source.pixel(from_x, from_y));
                    }
                }
            }
        }
    }

    /// The raw pixels, laid out as [`ImageData::from_raw`] takes them for
    /// the image's format. For [`PixelFormat::Rgba8`], as
    /// [`ImageData::from_rgba`] takes them.
    pub fn as_bytes(&self) -> &[u8] {
        &self.pixels
    }

    /// The pixels as four bytes each (red, green, blue, alpha), converted
    /// if the image is in another format. This is what the window system
    /// and array and cube images are given.
    pub(crate) fn rgba8(&self) -> Cow<'_, [u8]> {
        self.bytes_in(PixelFormat::Rgba8)
    }

    /// The pixels in `format`, converted if the image is in another one.
    /// sRGB and linear versions of a format count as the same.
    pub(crate) fn bytes_in(&self, format: PixelFormat) -> Cow<'_, [u8]> {
        if format.to_linear() == self.format.to_linear() {
            return Cow::Borrowed(&self.pixels);
        }
        let (from_size, to_size) = (self.format.bytes_per_pixel(), format.bytes_per_pixel());
        let mut converted = vec![0; self.width as usize * self.height as usize * to_size];
        for (to, from) in converted
            .chunks_exact_mut(to_size)
            .zip(self.pixels.chunks_exact(from_size))
        {
            pixels::encode(format, pixels::decode(self.format, from), to);
        }
        Cow::Owned(converted)
    }

    /// The image as the contents of a file in `format`, the equivalent of
    /// `ImageData:encode`. Works for [`PixelFormat::Rgba8`] images, and
    /// [`PixelFormat::Rgba16`] ones as PNG, as in LÖVE.
    pub fn encode(&self, format: ImageFormat) -> Result<Vec<u8>, ImageError> {
        let color_type = match (self.format, format) {
            (PixelFormat::Rgba8 | PixelFormat::Srgba8, _) => ::image::ExtendedColorType::Rgba8,
            (PixelFormat::Rgba16, ImageFormat::Png) => ::image::ExtendedColorType::Rgba16,
            (pixel_format, _) => {
                return Err(ImageError::Invalid(format!(
                    "{pixel_format} pixels cannot be saved as {format:?}: \
                     only rgba8, and rgba16 as PNG, can"
                )));
            }
        };
        let pixels: Cow<'_, [u8]> = if self.format == PixelFormat::Rgba16 {
            // The encoder takes 16-bit channels in the machine's byte order.
            Cow::Owned(
                self.pixels
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .flat_map(|pair| u16::from_le_bytes([pair[0], pair[1]]).to_ne_bytes())
                    .collect(),
            )
        } else {
            Cow::Borrowed(&self.pixels)
        };
        let format = match format {
            ImageFormat::Png => ::image::ImageFormat::Png,
            ImageFormat::Tga => ::image::ImageFormat::Tga,
        };
        let mut file = Vec::new();
        ::image::write_buffer_with_format(
            &mut std::io::Cursor::new(&mut file),
            &pixels,
            self.width,
            self.height,
            color_type,
            format,
        )
        .map_err(|error| ImageError::Invalid(error.to_string()))?;
        Ok(file)
    }

    /// The image as the contents of a PNG file.
    pub fn encode_png(&self) -> Result<Vec<u8>, ImageError> {
        self.encode(ImageFormat::Png)
    }

    /// Writes the image to a file in the save directory: a TGA file if
    /// `path` ends in `.tga`, and a PNG file otherwise.
    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        let is_tga = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("tga"));
        let format = if is_tga {
            ImageFormat::Tga
        } else {
            ImageFormat::Png
        };
        let file = self.encode(format).map_err(std::io::Error::other)?;
        crate::filesystem::write(path, file)
    }
}

/// A file format [`ImageData::encode`] can write, the equivalent of LÖVE's
/// `ImageFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Png,
    Tga,
}

/// Whether `bytes` are a compressed texture file, which loads as a
/// [`CompressedImageData`](super::CompressedImageData) rather than an
/// [`ImageData`], the equivalent of `love.image.isCompressed`. Smitten
/// reads DDS and KTX files.
pub fn is_compressed(bytes: &[u8]) -> bool {
    super::compressed::is_compressed_file(bytes)
}

impl fmt::Debug for ImageData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageData")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .finish()
    }
}

/// Fails for formats an [`ImageData`] cannot hold.
fn check_format(format: PixelFormat) -> Result<(), ImageError> {
    if format.is_depth() {
        return Err(ImageError::Invalid(format!(
            "image data cannot be in {format}, which only canvases can have"
        )));
    }
    Ok(())
}

impl Image {
    /// Creates an image to draw from pixels in memory. Later changes to
    /// `data` do not show until [`Image::replace_pixels`] is called.
    ///
    /// The image keeps the data's [`PixelFormat`]. It fails if the GPU
    /// cannot sample that format, which
    /// [`graphics::is_image_format_supported`](super::is_image_format_supported)
    /// tells beforehand.
    pub fn from_image_data(data: &ImageData) -> Result<Image, ImageError> {
        Image::from_image_data_with(data, super::ImageSettings::default())
    }

    /// As [`Image::from_image_data`], with options such as mipmaps.
    pub fn from_image_data_with(
        data: &ImageData,
        settings: super::ImageSettings,
    ) -> Result<Image, ImageError> {
        Image::from_pixels(data.format, data.width, data.height, &data.pixels, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };

    #[test]
    fn images_round_trip_through_png_and_tga() {
        let mut data = ImageData::new(3, 2);
        data.set_pixel(0, 0, RED);
        data.set_pixel(2, 1, Color::rgba(0.0, 0.5, 1.0, 0.5));
        for format in [ImageFormat::Png, ImageFormat::Tga] {
            let file = data.encode(format).unwrap();
            let back = ImageData::from_bytes(&file).unwrap();
            assert_eq!(back.dimensions(), (3, 2), "{format:?}");
            assert_eq!(back.as_bytes(), data.as_bytes(), "{format:?}");
        }
    }

    #[test]
    fn only_dds_files_count_as_compressed() {
        let dds = super::super::compressed::dds_file(b"DXT1", 4, 4, &[&[0; 8]]);
        assert!(is_compressed(&dds));
        assert!(!is_compressed(&ImageData::new(1, 1).encode_png().unwrap()));
        assert!(!is_compressed(b""));
    }

    #[test]
    fn new_image_data_is_transparent() {
        let data = ImageData::new(3, 2);
        assert_eq!(data.dimensions(), (3, 2));
        assert_eq!(data.as_bytes().len(), 24);
        assert_eq!(data.pixel(2, 1), Color::rgba(0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn a_pixel_that_is_set_reads_back() {
        let mut data = ImageData::new(4, 4);
        data.set_pixel(1, 2, RED);
        assert_eq!(data.pixel(1, 2), RED);
        assert_eq!(data.pixel(2, 1).a, 0.0);
        // Out-of-range components are clamped, not wrapped.
        data.set_pixel(0, 0, Color::rgba(2.0, -1.0, 0.5, 1.0));
        assert_eq!(&data.as_bytes()[..4], [255, 0, 128, 255]);
    }

    #[test]
    #[should_panic(expected = "outside the 4 by 4 image")]
    fn reading_outside_the_image_panics() {
        ImageData::new(4, 4).pixel(4, 0);
    }

    #[test]
    fn map_pixels_visits_every_pixel_with_its_position() {
        let mut data = ImageData::new(2, 2);
        data.map_pixels(|x, y, _| Color::rgba(x as f32, y as f32, 0.0, 1.0));
        assert_eq!(data.pixel(1, 0), Color::rgba(1.0, 0.0, 0.0, 1.0));
        assert_eq!(data.pixel(0, 1), Color::rgba(0.0, 1.0, 0.0, 1.0));
    }

    #[test]
    fn paste_copies_a_rectangle_and_clips_what_does_not_fit() {
        let mut source = ImageData::new(2, 2);
        source.map_pixels(|_, _, _| RED);
        let mut target = ImageData::new(3, 3);

        target.paste(&source, 2, 2, 0, 0, 2, 2);
        assert_eq!(target.pixel(2, 2), RED);
        assert_eq!(target.pixel(1, 1).a, 0.0);

        target.paste(&source, -1, -1, 0, 0, 2, 2);
        assert_eq!(target.pixel(0, 0), RED);
        assert_eq!(target.pixel(1, 0).a, 0.0);

        // Asking for more than the source holds copies what there is.
        target.paste(&source, 0, 2, 1, 1, 5, 5);
        assert_eq!(target.pixel(0, 2), RED);
        assert_eq!(target.pixel(1, 2).a, 0.0);
    }

    #[test]
    fn a_png_round_trip_keeps_every_pixel() {
        let mut data = ImageData::new(5, 3);
        data.map_pixels(|x, y, _| Color::rgba(x as f32 / 4.0, y as f32 / 2.0, 0.5, 1.0));
        let file = data.encode_png().unwrap();
        assert_eq!(ImageData::from_bytes(&file).unwrap(), data);
        assert!(ImageData::from_bytes(b"not an image").is_err());
    }

    #[test]
    fn image_data_can_hold_other_formats() {
        let mut data = ImageData::with_format(2, 1, PixelFormat::Rgba16f).unwrap();
        assert_eq!(data.format(), PixelFormat::Rgba16f);
        assert_eq!(data.as_bytes().len(), 16);
        let bright = Color::rgba(3.0, 0.5, 0.0, 1.0);
        data.set_pixel(1, 0, bright);
        assert_eq!(data.pixel(1, 0), bright);
        assert_eq!(data.pixel(0, 0), Color::rgba(0.0, 0.0, 0.0, 0.0));

        let red_only = ImageData::with_format(1, 1, PixelFormat::R8).unwrap();
        assert_eq!(red_only.pixel(0, 0), Color::rgba(0.0, 0.0, 0.0, 1.0));
        assert!(ImageData::with_format(1, 1, PixelFormat::Depth24).is_err());
    }

    #[test]
    fn raw_pixels_in_any_format_must_match_the_size() {
        let data = ImageData::from_raw(2, 1, PixelFormat::Rg8, vec![255, 0, 0, 255]).unwrap();
        assert_eq!(data.pixel(0, 0), Color::rgba(1.0, 0.0, 0.0, 1.0));
        assert_eq!(data.pixel(1, 0), Color::rgba(0.0, 1.0, 0.0, 1.0));
        assert!(ImageData::from_raw(2, 1, PixelFormat::Rg8, vec![0; 3]).is_err());
    }

    #[test]
    fn paste_converts_between_formats() {
        let mut source = ImageData::with_format(1, 1, PixelFormat::R32f).unwrap();
        source.set_pixel(0, 0, Color::rgba(0.5, 0.0, 0.0, 1.0));
        let mut target = ImageData::new(1, 1);
        target.paste(&source, 0, 0, 0, 0, 1, 1);
        assert_eq!(target.as_bytes(), [128, 0, 0, 255]);
        // Converted for the GPU in the same way.
        assert_eq!(&*source.rgba8(), [128, 0, 0, 255]);
    }

    #[test]
    fn sixteen_bit_pngs_keep_their_precision() {
        let mut data = ImageData::with_format(2, 1, PixelFormat::Rgba16).unwrap();
        data.set_pixel(0, 0, Color::rgba(1.0 / 65535.0, 0.25, 1.0, 1.0));
        let file = data.encode(ImageFormat::Png).unwrap();
        let back = ImageData::from_bytes(&file).unwrap();
        assert_eq!(back.format(), PixelFormat::Rgba16);
        assert_eq!(back, data);
        // TGA has no 16-bit channels, and floats go in neither file.
        assert!(data.encode(ImageFormat::Tga).is_err());
        let floats = ImageData::with_format(1, 1, PixelFormat::Rgba32f).unwrap();
        assert!(floats.encode(ImageFormat::Png).is_err());
    }

    #[test]
    #[should_panic(expected = "need 16 bytes")]
    fn raw_pixels_must_match_the_size() {
        ImageData::from_rgba(2, 2, vec![0; 15]);
    }
}
