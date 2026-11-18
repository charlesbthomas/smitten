//! Textures with more than one picture: array, cube and volume images.

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
        let rgba: Vec<_> = layers.iter().map(ImageData::rgba8).collect();
        let pixels: Vec<&[u8]> = rgba.iter().map(|layer| &**layer).collect();
        context::with(|ctx| {
            let texture = ctx
                .backend
                .create_layered_texture(
                    kind,
                    super::normal_format(&*ctx.backend),
                    width,
                    height,
                    &pixels,
                )
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
/// Draw one layer with [`graphics::draw`](super::draw) and
/// [`ArrayImage::layer`], or
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

    /// An array image for a texture that was never made, for tests that
    /// need no GPU.
    #[cfg(test)]
    pub(crate) fn for_tests(texture: TextureId, width: u32, height: u32, layers: u32) -> Self {
        ArrayImage {
            inner: Rc::new(Inner {
                texture,
                width,
                height,
                layers,
                filter: Cell::new(Filter::default()),
                _counted: Counted::new(Kind::Image),
            }),
        }
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

    /// Makes a cube image from one picture holding all six faces, laid out
    /// as LÖVE accepts them and told apart by the picture's shape:
    ///
    /// - a row of six faces, 6 by 1, in the order +x, -x, +y, -y, +z, -z;
    /// - a column of six, 1 by 6, in the same order;
    /// - a cross lying down, 4 by 3: +y on top, then -x, +z, +x, -z across
    ///   the middle, and -y below;
    /// - a cross standing up, 3 by 4: as above, with -z at the bottom,
    ///   upside down.
    pub fn from_layout(picture: &ImageData) -> Result<CubeImage, ImageError> {
        CubeImage::from_image_data(&split_cube_layout(picture)?)
    }

    /// Loads one image file holding all six faces, laid out as
    /// [`CubeImage::from_layout`] describes.
    pub fn from_layout_file(path: impl AsRef<Path>) -> Result<CubeImage, ImageError> {
        let (width, height, rgba) = super::image::load_rgba(path.as_ref())?;
        CubeImage::from_layout(&ImageData::from_rgba(width, height, rgba))
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

/// A block of pixels with depth as well as width and height, the
/// equivalent of LÖVE's volume `Image` (`TextureType` `volume`). It is
/// built from slices of the same size, stacked front to back, and
/// sampled in three dimensions with a coordinate from 0 to 1 on each
/// axis, so the GPU blends between neighbouring slices as well as
/// neighbouring pixels. It suits color grading tables, fog and noise.
///
/// As in LÖVE, a volume image cannot be drawn directly. Send it to a
/// [`Shader`](super::Shader) that declares a `texture_3d<f32>`:
///
/// ```wgsl
/// var grade: texture_3d<f32>;
///
/// fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
///     let original = textureSample(image, image_sampler, uv) * color;
///     return vec4<f32>(textureSample(grade, grade_sampler, original.rgb).rgb, original.a);
/// }
/// ```
///
/// Cloning is cheap and gives another handle to the same texture.
#[derive(Clone)]
pub struct VolumeImage {
    inner: Rc<Inner>,
}

impl VolumeImage {
    /// Makes a volume image from its slices, the first at the front (a
    /// depth coordinate of 0). They must all be the same size.
    pub fn from_image_data(slices: &[ImageData]) -> Result<VolumeImage, ImageError> {
        Ok(VolumeImage {
            inner: Rc::new(Inner::create(TextureKind::Volume, slices)?),
        })
    }

    /// Loads an image file for each slice, front to back. They must all
    /// be the same size.
    pub fn from_files(paths: &[impl AsRef<Path>]) -> Result<VolumeImage, ImageError> {
        VolumeImage::from_image_data(&load_all(paths)?)
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.inner.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.inner.height
    }

    /// How many slices deep it is, the equivalent of `Texture:getDepth`.
    pub fn depth(&self) -> u32 {
        self.inner.layers
    }

    /// Size in pixels as `(width, height, depth)`.
    pub fn dimensions(&self) -> (u32, u32, u32) {
        (self.inner.width, self.inner.height, self.inner.layers)
    }

    /// How the volume is sampled between pixels and slices.
    pub fn filter(&self) -> Filter {
        self.inner.filter.get()
    }

    /// Sets the filter for every handle to this volume image.
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

impl fmt::Debug for VolumeImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VolumeImage")
            .field("width", &self.inner.width)
            .field("height", &self.inner.height)
            .field("depth", &self.inner.layers)
            .finish()
    }
}

/// Cuts a picture of all six faces of a cube into the faces, in the order
/// +x, -x, +y, -y, +z, -z. See [`CubeImage::from_layout`].
fn split_cube_layout(picture: &ImageData) -> Result<[ImageData; 6], ImageError> {
    let (width, height) = picture.dimensions();
    // Where each face sits, in face-sized cells, and whether it is upside
    // down.
    let (size, cells): (u32, [(u32, u32, bool); 6]) = if width == height * 6 {
        (height, std::array::from_fn(|i| (i as u32, 0, false)))
    } else if height == width * 6 {
        (width, std::array::from_fn(|i| (0, i as u32, false)))
    } else if width * 3 == height * 4 && width % 4 == 0 {
        (
            width / 4,
            [
                (2, 1, false),
                (0, 1, false),
                (1, 0, false),
                (1, 2, false),
                (1, 1, false),
                (3, 1, false),
            ],
        )
    } else if width * 4 == height * 3 && width % 3 == 0 {
        (
            width / 3,
            [
                (2, 1, false),
                (0, 1, false),
                (1, 0, false),
                (1, 2, false),
                (1, 1, false),
                (1, 3, true),
            ],
        )
    } else {
        return Err(ImageError::Invalid(format!(
            "a {width} by {height} picture is not a cube layout: \
             it must be 6 by 1, 1 by 6, 4 by 3 or 3 by 4 faces"
        )));
    };
    if size == 0 {
        return Err(ImageError::Invalid("the picture has no pixels".to_string()));
    }
    let pixels = picture.rgba8();
    Ok(cells.map(|(column, row, upside_down)| {
        let mut face = Vec::with_capacity(size as usize * size as usize * 4);
        for y in 0..size {
            for x in 0..size {
                let (fx, fy) = if upside_down {
                    (size - 1 - x, size - 1 - y)
                } else {
                    (x, y)
                };
                let px = column * size + fx;
                let py = row * size + fy;
                let start = (py as usize * width as usize + px as usize) * 4;
                face.extend_from_slice(&pixels[start..start + 4]);
            }
        }
        ImageData::from_rgba(size, size, face)
    }))
}

impl fmt::Debug for CubeImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CubeImage")
            .field("size", &self.inner.width)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture `columns` by `rows` faces of `size` pixels, where every
    /// pixel's red says which cell it is in and green and blue say where
    /// in the cell.
    fn layout(columns: u32, rows: u32, size: u32) -> ImageData {
        let (width, height) = (columns * size, rows * size);
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let cell = (y / size) * columns + x / size;
                rgba.extend_from_slice(&[cell as u8, (x % size) as u8, (y % size) as u8, 255]);
            }
        }
        ImageData::from_rgba(width, height, rgba)
    }

    fn cell_of(face: &ImageData) -> u8 {
        face.as_bytes()[0]
    }

    #[test]
    fn strips_hold_the_faces_in_order() {
        let faces = split_cube_layout(&layout(6, 1, 2)).unwrap();
        assert_eq!(faces.each_ref().map(cell_of), [0, 1, 2, 3, 4, 5]);
        let faces = split_cube_layout(&layout(1, 6, 2)).unwrap();
        assert_eq!(faces.each_ref().map(cell_of), [0, 1, 2, 3, 4, 5]);
        assert_eq!(faces[0].dimensions(), (2, 2));
    }

    #[test]
    fn a_lying_cross_is_read_from_its_arms() {
        // Cells are numbered across rows of four.
        let faces = split_cube_layout(&layout(4, 3, 2)).unwrap();
        assert_eq!(faces.each_ref().map(cell_of), [6, 4, 1, 9, 5, 7]);
    }

    #[test]
    fn a_standing_cross_turns_its_last_face_the_right_way_up() {
        // Cells are numbered across rows of three.
        let faces = split_cube_layout(&layout(3, 4, 2)).unwrap();
        assert_eq!(faces.each_ref().map(cell_of)[..5], [5, 3, 1, 7, 4]);
        // -z is cell 10, read from its bottom-right corner first.
        let back = faces[5].as_bytes();
        assert_eq!(&back[..4], &[10, 1, 1, 255]);
        assert_eq!(&back[12..16], &[10, 0, 0, 255]);
    }

    #[test]
    fn other_shapes_are_refused() {
        assert!(split_cube_layout(&layout(2, 2, 2)).is_err());
    }
}
