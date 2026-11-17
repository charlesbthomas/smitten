//! Pixels held in memory, the equivalent of LÖVE's `ImageData`.
//!
//! An [`Image`] lives on the GPU and can only be drawn. An [`ImageData`]
//! lives in ordinary memory, so its pixels can be read and changed: to
//! generate a texture, to read a level out of a picture, or to check what
//! color is under a point. Turn one into an image with
//! [`Image::from_image_data`] when it is ready to draw.

use std::fmt;
use std::path::Path;

use super::image::{Image, ImageError, decode};
use crate::Color;

/// A grid of pixels that can be read and changed.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageData {
    width: u32,
    height: u32,
    /// Four bytes a pixel (red, green, blue, alpha), in rows from the top.
    pixels: Vec<u8>,
}

fn to_byte(component: f32) -> u8 {
    (component.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl ImageData {
    /// Creates `width` by `height` transparent black pixels.
    pub fn new(width: u32, height: u32) -> ImageData {
        ImageData {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
        }
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
            pixels: rgba,
        }
    }

    /// Decodes the contents of a PNG or JPEG file.
    pub fn from_bytes(bytes: &[u8]) -> Result<ImageData, ImageError> {
        let decoded = decode(bytes).map_err(ImageError::Invalid)?;
        Ok(ImageData {
            width: decoded.width(),
            height: decoded.height(),
            pixels: decoded.into_raw(),
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
        (y as usize * self.width as usize + x as usize) * 4
    }

    /// The color of the pixel at `(x, y)`, counted from the top-left.
    ///
    /// Panics if the pixel is outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Color {
        let at = self.offset(x, y);
        let component = |index: usize| f32::from(self.pixels[at + index]) / 255.0;
        Color::rgba(component(0), component(1), component(2), component(3))
    }

    /// Sets the color of the pixel at `(x, y)`.
    ///
    /// Panics if the pixel is outside the image.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Color) {
        let at = self.offset(x, y);
        self.pixels[at..at + 4].copy_from_slice(&[
            to_byte(color.r),
            to_byte(color.g),
            to_byte(color.b),
            to_byte(color.a),
        ]);
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
    /// Whatever falls outside either image is left out.
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
                    let from = source.offset(from_x, from_y);
                    let to = self.offset(to_x as u32, to_y as u32);
                    self.pixels[to..to + 4].copy_from_slice(&source.pixels[from..from + 4]);
                }
            }
        }
    }

    /// The raw pixels, laid out as [`ImageData::from_rgba`] takes them.
    pub fn as_bytes(&self) -> &[u8] {
        &self.pixels
    }

    /// The image as the contents of a PNG file.
    pub fn encode_png(&self) -> Result<Vec<u8>, ImageError> {
        let mut file = Vec::new();
        ::image::write_buffer_with_format(
            &mut std::io::Cursor::new(&mut file),
            &self.pixels,
            self.width,
            self.height,
            ::image::ExtendedColorType::Rgba8,
            ::image::ImageFormat::Png,
        )
        .map_err(|error| ImageError::Invalid(error.to_string()))?;
        Ok(file)
    }

    /// Writes the image as a PNG file in the save directory.
    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let file = self.encode_png().map_err(std::io::Error::other)?;
        crate::filesystem::write(path, file)
    }
}

impl fmt::Debug for ImageData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageData")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl Image {
    /// Creates an image to draw from pixels in memory. Later changes to
    /// `data` do not show until [`Image::replace_pixels`] is called.
    pub fn from_image_data(data: &ImageData) -> Result<Image, ImageError> {
        Image::from_rgba(data.width, data.height, &data.pixels)
    }

    /// As [`Image::from_image_data`], with options such as mipmaps.
    pub fn from_image_data_with(
        data: &ImageData,
        settings: super::ImageSettings,
    ) -> Result<Image, ImageError> {
        Image::from_rgba_with(data.width, data.height, &data.pixels, settings)
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
    #[should_panic(expected = "need 16 bytes")]
    fn raw_pixels_must_match_the_size() {
        ImageData::from_rgba(2, 2, vec![0; 15]);
    }
}
