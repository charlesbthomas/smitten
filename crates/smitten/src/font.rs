//! Fonts before they are on the GPU.
//!
//! A [`Rasterizer`] holds a font's characters as outlines or pictures in
//! memory. It measures text and draws single characters into
//! [`GlyphData`], whose pixels can be read or turned into
//! [`ImageData`](crate::graphics::ImageData). Use
//! [`Font::from_rasterizer`](crate::graphics::Font::from_rasterizer) to
//! draw with it.
//!
//! Most games load fonts with [`Font::load`](crate::graphics::Font::load)
//! and never need this module.

use std::path::Path;

pub use crate::graphics::text::{GlyphData, GlyphFormat, Rasterizer};
use crate::graphics::{FontError, Hinting};

impl Rasterizer {
    /// Loads a TrueType or OpenType font file as a rasterizer at `size`
    /// units, or an AngelCode BMFont if the file name ends in `.fnt`.
    pub fn load(path: impl AsRef<Path>, size: f32) -> Result<Rasterizer, FontError> {
        let path = path.as_ref();
        let is_bmfont = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fnt"));
        if is_bmfont {
            Rasterizer::read_bmfont(path)
        } else {
            Rasterizer::load_true_type(path, size, Hinting::Normal, None)
        }
    }
}

impl Rasterizer {
    /// Loads a TrueType or OpenType font file as a rasterizer with every
    /// setting given. See [`Rasterizer::from_bytes`].
    /// [`Rasterizer::default_at`] makes one for the built-in font.
    pub fn load_true_type(
        path: impl AsRef<Path>,
        size: f32,
        hinting: Hinting,
        dpi_scale: Option<f32>,
    ) -> Result<Rasterizer, FontError> {
        let path = path.as_ref();
        let bytes =
            crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
        Rasterizer::from_bytes(&bytes, size, hinting, dpi_scale).map_err(|error| match error {
            FontError::Parse(message) => FontError::Parse(format!("{}: {message}", path.display())),
            other => other,
        })
    }
}

impl Rasterizer {
    /// Loads an image font as a rasterizer. See
    /// [`Font::from_image`](crate::graphics::Font::from_image) for how the
    /// image is laid out.
    pub fn load_image(
        path: impl AsRef<Path>,
        glyphs: &str,
        extra_spacing: f32,
    ) -> Result<Rasterizer, FontError> {
        let path = path.as_ref();
        let bytes =
            crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
        Rasterizer::from_image(&bytes, glyphs, extra_spacing).map_err(|error| match error {
            FontError::Parse(message) => FontError::Parse(format!("{}: {message}", path.display())),
            other => other,
        })
    }
}

impl Rasterizer {
    /// Loads an AngelCode BMFont `.fnt` file and its pages as a rasterizer.
    pub fn load_bmfont(path: impl AsRef<Path>) -> Result<Rasterizer, FontError> {
        Rasterizer::read_bmfont(path.as_ref())
    }
}
