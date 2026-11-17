//! Fonts and text layout.
//!
//! A [`Font`] is a typeface at one size. Its glyphs are drawn into a
//! texture atlas the first time each is needed, so printing text costs one
//! textured quad per character and usually a single draw call.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use smitten_core::Filter;
use smitten_core::backend::{Backend, TextureId};

use crate::context;
use crate::reload::Asset;

/// The typeface LÖVE also uses by default.
static DEFAULT_FONT: &[u8] = include_bytes!("fonts/Vera.ttf");

/// The size of LÖVE's default font.
pub(crate) const DEFAULT_FONT_SIZE: f32 = 12.0;

/// The size of the text on the error screen.
pub(crate) const ERROR_FONT_SIZE: f32 = 14.0;

/// Empty pixels kept around each glyph in the atlas so that neighbours do
/// not bleed into each other when text is scaled or rotated.
const ATLAS_PADDING: u32 = 1;

/// How text is placed within the width given to [`crate::graphics::printf`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// A typeface at one size, the equivalent of LÖVE's `Font`.
///
/// Cloning is cheap and gives another handle to the same font.
#[derive(Clone)]
pub struct Font {
    inner: Rc<Inner>,
}

pub(crate) struct Inner {
    // In cells so a changed file can be loaded into every handle at once.
    face: RefCell<Face>,
    atlas: RefCell<Atlas>,
    /// What a bitmap font needs besides its image to be built again.
    image_layout: Option<(String, f32)>,
    filter: Cell<Filter>,
    line_height: Cell<f32>,
    /// Counts the times the glyphs were replaced, so a [`Text`] laid out
    /// with the old ones knows to lay itself out again.
    ///
    /// [`Text`]: super::Text
    generation: Cell<u32>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let pages = std::mem::take(&mut self.atlas.get_mut().pages);
        // If the game has already shut down, the backend freed everything.
        context::try_with(|ctx| {
            for page in &pages {
                ctx.backend.destroy_texture(page.texture);
            }
        });
    }
}

impl Font {
    /// Creates a font from the bytes of a TrueType or OpenType file.
    /// `size` is the height of the font in units.
    ///
    /// Its glyphs are drawn as sharp as the screen, so text stays crisp on
    /// a high-DPI display.
    pub fn from_bytes(bytes: &[u8], size: f32) -> Result<Font, FontError> {
        let (filter, scale) =
            context::with(|ctx| (ctx.gfx.default_filter, ctx.gfx.screen.draw_scale()));
        Font::with_filter(bytes, size, filter, scale)
    }

    /// As [`Font::from_bytes`], with glyphs drawn at `dpi_scale` pixels
    /// for each unit. Use a higher scale for text that will be enlarged.
    pub fn with_dpi_scale(bytes: &[u8], size: f32, dpi_scale: f32) -> Result<Font, FontError> {
        let filter = context::with(|ctx| ctx.gfx.default_filter);
        Font::with_filter(bytes, size, filter, dpi_scale)
    }

    /// As [`Font::from_bytes`], for use where the framework state is
    /// already borrowed and its settings are at hand.
    pub(crate) fn with_filter(
        bytes: &[u8],
        size: f32,
        filter: Filter,
        scale: f32,
    ) -> Result<Font, FontError> {
        let face = Face::new(bytes, size, scale).map_err(FontError::Parse)?;
        Ok(Font {
            inner: Rc::new(Inner {
                atlas: RefCell::new(Atlas::new(face.size * face.scale)),
                face: RefCell::new(face),
                image_layout: None,
                filter: Cell::new(filter),
                line_height: Cell::new(1.0),
                generation: Cell::new(0),
            }),
        })
    }

    /// Creates a bitmap font from the bytes of an image file, the
    /// equivalent of `love.graphics.newImageFont`.
    ///
    /// The image holds the characters of `glyphs` in a row from left to
    /// right. The color of its top-left pixel is the spacer: a column of
    /// that color separates each character from the next, and the image
    /// starts with one. A character can be any width.
    ///
    /// `extra_spacing` is added after each character, in units, and may be
    /// negative. One pixel of the image is one unit.
    pub fn from_image(bytes: &[u8], glyphs: &str, extra_spacing: f32) -> Result<Font, FontError> {
        let (filter, built) = context::with(|ctx| {
            (
                ctx.gfx.default_filter,
                build_image_font(ctx.backend.as_mut(), bytes, glyphs, extra_spacing),
            )
        });
        let (face, atlas) = built.map_err(FontError::Parse)?;
        Ok(Font {
            inner: Rc::new(Inner {
                face: RefCell::new(face),
                atlas: RefCell::new(atlas),
                image_layout: Some((glyphs.to_string(), extra_spacing)),
                filter: Cell::new(filter),
                line_height: Cell::new(1.0),
                generation: Cell::new(0),
            }),
        })
    }

    /// Has `asset` loaded again when the file at `path` changes.
    pub(crate) fn watch(&self, path: &Path) {
        let file = crate::filesystem::resolve(path);
        context::with(|ctx| {
            ctx.reload
                .watch(file, Asset::Font(Rc::downgrade(&self.inner)));
        });
    }

    /// Creates the built-in font, Bitstream Vera Sans, at `size` units.
    pub fn default_at(size: f32) -> Font {
        Font::from_bytes(DEFAULT_FONT, size).expect("the built-in font is valid")
    }

    /// The built-in font at its default size, used when the game draws
    /// text without choosing a font.
    pub(crate) fn fallback(filter: Filter, scale: f32) -> Font {
        Font::fallback_at(DEFAULT_FONT_SIZE, filter, scale)
    }

    /// The built-in font at `size` units, for use where the framework
    /// state is already borrowed.
    pub(crate) fn fallback_at(size: f32, filter: Filter, scale: f32) -> Font {
        Font::with_filter(DEFAULT_FONT, size, filter, scale).expect("the built-in font is valid")
    }

    /// The size the font was created with, in units.
    pub fn size(&self) -> f32 {
        self.inner.face.borrow().size
    }

    /// How many pixels the font's glyphs hold for each unit.
    pub fn dpi_scale(&self) -> f32 {
        self.inner.face.borrow().scale
    }

    /// The height of one line of text in units, before line height scaling.
    pub fn height(&self) -> f32 {
        self.inner.face.borrow().height
    }

    /// The distance from the top of a line to its baseline.
    pub fn ascent(&self) -> f32 {
        self.inner.face.borrow().ascent
    }

    /// The distance from the baseline to the bottom of a line.
    pub fn descent(&self) -> f32 {
        self.inner.face.borrow().descent
    }

    /// The spacing between lines as a multiple of [`Font::height`].
    pub fn line_height(&self) -> f32 {
        self.inner.line_height.get()
    }

    /// Sets the spacing between lines as a multiple of [`Font::height`].
    /// The default is 1.0.
    pub fn set_line_height(&self, line_height: f32) {
        self.inner.line_height.set(line_height);
    }

    /// The width in units of `text`. For several lines, the widest one.
    pub fn width(&self, text: &str) -> f32 {
        text.split('\n')
            .map(|line| self.inner.face.borrow().line_width(line))
            .fold(0.0, f32::max)
    }

    /// Breaks `text` into lines no wider than `limit` units.
    ///
    /// Returns the width of the widest line and the lines themselves.
    pub fn wrap(&self, text: &str, limit: f32) -> (f32, Vec<String>) {
        let lines = self.inner.face.borrow().wrap(text, limit);
        let width = lines
            .iter()
            .map(|line| self.inner.face.borrow().line_width(line))
            .fold(0.0, f32::max);
        (width, lines)
    }

    /// How the font is sampled when scaled or rotated.
    pub fn filter(&self) -> Filter {
        self.inner.filter.get()
    }

    /// Sets the filter for every handle to this font.
    pub fn set_filter(&self, filter: Filter) {
        self.inner.filter.set(filter);
    }

    /// Changes whenever a layout made before now may be out of date: the
    /// font was loaded again, or its line height changed.
    pub(crate) fn layout_key(&self) -> (u32, u32) {
        (
            self.inner.generation.get(),
            self.inner.line_height.get().to_bits(),
        )
    }

    /// Whether `self` and `other` are handles to the same font.
    pub(crate) fn is(&self, other: &Font) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    /// Lays out `text` and calls `emit` for every visible glyph with its
    /// atlas texture, its rectangle in text coordinates (origin at the
    /// top-left of the first line), and its texture coordinates.
    pub(crate) fn layout(
        &self,
        backend: &mut dyn Backend,
        text: &str,
        wrap: Option<(f32, Align)>,
        mut emit: impl FnMut(TextureId, [f32; 4], [f32; 4]),
    ) {
        let face = &*self.inner.face.borrow();
        let mut atlas = self.inner.atlas.borrow_mut();
        let lines = match wrap {
            Some((limit, _)) => face.wrap(text, limit),
            None => text.split('\n').map(str::to_string).collect(),
        };
        // Snapping to whole pixels keeps unscaled text sharp.
        let snap = |value: f32| (value * face.scale).round() / face.scale;
        let line_advance = snap(face.height * self.inner.line_height.get());

        for (index, line) in lines.iter().enumerate() {
            let start_x = match wrap {
                Some((limit, Align::Center)) => snap((limit - face.line_width(line)) / 2.0),
                Some((limit, Align::Right)) => snap(limit - face.line_width(line)),
                _ => 0.0,
            };
            let baseline = index as f32 * line_advance + snap(face.ascent);

            let mut pen = start_x;
            let mut previous = None;
            for character in line.chars() {
                if let Some(previous) = previous {
                    pen += face.kerning(previous, character);
                }
                if let Some(glyph) = atlas.glyph(face, backend, character) {
                    let left = snap(pen + glyph.left);
                    let top = baseline + glyph.top;
                    emit(
                        atlas.pages[glyph.page].texture,
                        [left, top, left + glyph.width, top + glyph.height],
                        glyph.uv,
                    );
                }
                pen += face.advance(character);
                previous = Some(character);
            }
        }
    }
}

impl fmt::Debug for Font {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Font")
            .field("size", &self.inner.face.borrow().size)
            .field("height", &self.inner.face.borrow().height)
            .finish()
    }
}

/// Why a font could not be created.
#[derive(Debug)]
pub enum FontError {
    /// The file could not be read.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The data is not a font Smitten can use.
    Parse(String),
}

impl FontError {
    pub(crate) fn read(path: &Path, source: std::io::Error) -> Self {
        FontError::Read {
            path: path.to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontError::Read { path, source } => {
                write!(f, "could not read font {}: {source}", path.display())
            }
            FontError::Parse(message) => write!(f, "could not load font: {message}"),
        }
    }
}

impl std::error::Error for FontError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FontError::Read { source, .. } => Some(source),
            FontError::Parse(_) => None,
        }
    }
}

/// Builds a bitmap font from its image: the measurements, and an atlas
/// whose one page is the image itself.
fn build_image_font(
    backend: &mut dyn Backend,
    bytes: &[u8],
    glyphs: &str,
    extra_spacing: f32,
) -> Result<(Face, Atlas), String> {
    let decoded = super::image::decode(bytes)?;
    let (width, height) = (decoded.width(), decoded.height());
    let rgba = decoded.into_raw();
    let cells = slice_glyphs(width, height, &rgba, glyphs)?;
    let texture = backend.create_texture(width, height, &rgba)?;

    let mut advances = HashMap::new();
    let mut known = HashMap::new();
    for cell in cells {
        advances.insert(cell.character, cell.width as f32 + extra_spacing);
        known.insert(
            cell.character,
            Some(Glyph {
                page: 0,
                uv: [
                    cell.x as f32 / width as f32,
                    0.0,
                    (cell.x + cell.width) as f32 / width as f32,
                    1.0,
                ],
                width: cell.width as f32,
                height: height as f32,
                left: 0.0,
                top: -(height as f32),
            }),
        );
    }
    let face = Face {
        source: Source::Image(advances),
        size: height as f32,
        scale: 1.0,
        ascent: height as f32,
        descent: 0.0,
        height: height as f32,
    };
    let atlas = Atlas {
        page_size: width.max(height),
        pages: vec![Page {
            texture,
            cursor_x: width,
            cursor_y: height,
            row_height: 0,
        }],
        glyphs: known,
    };
    Ok((face, atlas))
}

/// Replaces a font's glyphs with a newer version of its file, keeping its
/// size. Every handle to the font changes at once.
pub(crate) fn reload(inner: &Inner, backend: &mut dyn Backend, bytes: &[u8]) -> Result<(), String> {
    let (face, atlas) = match &inner.image_layout {
        Some((glyphs, extra_spacing)) => build_image_font(backend, bytes, glyphs, *extra_spacing)?,
        None => {
            let (size, scale) = {
                let face = inner.face.borrow();
                (face.size, face.scale)
            };
            let face = Face::new(bytes, size, scale)?;
            let atlas = Atlas::new(face.size * face.scale);
            (face, atlas)
        }
    };
    *inner.face.borrow_mut() = face;
    let old = inner.atlas.replace(atlas);
    inner.generation.set(inner.generation.get().wrapping_add(1));
    for page in &old.pages {
        backend.destroy_texture(page.texture);
    }
    Ok(())
}

/// The measuring half of a font: metrics and line breaking, no GPU.
struct Face {
    source: Source,
    /// In units. All the metrics here are in units too.
    size: f32,
    /// Pixels per unit that glyphs are drawn at.
    scale: f32,
    ascent: f32,
    descent: f32,
    height: f32,
}

/// Where a font's characters come from.
enum Source {
    /// Outlines from a TrueType or OpenType file, drawn when first used.
    Vector(fontdue::Font),
    /// A picture of every character. Holds how far each one moves the pen.
    Image(HashMap<char, f32>),
}

impl Face {
    fn new(bytes: &[u8], size: f32, scale: f32) -> Result<Self, String> {
        if !(size.is_finite() && size > 0.0) {
            return Err(format!("the font size must be positive, got {size}"));
        }
        if !(scale.is_finite() && scale > 0.0) {
            return Err(format!("the DPI scale must be positive, got {scale}"));
        }
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(str::to_string)?;
        let metrics = font
            .horizontal_line_metrics(size)
            .ok_or("the font has no horizontal metrics")?;
        // fontdue reports the descent as a negative number.
        let (ascent, descent) = (metrics.ascent, -metrics.descent);
        Ok(Self {
            source: Source::Vector(font),
            size,
            scale,
            ascent,
            descent,
            height: (ascent + descent).ceil(),
        })
    }

    fn advance(&self, character: char) -> f32 {
        match &self.source {
            _ if character.is_control() => 0.0,
            Source::Vector(font) => font.metrics(character, self.size).advance_width,
            // A character the picture lacks takes no room, as it draws
            // nothing.
            Source::Image(advances) => advances.get(&character).copied().unwrap_or(0.0),
        }
    }

    fn kerning(&self, left: char, right: char) -> f32 {
        match &self.source {
            Source::Vector(font) => font.horizontal_kern(left, right, self.size).unwrap_or(0.0),
            Source::Image(_) => 0.0,
        }
    }

    /// The width of text that contains no line breaks.
    fn line_width(&self, line: &str) -> f32 {
        let mut width = 0.0;
        let mut previous = None;
        for character in line.chars() {
            if let Some(previous) = previous {
                width += self.kerning(previous, character);
            }
            width += self.advance(character);
            previous = Some(character);
        }
        width
    }

    /// Breaks text at spaces so no line is wider than `limit`. Existing
    /// line breaks are kept, and a single word wider than the limit is
    /// split between characters.
    fn wrap(&self, text: &str, limit: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split(' ') {
                let candidate = if line.is_empty() {
                    word.to_string()
                } else {
                    format!("{line} {word}")
                };
                if self.line_width(&candidate) <= limit {
                    line = candidate;
                    continue;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                // The word starts a new line; split it if it is too wide
                // even on its own.
                for character in word.chars() {
                    let mut longer = line.clone();
                    longer.push(character);
                    if !line.is_empty() && self.line_width(&longer) > limit {
                        lines.push(std::mem::replace(&mut line, character.to_string()));
                    } else {
                        line = longer;
                    }
                }
            }
            lines.push(line);
        }
        lines
    }
}

/// One character's columns in a bitmap font's image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    character: char,
    x: u32,
    width: u32,
}

/// Finds each character of `glyphs` in a bitmap font's image by reading
/// its top row: columns in the color of the top-left pixel are spacers,
/// and each run of other columns between them is the next character.
fn slice_glyphs(width: u32, height: u32, rgba: &[u8], glyphs: &str) -> Result<Vec<Slot>, String> {
    if width == 0 || height == 0 {
        return Err("the font image is empty".to_string());
    }
    let pixel = |x: u32| &rgba[x as usize * 4..x as usize * 4 + 4];
    let spacer = pixel(0);

    let mut characters = glyphs.chars();
    let mut cells = Vec::new();
    let mut x = 0;
    while x < width {
        if pixel(x) == spacer {
            x += 1;
            continue;
        }
        let start = x;
        while x < width && pixel(x) != spacer {
            x += 1;
        }
        // Columns beyond the last character named are ignored.
        let Some(character) = characters.next() else {
            break;
        };
        cells.push(Slot {
            character,
            x: start,
            width: x - start,
        });
    }

    let missing = characters.count();
    if missing > 0 {
        return Err(format!(
            "the font image holds {} characters but {} were named; check that a \
             column in the color of the top-left pixel separates each one",
            cells.len(),
            cells.len() + missing
        ));
    }
    Ok(cells)
}

/// Where one glyph sits in the atlas and relative to the pen.
#[derive(Clone, Copy)]
struct Glyph {
    page: usize,
    /// Left, top, right, bottom in texture coordinates.
    uv: [f32; 4],
    width: f32,
    height: f32,
    /// Offset from the pen position to the bitmap's left edge.
    left: f32,
    /// Offset from the baseline to the bitmap's top edge; negative is up.
    top: f32,
}

/// One atlas texture, filled left to right in rows.
struct Page {
    texture: TextureId,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

struct Atlas {
    page_size: u32,
    pages: Vec<Page>,
    /// `None` records a character with nothing to draw, such as a space.
    glyphs: HashMap<char, Option<Glyph>>,
}

impl Atlas {
    fn new(font_size: f32) -> Self {
        // Room for a few hundred glyphs per page at typical sizes.
        let page_size = ((font_size.ceil() as u32).saturating_mul(8))
            .next_power_of_two()
            .clamp(256, 2048);
        Self {
            page_size,
            pages: Vec::new(),
            glyphs: HashMap::new(),
        }
    }

    /// Returns the glyph for `character`, drawing it into the atlas first
    /// if this is the first time it is used.
    fn glyph(&mut self, face: &Face, backend: &mut dyn Backend, character: char) -> Option<Glyph> {
        if let Some(&known) = self.glyphs.get(&character) {
            return known;
        }
        let glyph = self.rasterize(face, backend, character);
        self.glyphs.insert(character, glyph);
        glyph
    }

    fn rasterize(
        &mut self,
        face: &Face,
        backend: &mut dyn Backend,
        character: char,
    ) -> Option<Glyph> {
        if character.is_control() {
            return None;
        }
        // A bitmap font has all its glyphs from the start.
        let Source::Vector(font) = &face.source else {
            return None;
        };
        let (metrics, coverage) = font.rasterize(character, face.size * face.scale);
        let (width, height) = (metrics.width as u32, metrics.height as u32);
        if width == 0 || height == 0 {
            return None;
        }

        let (page, x, y) = self.allocate(backend, width, height)?;
        // White pixels whose alpha is the glyph's coverage, so the current
        // color tints text the same way it tints images.
        let rgba: Vec<u8> = coverage
            .iter()
            .flat_map(|&alpha| [255, 255, 255, alpha])
            .collect();
        backend
            .update_texture(self.pages[page].texture, x, y, width, height, &rgba)
            .ok()?;

        let size = self.page_size as f32;
        Some(Glyph {
            page,
            uv: [
                x as f32 / size,
                y as f32 / size,
                (x + width) as f32 / size,
                (y + height) as f32 / size,
            ],
            // The bitmap is in pixels; the layout works in units.
            width: width as f32 / face.scale,
            height: height as f32 / face.scale,
            left: metrics.xmin as f32 / face.scale,
            // fontdue measures up from the baseline to the bitmap's bottom.
            top: -(metrics.ymin as f32 + height as f32) / face.scale,
        })
    }

    /// Finds room for a `width` by `height` bitmap, adding a page if needed.
    /// Returns the page index and the bitmap's top-left corner.
    fn allocate(
        &mut self,
        backend: &mut dyn Backend,
        width: u32,
        height: u32,
    ) -> Option<(usize, u32, u32)> {
        let (padded_width, padded_height) = (width + ATLAS_PADDING, height + ATLAS_PADDING);
        if padded_width > self.page_size || padded_height > self.page_size {
            return None;
        }

        let last = self.pages.len().wrapping_sub(1);
        if let Some(page) = self.pages.last_mut() {
            if page.cursor_x + padded_width > self.page_size {
                // This row is full; start the next one.
                page.cursor_x = 0;
                page.cursor_y += page.row_height;
                page.row_height = 0;
            }
            if page.cursor_y + padded_height <= self.page_size {
                let position = (last, page.cursor_x, page.cursor_y);
                page.cursor_x += padded_width;
                page.row_height = page.row_height.max(padded_height);
                return Some(position);
            }
        }

        let blank = vec![0; self.page_size as usize * self.page_size as usize * 4];
        let texture = backend
            .create_texture(self.page_size, self.page_size, &blank)
            .ok()?;
        self.pages.push(Page {
            texture,
            cursor_x: padded_width,
            cursor_y: 0,
            row_height: padded_height,
        });
        Some((self.pages.len() - 1, 0, 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face() -> Face {
        Face::new(DEFAULT_FONT, 16.0, 1.0).expect("the built-in font is valid")
    }

    /// A font image one pixel tall: `#` is a spacer column, a letter is a
    /// column of that character's color, so `#aa#b#` holds a two-pixel
    /// glyph and a one-pixel glyph.
    fn strip(columns: &str) -> Vec<u8> {
        columns
            .bytes()
            .flat_map(|column| match column {
                b'#' => [255, 0, 255, 255],
                other => [other, other, other, 255],
            })
            .collect()
    }

    #[test]
    fn a_font_image_is_cut_at_its_spacer_columns() {
        let columns = "#aaa#b#cc#";
        let cells = slice_glyphs(columns.len() as u32, 1, &strip(columns), "ABC").unwrap();
        let found: Vec<_> = cells
            .iter()
            .map(|cell| (cell.character, cell.x, cell.width))
            .collect();
        assert_eq!(found, [('A', 1, 3), ('B', 5, 1), ('C', 7, 2)]);
    }

    #[test]
    fn a_font_image_may_end_without_a_spacer_or_double_one() {
        let columns = "##aa##bb";
        let cells = slice_glyphs(columns.len() as u32, 1, &strip(columns), "AB").unwrap();
        assert_eq!((cells[0].x, cells[0].width), (2, 2));
        assert_eq!((cells[1].x, cells[1].width), (6, 2));
    }

    #[test]
    fn naming_more_characters_than_the_image_holds_is_an_error() {
        let columns = "#aa#bb#";
        let error = slice_glyphs(columns.len() as u32, 1, &strip(columns), "ABC").unwrap_err();
        assert!(
            error.contains("holds 2 characters but 3 were named"),
            "{error}"
        );
        assert!(slice_glyphs(0, 0, &[], "A").is_err());
    }

    #[test]
    fn spare_columns_in_a_font_image_are_ignored() {
        let columns = "#aa#bb#";
        let cells = slice_glyphs(columns.len() as u32, 1, &strip(columns), "A").unwrap();
        assert_eq!(cells.len(), 1);
    }

    #[test]
    fn a_bitmap_face_measures_with_its_own_widths() {
        let face = Face {
            source: Source::Image(HashMap::from([('A', 4.0), ('B', 2.0)])),
            size: 6.0,
            scale: 1.0,
            ascent: 6.0,
            descent: 0.0,
            height: 6.0,
        };
        assert_eq!(face.line_width("ABA"), 10.0);
        // A character the image lacks takes no room.
        assert_eq!(face.line_width("A?B"), 6.0);
        assert_eq!(face.wrap("AA BB", 8.0), ["AA", "BB"]);
    }

    #[test]
    fn the_built_in_font_has_sensible_metrics() {
        let face = face();
        assert!(face.ascent > 0.0 && face.descent > 0.0);
        assert!(face.height >= face.ascent + face.descent);
        // A line is roughly as tall as the requested size.
        assert!((12.0..=24.0).contains(&face.height));
    }

    #[test]
    fn a_bad_size_or_bad_data_is_an_error() {
        assert!(Face::new(DEFAULT_FONT, 0.0, 1.0).is_err());
        assert!(Face::new(DEFAULT_FONT, f32::NAN, 1.0).is_err());
        assert!(Face::new(b"not a font", 16.0, 1.0).is_err());
    }

    #[test]
    fn longer_text_is_wider() {
        let face = face();
        assert_eq!(face.line_width(""), 0.0);
        assert!(face.line_width("hello") > 0.0);
        assert!(face.line_width("hello world") > face.line_width("hello"));
    }

    #[test]
    fn text_that_fits_stays_on_one_line() {
        assert_eq!(face().wrap("hello world", 1000.0), ["hello world"]);
    }

    #[test]
    fn wrapping_breaks_at_spaces_and_respects_the_limit() {
        let face = face();
        let limit = face.line_width("the quick brown") + 1.0;
        let lines = face.wrap("the quick brown fox jumps", limit);

        assert_eq!(lines, ["the quick brown", "fox jumps"]);
        assert!(lines.iter().all(|line| face.line_width(line) <= limit));
    }

    #[test]
    fn wrapping_keeps_existing_line_breaks() {
        assert_eq!(face().wrap("one\n\ntwo", 1000.0), ["one", "", "two"]);
    }

    #[test]
    fn a_word_wider_than_the_limit_is_split() {
        let face = face();
        let limit = face.line_width("aaaa") + 0.5;
        let lines = face.wrap("aaaaaaaaaa", limit);

        assert_eq!(lines, ["aaaa", "aaaa", "aa"]);
    }

    #[test]
    fn a_limit_too_small_for_one_character_still_makes_progress() {
        assert_eq!(face().wrap("abc", 0.0), ["a", "b", "c"]);
    }
}
