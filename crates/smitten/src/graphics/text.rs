//! Fonts and text layout.
//!
//! A [`Font`] is a typeface at one size. Its glyphs are drawn into a
//! texture atlas the first time each is needed, so printing text costs one
//! textured quad per character and usually a single draw call.

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use smitten_core::backend::{Backend, TextureId};
use smitten_core::{Filter, PixelFormat};

use super::stats::{Counted, Kind};
use crate::reload::Asset;
use crate::{Color, context};

#[path = "rasterizer.rs"]
mod rasterizer;
pub use rasterizer::{GlyphData, GlyphFormat, Rasterizer};

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
    /// Lines are stretched to the full width by widening their spaces.
    /// The last line of each paragraph is left as it is, aligned left.
    Justify,
}

/// How a TrueType font's outlines are fitted to the pixel grid, the
/// equivalent of LÖVE's `HintingMode`.
///
/// Smitten draws outlines without hinting, so `Normal`, `Light` and
/// `None` look the same: smooth and true to the outline. `Mono` draws
/// every pixel fully on or off, with no smoothing, which suits pixel-art
/// TrueType fonts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Hinting {
    #[default]
    Normal,
    Light,
    Mono,
    None,
}

/// A typeface at one size, the equivalent of LÖVE's `Font`.
///
/// Cloning is cheap and gives another handle to the same font.
#[derive(Clone)]
pub struct Font {
    inner: Rc<Inner>,
}

identity_eq!(Font, inner);

pub(crate) struct Inner {
    // In cells so a changed file can be loaded into every handle at once.
    face: RefCell<Face>,
    atlas: RefCell<Atlas>,
    /// What the font needs besides its file to be built again.
    origin: Origin,
    /// Fonts that draw the characters this one lacks, in order.
    fallbacks: RefCell<Vec<Font>>,
    filter: Cell<Filter>,
    line_height: Cell<f32>,
    /// Counts the times the glyphs were replaced, so a [`Text`] laid out
    /// with the old ones knows to lay itself out again.
    ///
    /// [`Text`]: super::Text
    generation: Cell<u32>,
    _counted: Counted,
}

/// What kind of file a font was made from, and what else rebuilding it
/// takes.
enum Origin {
    Vector,
    /// An image font: the characters it holds and the spacing after each.
    Image(String, f32),
    /// A BMFont: the folder its page images are found in.
    BMFont(PathBuf),
    /// Made from a [`Rasterizer`], with no file to reload.
    Rasterizer,
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

/// Options for making a font, the arguments LÖVE's `newFont` takes after
/// the size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FontSettings {
    /// How outlines are fitted to pixels.
    pub hinting: Hinting,
    /// How many pixels the glyphs hold for each unit, or `None` to match
    /// the screen. Use a higher scale for text that will be enlarged.
    pub dpi_scale: Option<f32>,
}

impl Font {
    /// Creates a font from the bytes of a TrueType or OpenType file.
    /// `size` is the height of the font in units.
    ///
    /// Its glyphs are drawn as sharp as the screen, so text stays crisp on
    /// a high-DPI display.
    pub fn from_bytes(bytes: &[u8], size: f32) -> Result<Font, FontError> {
        Font::from_bytes_with(bytes, size, FontSettings::default())
    }

    /// Creates a font with the full set of LÖVE's `newFont` arguments:
    /// its size, and how its outlines are fitted to pixels and how sharp
    /// its glyphs are, as [`FontSettings`] says.
    pub fn from_bytes_with(
        bytes: &[u8],
        size: f32,
        settings: FontSettings,
    ) -> Result<Font, FontError> {
        let FontSettings { hinting, dpi_scale } = settings;
        let (filter, scale) =
            context::with(|ctx| (ctx.gfx.default_filter, ctx.gfx.screen.draw_scale()));
        Font::build(bytes, size, filter, dpi_scale.unwrap_or(scale), hinting)
    }

    /// As [`Font::from_bytes`], for use where the framework state is
    /// already borrowed and its settings are at hand.
    pub(crate) fn with_filter(
        bytes: &[u8],
        size: f32,
        filter: Filter,
        scale: f32,
    ) -> Result<Font, FontError> {
        Font::build(bytes, size, filter, scale, Hinting::Normal)
    }

    fn build(
        bytes: &[u8],
        size: f32,
        filter: Filter,
        scale: f32,
        hinting: Hinting,
    ) -> Result<Font, FontError> {
        let face = Face::new(bytes, size, scale, hinting).map_err(FontError::Parse)?;
        let atlas = Atlas::new(face.size * face.scale);
        Ok(Font::assemble(face, atlas, Origin::Vector, filter))
    }

    fn assemble(face: Face, atlas: Atlas, origin: Origin, filter: Filter) -> Font {
        Font {
            inner: Rc::new(Inner {
                face: RefCell::new(face),
                atlas: RefCell::new(atlas),
                origin,
                fallbacks: RefCell::new(Vec::new()),
                filter: Cell::new(filter),
                line_height: Cell::new(1.0),
                generation: Cell::new(0),
                _counted: Counted::new(Kind::Font),
            }),
        }
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
        let origin = Origin::Image(glyphs.to_string(), extra_spacing);
        Ok(Font::assemble(face, atlas, origin, filter))
    }

    /// Creates a font from an AngelCode BMFont: the text of its `.fnt`
    /// file, and the bytes of each page image it names, in the order of
    /// their ids. [`Font::load`](crate::graphics::Font::load) loads
    /// a `.fnt` file and its pages for you.
    ///
    /// Only the text form of `.fnt` files is read, as in LÖVE.
    pub fn from_bmfont(fnt: &str, pages: &[&[u8]]) -> Result<Font, FontError> {
        let file = BMFontFile::parse(fnt).map_err(FontError::Parse)?;
        Font::from_bmfont_file(&file, pages, PathBuf::new())
    }

    fn from_bmfont_file(
        file: &BMFontFile,
        pages: &[&[u8]],
        dir: PathBuf,
    ) -> Result<Font, FontError> {
        let (filter, built) = context::with(|ctx| {
            (
                ctx.gfx.default_filter,
                build_bmfont(ctx.backend.as_mut(), file, pages),
            )
        });
        let (face, atlas) = built.map_err(FontError::Parse)?;
        Ok(Font::assemble(face, atlas, Origin::BMFont(dir), filter))
    }

    /// Loads a `.fnt` file and the page images it names, which are looked
    /// for next to it.
    pub(crate) fn read_bmfont(path: &Path) -> Result<Font, FontError> {
        let (file, pages) = read_bmfont(path)?;
        let pages: Vec<&[u8]> = pages.iter().map(Vec::as_slice).collect();
        let disk_dir = crate::filesystem::resolve(path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        Font::from_bmfont_file(&file, &pages, disk_dir).map_err(|error| in_file(path, error))
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

    /// How the font's outlines are fitted to pixels. Bitmap fonts report
    /// [`Hinting::Normal`].
    pub fn hinting(&self) -> Hinting {
        self.inner.face.borrow().hinting
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

    /// Where the baseline of the first line is drawn, in units below the
    /// top of the text, the equivalent of LÖVE's `Font:getBaseline`.
    pub fn baseline(&self) -> f32 {
        let face = self.inner.face.borrow();
        (face.ascent * face.scale).round() / face.scale
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

    /// How much closer together, in units, `left` and `right` are drawn
    /// when `right` follows `left`. Negative values move them together.
    /// Characters drawn by different fallback fonts are not kerned.
    pub fn kerning(&self, left: char, right: char) -> f32 {
        self.with_faces(|faces| faces.kerning(left, right))
    }

    /// Whether this font or one of its fallbacks can draw `character`.
    pub fn has_glyph(&self, character: char) -> bool {
        self.with_faces(|faces| faces.find(character).is_some())
    }

    /// Whether this font or its fallbacks can draw every character of
    /// `text`, the equivalent of LÖVE's `Font:hasGlyphs`.
    pub fn has_glyphs(&self, text: &str) -> bool {
        self.with_faces(|faces| {
            text.chars()
                .all(|character| faces.find(character).is_some())
        })
    }

    /// Sets the fonts that draw the characters this one lacks, the
    /// equivalent of LÖVE's `Font:setFallbacks`. Each character is drawn
    /// by the first font in the list that has it; one no font has takes
    /// no room. Line heights and the baseline stay this font's own.
    ///
    /// Only this font's own fallbacks are used, not theirs. Fonts that
    /// are each other's fallbacks keep each other alive.
    pub fn set_fallbacks(&self, fallbacks: &[&Font]) {
        let mut list: Vec<Font> = Vec::new();
        for &font in fallbacks {
            if !font.is(self) && !list.iter().any(|known| known.is(font)) {
                list.push(font.clone());
            }
        }
        *self.inner.fallbacks.borrow_mut() = list;
        self.inner
            .generation
            .set(self.inner.generation.get().wrapping_add(1));
    }

    /// The fonts that draw the characters this one lacks.
    pub fn fallbacks(&self) -> Vec<Font> {
        self.inner.fallbacks.borrow().clone()
    }

    /// The width in units of `text`. For several lines, the widest one.
    pub fn width(&self, text: &str) -> f32 {
        self.with_faces(|faces| {
            text.split('\n')
                .map(|line| faces.line_width(line))
                .fold(0.0, f32::max)
        })
    }

    /// Breaks `text` into lines no wider than `limit` units.
    ///
    /// Returns the width of the widest line and the lines themselves.
    pub fn wrap(&self, text: &str, limit: f32) -> (f32, Vec<String>) {
        self.with_faces(|faces| {
            let lines = faces.wrap(text, limit);
            let width = lines
                .iter()
                .map(|line| faces.line_width(&line.text))
                .fold(0.0, f32::max);
            (width, lines.into_iter().map(|line| line.text).collect())
        })
    }

    /// How wide text wrapped to `limit` and aligned by `align` reaches
    /// from its left edge.
    pub(crate) fn aligned_width(&self, text: &str, limit: f32, align: Align) -> f32 {
        self.with_faces(|faces| {
            let lines = faces.wrap(text, limit);
            let widest = lines
                .iter()
                .map(|line| faces.line_width(&line.text))
                .fold(0.0, f32::max);
            match align {
                Align::Left => widest,
                Align::Center => (limit + widest) / 2.0,
                Align::Right => limit.max(widest),
                Align::Justify if lines.iter().any(|line| !line.last) => limit.max(widest),
                Align::Justify => widest,
            }
        })
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
    /// font or one of its fallbacks was loaded again, the fallbacks
    /// changed, or the line height changed.
    pub(crate) fn layout_key(&self) -> (u32, u32) {
        let generation =
            self.inner
                .fallbacks
                .borrow()
                .iter()
                .fold(self.inner.generation.get(), |key, font| {
                    key.wrapping_mul(31)
                        .wrapping_add(font.inner.generation.get().wrapping_add(1))
                });
        (generation, self.inner.line_height.get().to_bits())
    }

    /// Whether `self` and `other` are handles to the same font.
    pub(crate) fn is(&self, other: &Font) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    /// Runs `measure` with this font's face and its fallbacks' faces.
    fn with_faces<R>(&self, measure: impl FnOnce(&Faces) -> R) -> R {
        let primary = self.inner.face.borrow();
        let fallbacks = self.inner.fallbacks.borrow();
        let others: Vec<Ref<Face>> = fallbacks
            .iter()
            .map(|font| font.inner.face.borrow())
            .collect();
        let mut faces = vec![&*primary];
        faces.extend(others.iter().map(|face| &**face));
        measure(&Faces { faces })
    }

    /// Lays out `text` and calls `emit` for every visible glyph with its
    /// character, atlas texture, its rectangle in text coordinates (origin
    /// at the top-left of the first line), and its texture coordinates.
    /// Glyphs come in the order of their characters in `text`.
    pub(crate) fn layout(
        &self,
        backend: &mut dyn Backend,
        text: &str,
        wrap: Option<(f32, Align)>,
        mut emit: impl FnMut(char, TextureId, [f32; 4], [f32; 4]),
    ) {
        let fallbacks = self.inner.fallbacks.borrow();
        let line_height = self.inner.line_height.get();
        self.with_faces(|faces| {
            let mut atlases: Vec<RefMut<Atlas>> = vec![self.inner.atlas.borrow_mut()];
            atlases.extend(fallbacks.iter().map(|font| font.inner.atlas.borrow_mut()));
            let scale = faces.faces[0].scale;
            let snap = |value: f32| (value * scale).round() / scale;
            for placed in faces.place(text, wrap, line_height) {
                let face = faces.faces[placed.face];
                let atlas = &mut atlases[placed.face];
                if let Some(glyph) = atlas.glyph(face, backend, placed.character) {
                    let left = snap(placed.pen + glyph.left);
                    let top = placed.baseline + glyph.top;
                    emit(
                        placed.character,
                        atlas.pages[glyph.page].texture,
                        [left, top, left + glyph.width, top + glyph.height],
                        glyph.uv,
                    );
                }
            }
        });
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

/// Reads a `.fnt` file and the bytes of the page images it names, which
/// are looked for next to it.
fn read_bmfont(path: &Path) -> Result<(BMFontFile, Vec<Vec<u8>>), FontError> {
    let bytes = crate::filesystem::read(path).map_err(|source| FontError::read(path, source))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| in_file(path, FontError::Parse("it is not text".into())))?;
    let file =
        BMFontFile::parse(&text).map_err(|message| in_file(path, FontError::Parse(message)))?;
    let dir = path.parent().unwrap_or(Path::new(""));
    let mut pages = Vec::new();
    for page in &file.pages {
        let page_path = dir.join(page);
        let bytes = crate::filesystem::read(&page_path)
            .map_err(|source| FontError::read(&page_path, source))?;
        pages.push(bytes);
    }
    Ok((file, pages))
}

/// Names the file a parse error was in.
fn in_file(path: &Path, error: FontError) -> FontError {
    match error {
        FontError::Parse(message) => FontError::Parse(format!("{}: {message}", path.display())),
        other => other,
    }
}

/// Why a font could not be created.
#[non_exhaustive]
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
    let image = super::image::decode(bytes)?;
    let (face, bitmaps) = decode_image_font(image, glyphs, extra_spacing)?;
    let atlas = bitmaps.upload(backend)?;
    Ok((face, atlas))
}

/// Measures an image font and finds its characters, without the GPU.
fn decode_image_font(
    image: ::image::RgbaImage,
    glyphs: &str,
    extra_spacing: f32,
) -> Result<(Face, Bitmaps), String> {
    let (width, height) = (image.width(), image.height());
    let cells = slice_glyphs(width, height, image.as_raw(), glyphs)?;
    let mut advances = HashMap::new();
    let mut known = HashMap::new();
    for cell in cells {
        advances.insert(cell.character, cell.width as f32 + extra_spacing);
        known.insert(
            cell.character,
            Some(BitmapGlyph {
                page: 0,
                x: cell.x,
                y: 0,
                width: cell.width,
                height,
                left: 0.0,
                top: -(height as f32),
            }),
        );
    }
    let face = Face::bitmap(advances, HashMap::new(), height as f32, height as f32, 0.0);
    let bitmaps = Bitmaps {
        pages: vec![image],
        glyphs: known,
    };
    Ok((face, bitmaps))
}

/// Puts a page of glyphs on the GPU. Like an image, it is stored
/// sRGB-encoded with gamma-correct rendering, so the colors of a bitmap
/// font's pictures come out as they do drawn as an image. The coverage
/// of a TrueType glyph is in alpha, which is never encoded.
fn new_page(
    backend: &mut dyn Backend,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<TextureId, String> {
    match page_format(backend.is_gamma_correct()) {
        Some(format) => backend.create_texture_in(format, width, height, rgba, false),
        None => backend.create_texture(width, height, rgba),
    }
}

/// The format of a page of glyphs, or `None` for the default RGBA8.
fn page_format(gamma_correct: bool) -> Option<PixelFormat> {
    gamma_correct.then_some(PixelFormat::Srgba8)
}

/// Where one character of a bitmap font is in its pages, in pixels, and
/// how it sits against the pen.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BitmapGlyph {
    page: usize,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// Offset from the pen position to the picture's left edge.
    left: f32,
    /// Offset from the baseline to the picture's top edge; negative is up.
    top: f32,
}

/// The pictures of a bitmap font's characters, before they are on the
/// GPU.
#[derive(Clone)]
struct Bitmaps {
    pages: Vec<::image::RgbaImage>,
    /// `None` records a character with nothing to draw, such as a space.
    glyphs: HashMap<char, Option<BitmapGlyph>>,
}

impl Bitmaps {
    /// Puts the pages on the GPU as an atlas that already holds every
    /// glyph.
    fn upload(&self, backend: &mut dyn Backend) -> Result<Atlas, String> {
        let mut pages: Vec<Page> = Vec::new();
        for image in &self.pages {
            match new_page(backend, image.width(), image.height(), image.as_raw()) {
                Ok(texture) => pages.push(Page::full(texture, image.width(), image.height())),
                Err(error) => {
                    for page in &pages {
                        backend.destroy_texture(page.texture);
                    }
                    return Err(error);
                }
            }
        }
        let glyphs = self
            .glyphs
            .iter()
            .map(|(&character, glyph)| {
                let glyph = glyph.map(|glyph| {
                    let image = &self.pages[glyph.page];
                    let (page_width, page_height) = (image.width() as f32, image.height() as f32);
                    Glyph {
                        page: glyph.page,
                        uv: [
                            glyph.x as f32 / page_width,
                            glyph.y as f32 / page_height,
                            (glyph.x + glyph.width) as f32 / page_width,
                            (glyph.y + glyph.height) as f32 / page_height,
                        ],
                        width: glyph.width as f32,
                        height: glyph.height as f32,
                        left: glyph.left,
                        top: glyph.top,
                    }
                });
                (character, glyph)
            })
            .collect();
        let page_size = self
            .pages
            .iter()
            .map(|image| image.width().max(image.height()))
            .max()
            .unwrap_or(0);
        Ok(Atlas {
            page_size,
            pages,
            glyphs,
        })
    }
}

/// What an AngelCode BMFont `.fnt` file in its text form says.
#[derive(Debug, Default, PartialEq)]
struct BMFontFile {
    /// The size the font was made at, in pixels.
    size: f32,
    line_height: f32,
    /// The distance from the top of a line to the baseline.
    base: f32,
    /// The file name of each page image, by id.
    pages: Vec<String>,
    chars: Vec<BMChar>,
    kernings: Vec<(char, char, f32)>,
}

/// Where one character is in a BMFont's pages and how it is placed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BMChar {
    character: char,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    x_offset: f32,
    y_offset: f32,
    x_advance: f32,
    page: usize,
}

impl BMFontFile {
    fn parse(text: &str) -> Result<BMFontFile, String> {
        let mut file = BMFontFile::default();
        let mut saw_common = false;
        for (number, line) in text.lines().enumerate() {
            let mut fields = fields(line);
            let Some((tag, _)) = fields.first().cloned() else {
                continue;
            };
            fields.remove(0);
            let get = |key: &str| {
                fields
                    .iter()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value.as_str())
            };
            let number_of = |key: &str| -> Result<f32, String> {
                get(key)
                    .ok_or_else(|| format!("line {} has no {key}", number + 1))?
                    .parse::<f32>()
                    .map_err(|_| format!("line {} has a bad {key}", number + 1))
            };
            match tag.as_str() {
                "info" => file.size = number_of("size").map(f32::abs).unwrap_or(0.0),
                "common" => {
                    file.line_height = number_of("lineHeight")?;
                    file.base = number_of("base")?;
                    saw_common = true;
                }
                "page" => {
                    let id = number_of("id")? as usize;
                    let name =
                        get("file").ok_or_else(|| format!("line {} has no file", number + 1))?;
                    if file.pages.len() <= id {
                        file.pages.resize(id + 1, String::new());
                    }
                    file.pages[id] = name.to_string();
                }
                "char" => {
                    let id = number_of("id")? as u32;
                    // Some tools write -1 for the character drawn for
                    // missing ones; LÖVE has no use for it either.
                    let Some(character) = char::from_u32(id) else {
                        continue;
                    };
                    file.chars.push(BMChar {
                        character,
                        x: number_of("x")? as u32,
                        y: number_of("y")? as u32,
                        width: number_of("width")? as u32,
                        height: number_of("height")? as u32,
                        x_offset: number_of("xoffset")?,
                        y_offset: number_of("yoffset")?,
                        x_advance: number_of("xadvance")?,
                        page: number_of("page").unwrap_or(0.0) as usize,
                    });
                }
                "kerning" => {
                    let first = char::from_u32(number_of("first")? as u32);
                    let second = char::from_u32(number_of("second")? as u32);
                    if let (Some(first), Some(second)) = (first, second) {
                        file.kernings.push((first, second, number_of("amount")?));
                    }
                }
                _ => {}
            }
        }
        if !saw_common {
            return Err("it is not a BMFont file in text form: it has no common line".into());
        }
        if file.pages.is_empty() || file.pages.iter().any(String::is_empty) {
            return Err("it does not name a file for every page".into());
        }
        if let Some(char) = file.chars.iter().find(|char| char.page >= file.pages.len()) {
            return Err(format!(
                "the character {:?} is on page {}, which it does not have",
                char.character, char.page
            ));
        }
        Ok(file)
    }
}

/// Splits a line of a `.fnt` file into its tag, with an empty value, and
/// its `key=value` pairs. Values may be quoted to hold spaces.
fn fields(line: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut rest = line.trim_start();
    while !rest.is_empty() {
        let key_end = rest.find(['=', ' ', '\t']).unwrap_or(rest.len());
        let key = rest[..key_end].to_string();
        rest = &rest[key_end..];
        let value = if let Some(after) = rest.strip_prefix('=') {
            if let Some(quoted) = after.strip_prefix('"') {
                let end = quoted.find('"').unwrap_or(quoted.len());
                rest = quoted.get(end + 1..).unwrap_or("");
                quoted[..end].to_string()
            } else {
                let end = after.find([' ', '\t']).unwrap_or(after.len());
                rest = &after[end..];
                after[..end].to_string()
            }
        } else {
            String::new()
        };
        fields.push((key, value));
        rest = rest.trim_start();
    }
    fields
}

/// Builds a BMFont: the measurements, and an atlas whose pages are the
/// font's page images.
fn build_bmfont(
    backend: &mut dyn Backend,
    file: &BMFontFile,
    pages: &[&[u8]],
) -> Result<(Face, Atlas), String> {
    let (face, bitmaps) = decode_bmfont(file, pages)?;
    let atlas = bitmaps.upload(backend)?;
    Ok((face, atlas))
}

/// Measures a BMFont and decodes its pages, without the GPU.
fn decode_bmfont(file: &BMFontFile, pages: &[&[u8]]) -> Result<(Face, Bitmaps), String> {
    if pages.len() < file.pages.len() {
        return Err(format!(
            "it has {} pages but {} images were given",
            file.pages.len(),
            pages.len()
        ));
    }
    let mut decoded = Vec::new();
    for (bytes, name) in pages.iter().zip(&file.pages) {
        let image = super::image::decode(bytes).map_err(|error| format!("{name}: {error}"))?;
        decoded.push(image);
    }

    let mut advances = HashMap::new();
    let mut glyphs = HashMap::new();
    for char in &file.chars {
        advances.insert(char.character, char.x_advance);
        let image = &decoded[char.page];
        if char.x + char.width > image.width() || char.y + char.height > image.height() {
            return Err(format!(
                "the character {:?} reaches past the edge of {}",
                char.character, file.pages[char.page]
            ));
        }
        let glyph = (char.width > 0 && char.height > 0).then_some(BitmapGlyph {
            page: char.page,
            x: char.x,
            y: char.y,
            width: char.width,
            height: char.height,
            left: char.x_offset,
            top: char.y_offset - file.base,
        });
        glyphs.insert(char.character, glyph);
    }
    let kerning = file
        .kernings
        .iter()
        .map(|&(first, second, amount)| ((first, second), amount))
        .collect();
    let size = if file.size > 0.0 {
        file.size
    } else {
        file.line_height
    };
    let face = Face::bitmap(
        advances,
        kerning,
        size,
        file.base,
        file.line_height - file.base,
    );
    Ok((
        face,
        Bitmaps {
            pages: decoded,
            glyphs,
        },
    ))
}

/// Says which color each glyph of a piece of colored text is, the way
/// LÖVE's colored text tables work: each part of the text is tinted by
/// the color before it.
///
/// Glyphs are matched to characters in order. Wrapping only drops spaces
/// and line breaks, which have no glyphs, so those are left out.
pub(crate) struct GlyphColors {
    characters: Vec<(char, Color)>,
    next: usize,
}

impl GlyphColors {
    pub fn new(segments: &[(Color, &str)]) -> GlyphColors {
        let characters = segments
            .iter()
            .flat_map(|&(color, text)| text.chars().map(move |character| (character, color)))
            .filter(|&(character, _)| character != ' ' && character != '\n')
            .collect();
        GlyphColors {
            characters,
            next: 0,
        }
    }

    /// The color of the next glyph, which is for `character`. Characters
    /// passed over had no glyph to draw.
    pub fn color(&mut self, character: char) -> Color {
        while let Some(&(candidate, color)) = self.characters.get(self.next) {
            self.next += 1;
            if candidate == character {
                return color;
            }
        }
        Color::WHITE
    }
}

/// The text of colored segments, joined.
pub(crate) fn joined(segments: &[(Color, &str)]) -> String {
    segments.iter().map(|(_, text)| *text).collect()
}

/// Replaces a font's glyphs with a newer version of its file, keeping its
/// size. Every handle to the font changes at once.
pub(crate) fn reload(inner: &Inner, backend: &mut dyn Backend, bytes: &[u8]) -> Result<(), String> {
    let (face, atlas) = match &inner.origin {
        Origin::Image(glyphs, extra_spacing) => {
            build_image_font(backend, bytes, glyphs, *extra_spacing)?
        }
        Origin::BMFont(dir) => {
            let text = std::str::from_utf8(bytes).map_err(|_| "it is not text".to_string())?;
            let file = BMFontFile::parse(text)?;
            let mut pages = Vec::new();
            for page in &file.pages {
                let path = dir.join(page);
                pages.push(
                    std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?,
                );
            }
            let pages: Vec<&[u8]> = pages.iter().map(Vec::as_slice).collect();
            build_bmfont(backend, &file, &pages)?
        }
        Origin::Rasterizer => return Err("a font made from a rasterizer has no file".into()),
        Origin::Vector => {
            let (size, scale, hinting) = {
                let face = inner.face.borrow();
                (face.size, face.scale, face.hinting)
            };
            let face = Face::new(bytes, size, scale, hinting)?;
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
#[derive(Clone)]
struct Face {
    source: Source,
    /// In units. All the metrics here are in units too.
    size: f32,
    /// Pixels per unit that glyphs are drawn at.
    scale: f32,
    ascent: f32,
    descent: f32,
    height: f32,
    hinting: Hinting,
}

/// Where a font's characters come from.
#[derive(Clone)]
enum Source {
    /// Outlines from a TrueType or OpenType file, drawn when first used.
    Vector(fontdue::Font),
    /// Pictures of every character, from an image font or a BMFont. Holds
    /// how far each one moves the pen, and the kerning between pairs.
    Bitmap {
        advances: HashMap<char, f32>,
        kerning: HashMap<(char, char), f32>,
    },
}

impl Face {
    fn new(bytes: &[u8], size: f32, scale: f32, hinting: Hinting) -> Result<Self, String> {
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
            hinting,
        })
    }

    /// A face for a font drawn from pictures, one unit to the pixel.
    fn bitmap(
        advances: HashMap<char, f32>,
        kerning: HashMap<(char, char), f32>,
        size: f32,
        ascent: f32,
        descent: f32,
    ) -> Face {
        Face {
            source: Source::Bitmap { advances, kerning },
            size,
            scale: 1.0,
            ascent,
            descent,
            height: ascent + descent,
            hinting: Hinting::Normal,
        }
    }

    /// Whether the font has a picture or outline for `character`.
    fn has_glyph(&self, character: char) -> bool {
        match &self.source {
            Source::Vector(font) => font.lookup_glyph_index(character) != 0,
            Source::Bitmap { advances, .. } => advances.contains_key(&character),
        }
    }

    fn advance(&self, character: char) -> f32 {
        match &self.source {
            _ if character.is_control() => 0.0,
            Source::Vector(font) => font.metrics(character, self.size).advance_width,
            // A character the picture lacks takes no room, as it draws
            // nothing.
            Source::Bitmap { advances, .. } => advances.get(&character).copied().unwrap_or(0.0),
        }
    }

    fn kerning(&self, left: char, right: char) -> f32 {
        match &self.source {
            Source::Vector(font) => font.horizontal_kern(left, right, self.size).unwrap_or(0.0),
            Source::Bitmap { kerning, .. } => kerning.get(&(left, right)).copied().unwrap_or(0.0),
        }
    }

    #[cfg(test)]
    fn line_width(&self, line: &str) -> f32 {
        Faces { faces: vec![self] }.line_width(line)
    }

    #[cfg(test)]
    fn wrap(&self, text: &str, limit: f32) -> Vec<String> {
        Faces { faces: vec![self] }
            .wrap(text, limit)
            .into_iter()
            .map(|line| line.text)
            .collect()
    }
}

/// A font's face followed by its fallbacks' faces, measuring each
/// character with the first that has it.
struct Faces<'a> {
    faces: Vec<&'a Face>,
}

/// One line of wrapped text.
#[derive(Debug, PartialEq)]
struct Line {
    text: String,
    /// Whether the line ends its paragraph, which justified text leaves
    /// unstretched.
    last: bool,
}

/// Where one character goes, as worked out by [`Faces::place`].
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    character: char,
    /// Which of the faces draws it.
    face: usize,
    /// The pen position before the character, in units from the left.
    pen: f32,
    /// The line's baseline, in units from the top.
    baseline: f32,
}

impl Faces<'_> {
    /// The first face that has `character`.
    fn find(&self, character: char) -> Option<usize> {
        self.faces.iter().position(|face| face.has_glyph(character))
    }

    /// The face that draws `character`: the first that has it, or the
    /// font's own if none does.
    fn pick(&self, character: char) -> usize {
        if character.is_control() {
            return 0;
        }
        self.find(character).unwrap_or(0)
    }

    fn advance(&self, character: char) -> f32 {
        self.faces[self.pick(character)].advance(character)
    }

    fn kerning(&self, left: char, right: char) -> f32 {
        let face = self.pick(left);
        if face == self.pick(right) {
            self.faces[face].kerning(left, right)
        } else {
            0.0
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
    fn wrap(&self, text: &str, limit: f32) -> Vec<Line> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut broken: Vec<String> = Vec::new();
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
                    broken.push(std::mem::take(&mut line));
                }
                // The word starts a new line; split it if it is too wide
                // even on its own.
                for character in word.chars() {
                    let mut longer = line.clone();
                    longer.push(character);
                    if !line.is_empty() && self.line_width(&longer) > limit {
                        broken.push(std::mem::replace(&mut line, character.to_string()));
                    } else {
                        line = longer;
                    }
                }
            }
            broken.push(line);
            let count = broken.len();
            lines.extend(broken.into_iter().enumerate().map(|(index, text)| Line {
                text,
                last: index + 1 == count,
            }));
        }
        lines
    }

    /// Works out where every character of `text` goes: wrapped and
    /// aligned within `wrap`'s width if given, with lines `line_height`
    /// times the first face's height apart.
    fn place(&self, text: &str, wrap: Option<(f32, Align)>, line_height: f32) -> Vec<Placed> {
        let primary = self.faces[0];
        let lines = match wrap {
            Some((limit, _)) => self.wrap(text, limit),
            None => text
                .split('\n')
                .map(|line| Line {
                    text: line.to_string(),
                    last: true,
                })
                .collect(),
        };
        // Snapping to whole pixels keeps unscaled text sharp.
        let snap = |value: f32| (value * primary.scale).round() / primary.scale;
        let line_advance = snap(primary.height * line_height);

        let mut placed = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let width = self.line_width(&line.text);
            let (start_x, space_extra) = match wrap {
                Some((limit, Align::Center)) => (snap((limit - width) / 2.0), 0.0),
                Some((limit, Align::Right)) => (snap(limit - width), 0.0),
                Some((limit, Align::Justify)) if !line.last => {
                    let spaces = line.text.chars().filter(|&c| c == ' ').count();
                    let extra = if spaces > 0 {
                        (limit - width).max(0.0) / spaces as f32
                    } else {
                        0.0
                    };
                    (0.0, extra)
                }
                _ => (0.0, 0.0),
            };
            let baseline = index as f32 * line_advance + snap(primary.ascent);

            let mut pen = start_x;
            let mut previous = None;
            for character in line.text.chars() {
                if let Some(previous) = previous {
                    pen += self.kerning(previous, character);
                }
                placed.push(Placed {
                    character,
                    face: self.pick(character),
                    pen,
                    baseline,
                });
                pen += self.advance(character);
                if character == ' ' {
                    pen += space_extra;
                }
                previous = Some(character);
            }
        }
        placed
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

impl Page {
    /// A page that is all one picture, with no room for more glyphs.
    fn full(texture: TextureId, width: u32, height: u32) -> Page {
        Page {
            texture,
            cursor_x: width,
            cursor_y: height,
            row_height: 0,
        }
    }
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
        // Monochrome hinting draws every pixel fully on or off.
        let mono = face.hinting == Hinting::Mono;
        let rgba: Vec<u8> = coverage
            .iter()
            .map(|&alpha| match mono {
                true if alpha >= 128 => 255,
                true => 0,
                false => alpha,
            })
            .flat_map(|alpha| [255, 255, 255, alpha])
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
        let texture = new_page(backend, self.page_size, self.page_size, &blank).ok()?;
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

    #[test]
    fn glyph_colors_follow_the_segments_and_skip_spaces() {
        let red = Color::rgb(1.0, 0.0, 0.0);
        let blue = Color::rgb(0.0, 0.0, 1.0);
        let mut colors = GlyphColors::new(&[(red, "ab "), (blue, "c\nd")]);
        // Wrapping drops spaces and line breaks; the rest arrive in order.
        assert_eq!(colors.color('a'), red);
        assert_eq!(colors.color('b'), red);
        assert_eq!(colors.color('c'), blue);
        assert_eq!(colors.color('d'), blue);
        assert_eq!(colors.color('e'), Color::WHITE);
    }

    #[test]
    fn a_character_with_no_glyph_is_passed_over() {
        let red = Color::rgb(1.0, 0.0, 0.0);
        let blue = Color::rgb(0.0, 0.0, 1.0);
        // The tab draws nothing, so the next glyph is the x.
        let mut colors = GlyphColors::new(&[(red, "\t"), (blue, "x")]);
        assert_eq!(colors.color('x'), blue);
    }

    fn face() -> Face {
        Face::new(DEFAULT_FONT, 16.0, 1.0, Hinting::Normal).expect("the built-in font is valid")
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
        let face = bitmap_face(&[('A', 4.0), ('B', 2.0)]);
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
        assert!(Face::new(DEFAULT_FONT, 0.0, 1.0, Hinting::Normal).is_err());
        assert!(Face::new(DEFAULT_FONT, f32::NAN, 1.0, Hinting::Normal).is_err());
        assert!(Face::new(b"not a font", 16.0, 1.0, Hinting::Normal).is_err());
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

    /// A bitmap face six units tall holding `advances`.
    fn bitmap_face(advances: &[(char, f32)]) -> Face {
        Face::bitmap(
            advances.iter().copied().collect(),
            HashMap::new(),
            6.0,
            5.0,
            1.0,
        )
    }

    #[test]
    fn fallbacks_measure_the_characters_the_font_lacks() {
        let primary = bitmap_face(&[('A', 4.0)]);
        let vera = face();
        let faces = Faces {
            faces: vec![&primary, &vera],
        };
        assert_eq!(faces.pick('A'), 0);
        assert_eq!(faces.pick('z'), 1);
        assert_eq!(faces.find('\u{10FFFF}'), None);
        assert_eq!(faces.line_width("Az"), 4.0 + vera.advance('z'));
        // Without the fallback the z takes no room.
        assert_eq!(primary.line_width("Az"), 4.0);
    }

    #[test]
    fn characters_from_different_faces_are_not_kerned() {
        let mut kerning = HashMap::new();
        kerning.insert(('A', 'V'), -1.0);
        let primary = Face::bitmap(
            HashMap::from([('A', 4.0), ('V', 4.0)]),
            kerning,
            6.0,
            5.0,
            1.0,
        );
        let vera = face();
        let faces = Faces {
            faces: vec![&primary, &vera],
        };
        assert_eq!(faces.kerning('A', 'V'), -1.0);
        assert_eq!(faces.line_width("AV"), 7.0);
        assert_eq!(faces.kerning('A', 'z'), 0.0);
    }

    #[test]
    fn placing_uses_the_face_that_has_each_character() {
        let primary = bitmap_face(&[('A', 4.0)]);
        let vera = face();
        let faces = Faces {
            faces: vec![&primary, &vera],
        };
        let placed = faces.place("Az", None, 1.0);
        assert_eq!(placed.len(), 2);
        assert_eq!((placed[0].face, placed[0].pen), (0, 0.0));
        assert_eq!((placed[1].face, placed[1].pen), (1, 4.0));
        // The baseline is the primary face's.
        assert_eq!(placed[1].baseline, 5.0);
    }

    #[test]
    fn justified_lines_stretch_their_spaces_except_the_last() {
        let face = bitmap_face(&[('a', 2.0), (' ', 1.0)]);
        let faces = Faces { faces: vec![&face] };
        // "aa aa" is 9 wide; "aa aa aa" would be 14, past the limit.
        let placed = faces.place("aa aa aa\naa a", Some((12.0, Align::Justify)), 1.0);
        let pens: Vec<(char, f32, f32)> = placed
            .iter()
            .map(|placed| (placed.character, placed.pen, placed.baseline))
            .collect();
        // First line: one space gets the 3 spare units, so the second
        // word ends at the limit.
        assert_eq!(pens[3], ('a', 8.0, 5.0));
        assert_eq!(pens[4].1 + 2.0, 12.0);
        // The paragraph's last line, and the last paragraph, stay as
        // they are.
        let last_line: Vec<_> = pens.iter().filter(|pen| pen.2 == 11.0).collect();
        assert_eq!(last_line[1].1, 2.0);
        let final_line: Vec<_> = pens.iter().filter(|pen| pen.2 == 17.0).collect();
        assert_eq!(final_line[3].1, 5.0);
    }

    #[test]
    fn wrapping_marks_the_end_of_each_paragraph() {
        let face = bitmap_face(&[('a', 2.0), (' ', 1.0)]);
        let faces = Faces { faces: vec![&face] };
        let lines = faces.wrap("aa aa aa\naa", 10.0);
        let found: Vec<(&str, bool)> = lines
            .iter()
            .map(|line| (line.text.as_str(), line.last))
            .collect();
        assert_eq!(found, [("aa aa", false), ("aa", true), ("aa", true)]);
    }

    #[test]
    fn centered_and_right_aligned_lines_start_further_in() {
        let face = bitmap_face(&[('a', 2.0)]);
        let faces = Faces { faces: vec![&face] };
        assert_eq!(
            faces.place("aa", Some((10.0, Align::Center)), 1.0)[0].pen,
            3.0
        );
        assert_eq!(
            faces.place("aa", Some((10.0, Align::Right)), 1.0)[0].pen,
            6.0
        );
        assert_eq!(
            faces.place("aa", Some((10.0, Align::Justify)), 1.0)[0].pen,
            0.0
        );
    }

    #[test]
    fn the_built_in_font_knows_which_glyphs_it_has() {
        let face = face();
        assert!(face.has_glyph('a') && face.has_glyph(' '));
        assert!(!face.has_glyph('\u{4E00}'));
    }

    #[test]
    fn fonts_report_glyphs_kerning_and_baseline() {
        use crate::Filter;
        let font = Font::fallback_at(16.0, Filter::Linear, 1.0);
        assert!(font.has_glyphs("Hello, world!"));
        assert!(!font.has_glyphs("Hello \u{4E00}"));
        assert!(font.kerning('A', 'V') <= 0.0);
        assert_eq!(font.baseline(), font.ascent().round());
    }

    #[test]
    fn fallbacks_skip_the_font_itself_and_repeats() {
        use crate::Filter;
        let font = Font::fallback_at(16.0, Filter::Linear, 1.0);
        let other = Font::fallback_at(20.0, Filter::Linear, 1.0);
        let key = font.layout_key();
        font.set_fallbacks(&[&font, &other, &other]);
        let fallbacks = font.fallbacks();
        assert_eq!(fallbacks.len(), 1);
        assert!(fallbacks[0].is(&other));
        // Laid out text knows to lay itself out again.
        assert_ne!(font.layout_key(), key);
        font.set_fallbacks(&[]);
        assert!(font.fallbacks().is_empty());
    }

    const FNT: &str = r#"info face="Test Font" size=-12 bold=0 italic=0 charset="" unicode=1 padding=0,0,0,0 spacing=1,1
common lineHeight=14 base=11 scaleW=64 scaleH=32 pages=2 packed=0
page id=0 file="test_0.png"
page id=1 file="test 1.png"
chars count=3
char id=32   x=0     y=0     width=0     height=0     xoffset=0     yoffset=11    xadvance=4     page=0  chnl=15
char id=65   x=1     y=2     width=7     height=9     xoffset=0     yoffset=2     xadvance=8     page=0  chnl=15
char id=86   x=10    y=0     width=7     height=9     xoffset=-1    yoffset=2     xadvance=7     page=1  chnl=15
kernings count=1
kerning first=65  second=86  amount=-1
"#;

    #[test]
    fn a_bmfont_file_is_parsed() {
        let file = BMFontFile::parse(FNT).unwrap();
        assert_eq!(file.size, 12.0);
        assert_eq!((file.line_height, file.base), (14.0, 11.0));
        assert_eq!(file.pages, ["test_0.png", "test 1.png"]);
        assert_eq!(file.chars.len(), 3);
        assert_eq!(
            file.chars[2],
            BMChar {
                character: 'V',
                x: 10,
                y: 0,
                width: 7,
                height: 9,
                x_offset: -1.0,
                y_offset: 2.0,
                x_advance: 7.0,
                page: 1,
            }
        );
        assert_eq!(file.kernings, [('A', 'V', -1.0)]);
    }

    #[test]
    fn bad_bmfont_files_are_errors() {
        assert!(BMFontFile::parse("not a font").is_err());
        assert!(BMFontFile::parse("common lineHeight=14 base=11\n").is_err());
        let wrong_page = "common lineHeight=14 base=11\npage id=0 file=\"a.png\"\n\
                          char id=65 x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=3\n";
        assert!(
            BMFontFile::parse(wrong_page)
                .unwrap_err()
                .contains("page 3")
        );
    }

    #[test]
    fn fnt_fields_split_with_quotes() {
        let found = fields(r#"page id=0 file="my font.png"  x=-3"#);
        let found: Vec<(&str, &str)> = found
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("page", ""),
                ("id", "0"),
                ("file", "my font.png"),
                ("x", "-3")
            ]
        );
    }
}
