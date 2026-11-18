//! Rasterizers and glyph data, the equivalents of LÖVE's `Rasterizer`
//! and `GlyphData` from `love.font`. A child of the text module so it can
//! share the font's measuring code.

use std::path::Path;

use smitten_core::backend::Backend;

use super::{
    BMFontFile, Bitmaps, DEFAULT_FONT, Face, Font, FontError, Hinting, Origin, Source,
    decode_bmfont, decode_image_font, read_bmfont,
};
use crate::context;
use crate::graphics::ImageData;

/// A font's characters before they are on the GPU, the equivalent of
/// LÖVE's `Rasterizer`. It measures text and draws single characters
/// into [`GlyphData`], and
/// [`Font::from_rasterizer`] turns it into a font.
///
/// Its metrics are in units, as a [`Font`]'s are. A rasterizer can be
/// made without a running game.
#[derive(Clone)]
pub struct Rasterizer {
    face: Face,
    /// The pictures of a bitmap font's characters; `None` for outlines.
    bitmaps: Option<Bitmaps>,
}

/// How the pixels of a [`GlyphData`] are laid out, the part of LÖVE's
/// `PixelFormat` that glyphs use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphFormat {
    /// Two bytes a pixel, luminance then alpha, as TrueType glyphs are.
    /// The luminance is always 255: the alpha is the glyph's coverage.
    La8,
    /// Four bytes a pixel, red, green, blue and alpha, as bitmap font
    /// glyphs are.
    Rgba8,
}

impl GlyphFormat {
    /// How many bytes each pixel takes.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            GlyphFormat::La8 => 2,
            GlyphFormat::Rgba8 => 4,
        }
    }
}

/// The picture and measurements of one character, the equivalent of
/// LÖVE's `GlyphData`. Made by [`Rasterizer::glyph_data`] or
/// [`crate::font::new_glyph_data`].
///
/// Everything here is in the pixels of the picture, which are the
/// rasterizer's units times its DPI scale.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphData {
    glyph: char,
    width: u32,
    height: u32,
    advance: f32,
    bearing_x: f32,
    bearing_y: f32,
    format: GlyphFormat,
    pixels: Vec<u8>,
}

impl GlyphData {
    /// The character this is a picture of.
    pub fn glyph(&self) -> char {
        self.glyph
    }

    /// The character as a string, the equivalent of `getGlyphString`.
    pub fn glyph_string(&self) -> String {
        self.glyph.to_string()
    }

    /// The width of the picture in pixels. A character with nothing to
    /// draw, such as a space, is 0 by 0.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height of the picture in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The width and height of the picture in pixels.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// How far the pen moves after the character, in pixels.
    pub fn advance(&self) -> f32 {
        self.advance
    }

    /// Where the picture's top-left corner is from the pen on the
    /// baseline: how far right, and how far up.
    pub fn bearing(&self) -> (f32, f32) {
        (self.bearing_x, self.bearing_y)
    }

    /// The box the picture covers from the pen on the baseline, as `[x,
    /// y, width, height]`, with `y` how far below the baseline its bottom
    /// reaches, the same as LÖVE's `getBoundingBox`.
    pub fn bounding_box(&self) -> [f32; 4] {
        let min_y = self.height as f32 - self.bearing_y;
        [
            self.bearing_x,
            min_y,
            self.width as f32,
            self.bearing_y - min_y,
        ]
    }

    /// How the pixels are laid out.
    pub fn format(&self) -> GlyphFormat {
        self.format
    }

    /// The pixels, row by row from the top, in [`GlyphData::format`].
    pub fn as_bytes(&self) -> &[u8] {
        &self.pixels
    }

    /// The picture as [`ImageData`], with a TrueType glyph's coverage as
    /// the alpha of white pixels.
    pub fn to_image_data(&self) -> ImageData {
        let rgba = match self.format {
            GlyphFormat::Rgba8 => self.pixels.clone(),
            GlyphFormat::La8 => self
                .pixels
                .chunks_exact(2)
                .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
                .collect(),
        };
        ImageData::from_rgba(self.width, self.height, rgba)
    }
}

/// The scale of the screen, or 1.0 without a running game.
fn screen_scale() -> f32 {
    let mut scale = 1.0;
    context::try_with(|ctx| scale = ctx.gfx.screen.draw_scale());
    scale
}

impl Rasterizer {
    /// A rasterizer for a TrueType or OpenType file's bytes, the
    /// equivalent of `love.font.newTrueTypeRasterizer`: at `size` units,
    /// fitted to pixels as `hinting` says, at `dpi_scale` pixels a unit,
    /// or `None` to match the screen (1.0 without a running game).
    pub fn from_bytes(
        bytes: &[u8],
        size: f32,
        hinting: Hinting,
        dpi_scale: Option<f32>,
    ) -> Result<Rasterizer, FontError> {
        let scale = dpi_scale.unwrap_or_else(screen_scale);
        let face = Face::new(bytes, size, scale, hinting).map_err(FontError::Parse)?;
        Ok(Rasterizer {
            face,
            bitmaps: None,
        })
    }

    /// A rasterizer for the built-in font at `size` units, the equivalent
    /// of `love.font.newTrueTypeRasterizer(size)`.
    pub fn default_at(size: f32) -> Rasterizer {
        Rasterizer::from_bytes(DEFAULT_FONT, size, Hinting::Normal, None)
            .expect("the built-in font is valid")
    }

    /// A rasterizer for an image font, the equivalent of
    /// `love.font.newImageRasterizer`. See [`Font::from_image`] for how
    /// the image is laid out.
    pub fn from_image(
        bytes: &[u8],
        glyphs: &str,
        extra_spacing: f32,
    ) -> Result<Rasterizer, FontError> {
        let image = super::super::image::decode(bytes).map_err(FontError::Parse)?;
        Rasterizer::from_rgba(image, glyphs, extra_spacing)
    }

    /// As [`Rasterizer::from_image`], from pixels already in memory.
    pub fn from_image_data(
        data: &ImageData,
        glyphs: &str,
        extra_spacing: f32,
    ) -> Result<Rasterizer, FontError> {
        let (width, height) = data.dimensions();
        let image = ::image::RgbaImage::from_raw(width, height, data.rgba8().into_owned())
            .ok_or_else(|| FontError::Parse("the image data is the wrong size".into()))?;
        Rasterizer::from_rgba(image, glyphs, extra_spacing)
    }

    fn from_rgba(
        image: ::image::RgbaImage,
        glyphs: &str,
        extra_spacing: f32,
    ) -> Result<Rasterizer, FontError> {
        let (face, bitmaps) =
            decode_image_font(image, glyphs, extra_spacing).map_err(FontError::Parse)?;
        Ok(Rasterizer {
            face,
            bitmaps: Some(bitmaps),
        })
    }

    /// A rasterizer for an AngelCode BMFont, the equivalent of
    /// `love.font.newBMFontRasterizer`: the text of its `.fnt` file and
    /// the bytes of each page image, in the order of their ids.
    pub fn from_bmfont(fnt: &str, pages: &[&[u8]]) -> Result<Rasterizer, FontError> {
        let file = BMFontFile::parse(fnt).map_err(FontError::Parse)?;
        Rasterizer::from_bmfont_file(&file, pages)
    }

    fn from_bmfont_file(file: &BMFontFile, pages: &[&[u8]]) -> Result<Rasterizer, FontError> {
        let (face, bitmaps) = decode_bmfont(file, pages).map_err(FontError::Parse)?;
        Ok(Rasterizer {
            face,
            bitmaps: Some(bitmaps),
        })
    }

    /// Loads a `.fnt` file and its pages through the filesystem.
    pub(crate) fn load_bmfont(path: &Path) -> Result<Rasterizer, FontError> {
        let (file, pages) = read_bmfont(path)?;
        let pages: Vec<&[u8]> = pages.iter().map(Vec::as_slice).collect();
        Rasterizer::from_bmfont_file(&file, &pages).map_err(|error| super::in_file(path, error))
    }

    /// The size the rasterizer was made at, in units.
    pub fn size(&self) -> f32 {
        self.face.size
    }

    /// How many pixels its glyphs hold for each unit.
    pub fn dpi_scale(&self) -> f32 {
        self.face.scale
    }

    /// The height of one line, in units, as [`Font::height`].
    pub fn height(&self) -> f32 {
        self.face.height
    }

    /// The distance from the top of a line to its baseline.
    pub fn ascent(&self) -> f32 {
        self.face.ascent
    }

    /// The distance from the baseline to the bottom of a line.
    pub fn descent(&self) -> f32 {
        self.face.descent
    }

    /// The distance between the baselines of two lines that the font
    /// itself suggests, which for TrueType fonts includes their line gap.
    pub fn line_height(&self) -> f32 {
        match &self.face.source {
            Source::Vector(font) => font
                .horizontal_line_metrics(self.face.size)
                .map_or(self.face.height, |metrics| metrics.new_line_size),
            Source::Bitmap { .. } => self.face.height,
        }
    }

    /// The widest advance of any of its characters, in units, the
    /// equivalent of LÖVE's `Rasterizer:getAdvance`.
    pub fn advance(&self) -> f32 {
        match &self.face.source {
            Source::Vector(font) => (0..font.glyph_count())
                .map(|index| font.metrics_indexed(index, self.face.size).advance_width)
                .fold(0.0, f32::max),
            Source::Bitmap { advances, .. } => advances.values().copied().fold(0.0, f32::max),
        }
    }

    /// How many characters it has.
    pub fn glyph_count(&self) -> usize {
        match &self.face.source {
            Source::Vector(font) => font.glyph_count() as usize,
            Source::Bitmap { advances, .. } => advances.len(),
        }
    }

    /// Whether it has every character of `text`.
    pub fn has_glyphs(&self, text: &str) -> bool {
        text.chars().all(|character| self.face.has_glyph(character))
    }

    /// Draws `character` into a [`GlyphData`]. A character it lacks gives
    /// an empty picture that does not move the pen.
    pub fn glyph_data(&self, character: char) -> GlyphData {
        let empty = |format, advance| GlyphData {
            glyph: character,
            width: 0,
            height: 0,
            advance,
            bearing_x: 0.0,
            bearing_y: 0.0,
            format,
            pixels: Vec::new(),
        };
        match (&self.face.source, &self.bitmaps) {
            (Source::Vector(font), _) => {
                let (metrics, coverage) =
                    font.rasterize(character, self.face.size * self.face.scale);
                if font.lookup_glyph_index(character) == 0 {
                    return empty(GlyphFormat::La8, 0.0);
                }
                let mono = self.face.hinting == Hinting::Mono;
                let pixels = coverage
                    .iter()
                    .flat_map(|&alpha| {
                        let alpha = match mono {
                            true if alpha >= 128 => 255,
                            true => 0,
                            false => alpha,
                        };
                        [255, alpha]
                    })
                    .collect();
                GlyphData {
                    glyph: character,
                    width: metrics.width as u32,
                    height: metrics.height as u32,
                    advance: metrics.advance_width,
                    bearing_x: metrics.xmin as f32,
                    bearing_y: (metrics.ymin + metrics.height as i32) as f32,
                    format: GlyphFormat::La8,
                    pixels,
                }
            }
            (Source::Bitmap { advances, .. }, Some(bitmaps)) => {
                let advance = advances.get(&character).copied().unwrap_or(0.0);
                let Some(Some(glyph)) = bitmaps.glyphs.get(&character) else {
                    return empty(GlyphFormat::Rgba8, advance);
                };
                let page = &bitmaps.pages[glyph.page];
                let mut pixels = Vec::with_capacity((glyph.width * glyph.height * 4) as usize);
                for y in glyph.y..glyph.y + glyph.height {
                    for x in glyph.x..glyph.x + glyph.width {
                        pixels.extend_from_slice(&page.get_pixel(x, y).0);
                    }
                }
                GlyphData {
                    glyph: character,
                    width: glyph.width,
                    height: glyph.height,
                    advance,
                    bearing_x: glyph.left,
                    bearing_y: -glyph.top,
                    format: GlyphFormat::Rgba8,
                    pixels,
                }
            }
            (Source::Bitmap { .. }, None) => empty(GlyphFormat::Rgba8, 0.0),
        }
    }

    /// Puts the rasterizer's characters on the GPU as a font.
    fn upload(&self, backend: &mut dyn Backend) -> Result<super::Atlas, String> {
        match &self.bitmaps {
            Some(bitmaps) => bitmaps.upload(backend),
            None => Ok(super::Atlas::new(self.face.size * self.face.scale)),
        }
    }
}

impl std::fmt::Debug for Rasterizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rasterizer")
            .field("size", &self.face.size)
            .field("height", &self.face.height)
            .field("glyphs", &self.glyph_count())
            .finish()
    }
}

impl Font {
    /// Creates a font from a rasterizer, the equivalent of
    /// `love.graphics.newFont(rasterizer)`.
    pub fn from_rasterizer(rasterizer: &Rasterizer) -> Result<Font, FontError> {
        let (filter, atlas) = context::with(|ctx| {
            (
                ctx.gfx.default_filter,
                rasterizer.upload(ctx.backend.as_mut()),
            )
        });
        let atlas = atlas.map_err(FontError::Parse)?;
        Ok(Font::assemble(
            rasterizer.face.clone(),
            atlas,
            Origin::Rasterizer,
            filter,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_rasterizer_measures_like_its_font() {
        let rasterizer = Rasterizer::default_at(16.0);
        assert_eq!(rasterizer.size(), 16.0);
        assert_eq!(rasterizer.dpi_scale(), 1.0);
        assert!(rasterizer.ascent() > 0.0 && rasterizer.descent() > 0.0);
        assert!(rasterizer.line_height() >= rasterizer.ascent() + rasterizer.descent());
        assert!(rasterizer.glyph_count() > 200);
        assert!(rasterizer.advance() >= rasterizer.glyph_data('W').advance());
        assert!(rasterizer.has_glyphs("Hello"));
        assert!(!rasterizer.has_glyphs("\u{4E00}"));
    }

    #[test]
    fn a_true_type_glyph_is_luminance_and_alpha() {
        let rasterizer = Rasterizer::default_at(16.0);
        let glyph = rasterizer.glyph_data('A');
        assert_eq!(glyph.glyph(), 'A');
        assert_eq!(glyph.glyph_string(), "A");
        assert_eq!(glyph.format(), GlyphFormat::La8);
        let (width, height) = glyph.dimensions();
        assert!(width > 0 && height > 0);
        assert_eq!(glyph.as_bytes().len(), (width * height * 2) as usize);
        // An A sits on the baseline and rises above it.
        let (_, bearing_y) = glyph.bearing();
        assert!(bearing_y > 0.0 && bearing_y <= height as f32 + 1.0);
        let [_, min_y, box_width, box_height] = glyph.bounding_box();
        assert_eq!(box_width, width as f32);
        assert_eq!(box_height, bearing_y - min_y);
        assert!(glyph.as_bytes().chunks(2).any(|pixel| pixel[1] == 255));
        let image = glyph.to_image_data();
        assert_eq!(image.dimensions(), (width, height));
    }

    #[test]
    fn a_space_or_missing_character_is_an_empty_picture() {
        let rasterizer = Rasterizer::default_at(16.0);
        let space = rasterizer.glyph_data(' ');
        assert_eq!(space.dimensions(), (0, 0));
        assert!(space.advance() > 0.0);
        let missing = rasterizer.glyph_data('\u{4E00}');
        assert_eq!((missing.dimensions(), missing.advance()), ((0, 0), 0.0));
    }

    #[test]
    fn mono_hinting_draws_pixels_fully_on_or_off() {
        let rasterizer = Rasterizer::from_bytes(DEFAULT_FONT, 16.0, Hinting::Mono, None).unwrap();
        let glyph = rasterizer.glyph_data('o');
        assert!(
            glyph
                .as_bytes()
                .chunks(2)
                .all(|pixel| pixel[1] == 0 || pixel[1] == 255)
        );
    }

    /// An image font one row of 2 by 3 pixels: magenta spacers around a
    /// 2-wide red A and a 1-wide blue B.
    fn image_font() -> ImageData {
        let spacer = [255, 0, 255, 255];
        let (red, blue) = ([255, 0, 0, 255], [0, 0, 255, 255]);
        let row = [spacer, red, red, spacer, blue, spacer];
        let rgba: Vec<u8> = (0..3).flat_map(|_| row.iter().flatten().copied()).collect();
        ImageData::from_rgba(6, 3, rgba)
    }

    #[test]
    fn an_image_rasterizer_cuts_out_its_glyphs() {
        let rasterizer = Rasterizer::from_image_data(&image_font(), "AB", 1.0).unwrap();
        assert_eq!(rasterizer.glyph_count(), 2);
        assert_eq!((rasterizer.height(), rasterizer.ascent()), (3.0, 3.0));
        // The widest advance: two pixels and the extra spacing.
        assert_eq!(rasterizer.advance(), 3.0);
        let a = rasterizer.glyph_data('A');
        assert_eq!(a.format(), GlyphFormat::Rgba8);
        assert_eq!((a.dimensions(), a.advance()), ((2, 3), 3.0));
        assert_eq!(a.bearing(), (0.0, 3.0));
        assert!(
            a.as_bytes()
                .chunks(4)
                .all(|pixel| pixel == [255, 0, 0, 255])
        );
        let b = rasterizer.glyph_data('B');
        assert_eq!(&b.as_bytes()[..4], [0, 0, 255, 255]);
        assert_eq!(rasterizer.glyph_data('C').dimensions(), (0, 0));
    }

    #[test]
    fn a_bmfont_rasterizer_reads_its_pages() {
        let page = ImageData::from_rgba(
            4,
            4,
            (0..16).flat_map(|index| [index as u8, 0, 0, 255]).collect(),
        )
        .encode_png()
        .unwrap();
        let fnt = "common lineHeight=6 base=4\npage id=0 file=\"p.png\"\n\
                   char id=65 x=1 y=1 width=2 height=3 xoffset=1 yoffset=2 xadvance=5 page=0\n\
                   char id=32 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=3 page=0\n";
        let rasterizer = Rasterizer::from_bmfont(fnt, &[&page]).unwrap();
        assert_eq!(
            (
                rasterizer.height(),
                rasterizer.ascent(),
                rasterizer.descent()
            ),
            (6.0, 4.0, 2.0)
        );
        let a = rasterizer.glyph_data('A');
        assert_eq!(a.dimensions(), (2, 3));
        assert_eq!((a.bearing(), a.advance()), ((1.0, 2.0), 5.0));
        // The red channel counts the page's pixels: (1, 1) is the sixth.
        assert_eq!(a.as_bytes()[0], 5);
        assert_eq!(a.as_bytes()[4 * 2], 9);
        assert_eq!(rasterizer.glyph_data(' ').advance(), 3.0);
        // A character reaching off the page is an error.
        let bad = fnt.replace("width=2", "width=9");
        assert!(Rasterizer::from_bmfont(&bad, &[&page]).is_err());
    }
}
