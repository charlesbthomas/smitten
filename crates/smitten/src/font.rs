//! Fonts before they are on the GPU, the equivalent of `love.font`.
//!
//! A [`Rasterizer`] holds a font's characters as outlines or pictures in
//! memory. It measures text and draws single characters into
//! [`GlyphData`], whose pixels can be read or turned into
//! [`ImageData`](crate::graphics::ImageData). Use
//! [`Font::from_rasterizer`](crate::graphics::Font::from_rasterizer) to
//! draw with it.
//!
//! Most games load fonts with [`graphics::new_font`](crate::graphics::new_font)
//! and never need this module.

use std::path::Path;

pub use crate::graphics::{FontError, GlyphData, GlyphFormat, Hinting, Rasterizer};

/// Loads a TrueType or OpenType font file as a rasterizer at `size`
/// units, or an AngelCode BMFont if the file name ends in `.fnt`, the
/// equivalent of `love.font.newRasterizer`.
///
/// Panics with the file name if it cannot be loaded. Use
/// [`try_new_rasterizer`] to handle the failure instead.
pub fn new_rasterizer(path: impl AsRef<Path>, size: f32) -> Rasterizer {
    try_new_rasterizer(path, size).unwrap_or_else(|error| panic!("{error}"))
}

/// As [`new_rasterizer`], returning an error if it fails.
pub fn try_new_rasterizer(path: impl AsRef<Path>, size: f32) -> Result<Rasterizer, FontError> {
    let path = path.as_ref();
    let is_bmfont = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fnt"));
    if is_bmfont {
        Rasterizer::load_bmfont(path)
    } else {
        try_new_true_type_rasterizer(path, size, Hinting::Normal, None)
    }
}

/// Loads a TrueType or OpenType font file as a rasterizer with the full
/// set of LÖVE's `newTrueTypeRasterizer` arguments. See
/// [`Rasterizer::from_bytes`]. [`Rasterizer::default_at`] makes one for
/// the built-in font.
///
/// Panics with the file name if it cannot be loaded. Use
/// [`try_new_true_type_rasterizer`] to handle the failure instead.
pub fn new_true_type_rasterizer(
    path: impl AsRef<Path>,
    size: f32,
    hinting: Hinting,
    dpi_scale: Option<f32>,
) -> Rasterizer {
    try_new_true_type_rasterizer(path, size, hinting, dpi_scale)
        .unwrap_or_else(|error| panic!("{error}"))
}

/// As [`new_true_type_rasterizer`], returning an error if it fails.
pub fn try_new_true_type_rasterizer(
    path: impl AsRef<Path>,
    size: f32,
    hinting: Hinting,
    dpi_scale: Option<f32>,
) -> Result<Rasterizer, FontError> {
    let path = path.as_ref();
    let bytes = crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
    Rasterizer::from_bytes(&bytes, size, hinting, dpi_scale).map_err(|error| match error {
        FontError::Parse(message) => FontError::Parse(format!("{}: {message}", path.display())),
        other => other,
    })
}

/// Loads an image font as a rasterizer, the equivalent of
/// `love.font.newImageRasterizer`. See
/// [`Font::from_image`](crate::graphics::Font::from_image) for how the
/// image is laid out.
///
/// Panics with the file name if it cannot be loaded. Use
/// [`try_new_image_rasterizer`] to handle the failure instead.
pub fn new_image_rasterizer(
    path: impl AsRef<Path>,
    glyphs: &str,
    extra_spacing: f32,
) -> Rasterizer {
    try_new_image_rasterizer(path, glyphs, extra_spacing).unwrap_or_else(|error| panic!("{error}"))
}

/// As [`new_image_rasterizer`], returning an error if it fails.
pub fn try_new_image_rasterizer(
    path: impl AsRef<Path>,
    glyphs: &str,
    extra_spacing: f32,
) -> Result<Rasterizer, FontError> {
    let path = path.as_ref();
    let bytes = crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
    Rasterizer::from_image(&bytes, glyphs, extra_spacing).map_err(|error| match error {
        FontError::Parse(message) => FontError::Parse(format!("{}: {message}", path.display())),
        other => other,
    })
}

/// Loads an AngelCode BMFont `.fnt` file and its pages as a rasterizer,
/// the equivalent of `love.font.newBMFontRasterizer`.
///
/// Panics with the file name if it cannot be loaded. Use
/// [`try_new_bmfont_rasterizer`] to handle the failure instead.
pub fn new_bmfont_rasterizer(path: impl AsRef<Path>) -> Rasterizer {
    try_new_bmfont_rasterizer(path).unwrap_or_else(|error| panic!("{error}"))
}

/// As [`new_bmfont_rasterizer`], returning an error if it fails.
pub fn try_new_bmfont_rasterizer(path: impl AsRef<Path>) -> Result<Rasterizer, FontError> {
    Rasterizer::load_bmfont(path.as_ref())
}

/// Draws one character of `rasterizer`, the equivalent of
/// `love.font.newGlyphData`. The same as [`Rasterizer::glyph_data`].
pub fn new_glyph_data(rasterizer: &Rasterizer, glyph: char) -> GlyphData {
    rasterizer.glyph_data(glyph)
}
