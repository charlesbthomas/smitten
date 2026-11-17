//! Textures with more than one picture: array images and cube images.

use std::cell::Cell;
use std::fmt;
use std::path::Path;
use std::rc::Rc;

use smitten_core::Filter;
use smitten_core::backend::{TextureId, TextureKind};

use super::stats::{Counted, Kind};
use super::{ImageData, ImageError};
use crate::context;

/// The GPU side shared by [`ArrayImage`] and [`CubeImage`].
struct Inner {
    texture: TextureId,
    width: u32,
    height: u32,
    layers: u32,
    filter: Cell<Filter>,
    _counted: Counted,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // If the game has already shut down, the backend freed everything.
        context::try_with(|ctx| ctx.backend.destroy_texture(self.texture));
    }
}

impl Inner {
    fn create(kind: TextureKind, layers: &[ImageData]) -> Result<Inner, ImageError> {
        let Some(first) = layers.first() else {
            return Err(ImageError::Invalid(
                "there must be at least one layer".to_string(),
            ));
        };
        let (width, height) = first.dimensions();
        if let Some((index, layer)) = layers
            .iter()
            .enumerate()
            .find(|(_, layer)| layer.dimensions() != (width, height))
        {
            let (other_width, other_height) = layer.dimensions();
            return Err(ImageError::Invalid(format!(
                "every layer must be the same size: layer 0 is {width} by {height}, \
                 but layer {index} is {other_width} by {other_height}"
            )));
        }
        let pixels: Vec<&[u8]> = layers.iter().map(ImageData::as_bytes).collect();
        context::with(|ctx| {
            let texture = ctx
                .backend
                .create_layered_texture(kind, width, height, &pixels)
                .map_err(ImageError::Upload)?;
            Ok(Inner {
                texture,
                width,
                height,
                layers: layers.len() as u32,
                filter: Cell::new(ctx.gfx.default_filter),
                _counted: Counted::new(Kind::Image),
            })
        })
    }
}

fn load_all(paths: &[impl AsRef<Path>]) -> Result<Vec<ImageData>, ImageError> {
    paths
        .iter()
        .map(|path| {
            let (width, height, rgba) = super::image::load_rgba(path.as_ref())?;
            Ok(ImageData::from_rgba(width, height, rgba))
        })
        .collect()
}

/// Several pictures of the same size in one texture, the equivalent of
/// LÖVE's `ArrayImage`.
///
/// Draw one layer with [`graphics::draw_layer`](super::draw_layer), or
/// send the whole array to a [`Shader`](super::Shader) that declares a
/// `texture_2d_array<f32>`, where it can pick a layer per pixel, such as
/// for tiles that blend into each other.
///
/// Cloning is cheap and gives another handle to the same texture.
#[derive(Clone)]
pub struct ArrayImage {
    inner: Rc<Inner>,
}

impl ArrayImage {
    /// Makes an array image with one layer for each of `layers`, in order.
    /// They must all be the same size.
    pub fn from_image_data(layers: &[ImageData]) -> Result<ArrayImage, ImageError> {
        Ok(ArrayImage {
            inner: Rc::new(Inner::create(TextureKind::Array, layers)?),
        })
    }

    /// Loads an image file for each layer. They must all be the same size.
    pub fn from_files(paths: &[impl AsRef<Path>]) -> Result<ArrayImage, ImageError> {
        ArrayImage::from_image_data(&load_all(paths)?)
    }

    /// Width of each layer in pixels.
    pub fn width(&self) -> u32 {
        self.inner.width
    }

    /// Height of each layer in pixels.
    pub fn height(&self) -> u32 {
        self.inner.height
    }

    /// Size of each layer in pixels as `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.inner.width, self.inner.height)
    }

    /// How many layers there are.
    pub fn layer_count(&self) -> u32 {
        self.inner.layers
    }

    /// How the layers are sampled when scaled or rotated.
    pub fn filter(&self) -> Filter {
        self.inner.filter.get()
    }

    /// Sets the filter for every handle to this array image.
    pub fn set_filter(&self, filter: Filter) {
        self.inner.filter.set(filter);
    }

    pub(crate) fn texture(&self) -> TextureId {
        self.inner.texture
    }

    pub(crate) fn keep_alive(&self) -> Rc<dyn std::any::Any> {
        self.inner.clone()
    }
}

impl fmt::Debug for ArrayImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArrayImage")
            .field("width", &self.inner.width)
            .field("height", &self.inner.height)
            .field("layers", &self.inner.layers)
            .finish()
    }
}

/// Six square pictures that make the inside of a cube, the equivalent of
/// LÖVE's `CubeImage`, for a sky or for reflections.
///
/// A cube image cannot be drawn directly. Send it to a
/// [`Shader`](super::Shader) that declares a `texture_cube<f32>`, which
/// samples it by direction.
///
/// Cloning is cheap and gives another handle to the same texture.
#[derive(Clone)]
pub struct CubeImage {
    inner: Rc<Inner>,
}

impl CubeImage {
    /// Makes a cube image from its six faces, in LÖVE's order: +x, -x, +y,
    /// -y, +z, -z. They must be square and all the same size.
    pub fn from_image_data(faces: &[ImageData; 6]) -> Result<CubeImage, ImageError> {
        let (width, height) = faces[0].dimensions();
        if width != height {
            return Err(ImageError::Invalid(format!(
                "the faces of a cube image must be square, not {width} by {height}"
            )));
        }
        Ok(CubeImage {
            inner: Rc::new(Inner::create(TextureKind::Cube, faces)?),
        })
    }

    /// Loads the six faces from image files, in the order +x, -x, +y, -y,
    /// +z, -z.
    pub fn from_files(paths: &[impl AsRef<Path>; 6]) -> Result<CubeImage, ImageError> {
        let faces: [ImageData; 6] = load_all(paths)?
            .try_into()
            .expect("six paths load six faces");
        CubeImage::from_image_data(&faces)
    }

    /// The width and height of each face in pixels.
    pub fn size(&self) -> u32 {
        self.inner.width
    }

    /// How the faces are sampled.
    pub fn filter(&self) -> Filter {
        self.inner.filter.get()
    }

    /// Sets the filter for every handle to this cube image.
    pub fn set_filter(&self, filter: Filter) {
        self.inner.filter.set(filter);
    }

    pub(crate) fn texture(&self) -> TextureId {
        self.inner.texture
    }

    pub(crate) fn keep_alive(&self) -> Rc<dyn std::any::Any> {
        self.inner.clone()
    }
}

impl fmt::Debug for CubeImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CubeImage")
            .field("size", &self.inner.width)
            .finish()
    }
}
