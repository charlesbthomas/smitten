use std::cell::Cell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use smitten_core::Filter;
use smitten_core::backend::TextureId;

use crate::context;

/// A picture that can be drawn, the equivalent of LÖVE's `Image`.
///
/// Cloning is cheap and gives another handle to the same picture. Its GPU
/// memory is freed when the last handle is dropped.
#[derive(Clone)]
pub struct Image {
    inner: Rc<Inner>,
}

struct Inner {
    texture: TextureId,
    width: u32,
    height: u32,
    filter: Cell<Filter>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // If the game has already shut down, the backend freed everything.
        context::try_with(|ctx| ctx.backend.destroy_texture(self.texture));
    }
}

impl Image {
    /// Creates an image from raw pixels: `width * height` of them, four
    /// bytes each (red, green, blue, alpha), in rows from the top.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Image, ImageError> {
        context::with(|ctx| {
            let texture = ctx
                .backend
                .create_texture(width, height, rgba)
                .map_err(ImageError::Upload)?;
            Ok(Image {
                inner: Rc::new(Inner {
                    texture,
                    width,
                    height,
                    filter: Cell::new(ctx.gfx.default_filter),
                }),
            })
        })
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.inner.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.inner.height
    }

    /// Size in pixels as `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.inner.width, self.inner.height)
    }

    /// How the image is sampled when scaled or rotated.
    pub fn filter(&self) -> Filter {
        self.inner.filter.get()
    }

    /// Sets the filter for every handle to this image.
    pub fn set_filter(&self, filter: Filter) {
        self.inner.filter.set(filter);
    }

    pub(crate) fn texture(&self) -> TextureId {
        self.inner.texture
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.inner.width)
            .field("height", &self.inner.height)
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

pub(crate) fn load(path: &Path) -> Result<Image, ImageError> {
    let bytes = std::fs::read(path).map_err(|source| ImageError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let decoded = ::image::load_from_memory(&bytes)
        .map_err(|error| ImageError::Decode {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?
        .into_rgba8();
    Image::from_rgba(decoded.width(), decoded.height(), decoded.as_raw())
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
}
