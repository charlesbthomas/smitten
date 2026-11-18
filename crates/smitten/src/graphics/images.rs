//! Loading and drawing images, and the Drawable things.

use super::*;

impl Image {
    /// Loads an image file: PNG, JPEG or TGA, or DDS or KTX for compressed
    /// pixels.
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes, which
    /// normally means inside the game's `assets` folder.
    pub fn load(path: impl AsRef<Path>) -> Result<Image, ImageError> {
        image::load(path.as_ref(), ImageSettings::default())
    }
}

impl Image {
    /// Loads an image file with options such as mipmaps, the equivalent of
    /// `love.graphics.newImage(path, settings)`.
    #[doc(alias = "love.graphics.newImage")]
    pub fn load_with(path: impl AsRef<Path>, settings: ImageSettings) -> Result<Image, ImageError> {
        image::load(path.as_ref(), settings)
    }
}

impl ImageData {
    /// Loads an image file into memory, where its pixels can be read and
    /// changed. See [`ImageData`].
    pub fn load(path: impl AsRef<Path>) -> Result<ImageData, ImageError> {
        let (width, height, rgba) = image::load_rgba(path.as_ref())?;
        Ok(ImageData::from_rgba(width, height, rgba))
    }
}

/// Draws `item` placed by `params`, the equivalent of
/// `love.graphics.draw`. A position is enough for `params`, and
/// [`DrawParams`] adds rotation, scale, an origin, shear and a
/// [`Transform`].
///
/// `item` is any [`Drawable`]: an [`Image`] or [`Canvas`], part of one
/// from [`Image::region`], a layer from [`ArrayImage::layer`], a
/// [`SpriteBatch`], [`Mesh`], [`ParticleSystem`], [`Text`] or video.
///
/// ```no_run
/// use smitten::graphics::{self, DrawParams, Image, Quad};
///
/// # fn demo(sheet: &Image) {
/// graphics::draw(sheet, (10.0, 10.0));
/// let frame = Quad::new((0.0, 0.0), (16.0, 16.0), (64.0, 64.0));
/// graphics::draw(&sheet.region(&frame), DrawParams::at((40.0, 10.0)).scale(4.0));
/// # }
/// ```
#[doc(alias = "love.graphics.draw")]
#[doc(alias = "drawLayer")]
pub fn draw(item: &impl Drawable, params: impl Into<DrawParams>) {
    item.draw_transformed(&params.into().to_transform());
}

impl draw::sealed::Sealed for Image {
    fn draw_transformed(&self, local: &Transform) {
        let quad = whole_image(self);
        context::with(|ctx| ctx.gfx.push_image(self, &quad, local));
    }
}
impl Drawable for Image {}

impl draw::sealed::Sealed for Canvas {
    fn draw_transformed(&self, local: &Transform) {
        let image: &Image = self;
        draw::sealed::Sealed::draw_transformed(image, local);
    }
}
impl Drawable for Canvas {}

impl draw::sealed::Sealed for ImageRegion<'_> {
    fn draw_transformed(&self, local: &Transform) {
        context::with(|ctx| ctx.gfx.push_image(self.image, self.quad, local));
    }
}
impl Drawable for ImageRegion<'_> {}

#[cfg(feature = "video")]
impl draw::sealed::Sealed for Video {
    /// Drawing moves the picture on to the video's position.
    fn draw_transformed(&self, local: &Transform) {
        draw::sealed::Sealed::draw_transformed(self.image(), local);
    }
}
#[cfg(feature = "video")]
impl Drawable for Video {}

impl draw::sealed::Sealed for SpriteBatch {
    fn draw_transformed(&self, local: &Transform) {
        context::with(|ctx| ctx.gfx.push_sprite_batch(&mut *ctx.backend, self, local));
    }
}
impl Drawable for SpriteBatch {}

impl draw::sealed::Sealed for Mesh {
    fn draw_transformed(&self, local: &Transform) {
        context::with(|ctx| ctx.gfx.push_mesh(&mut *ctx.backend, self, local, 1));
    }
}
impl Drawable for Mesh {}

impl draw::sealed::Sealed for ParticleSystem {
    fn draw_transformed(&self, local: &Transform) {
        context::with(|ctx| ctx.gfx.push_particles(self, local));
    }
}
impl Drawable for ParticleSystem {}

impl draw::sealed::Sealed for Text {
    fn draw_transformed(&self, local: &Transform) {
        context::with(|ctx| ctx.gfx.push_text_object(ctx.backend.as_mut(), self, local));
    }
}
impl Drawable for Text {}

/// Draws `instances` copies of a mesh at once, placed by `params`, as
/// LÖVE's `drawInstanced`. Each copy is drawn in the same place, so give
/// them something to tell them apart: attributes attached with
/// [`VertexAttributeStep::PerInstance`] through
/// [`Mesh::attach_attribute_with`], which advance once per copy, or
/// `instance_index` in a [`Shader`]'s `position`, which counts the copies
/// from 0.
///
/// Panics if a per-instance attribute's mesh has fewer vertices than
/// `instances`. A mesh drawn as [`MeshDrawMode::Points`] is drawn once.
pub fn draw_instanced(mesh: &Mesh, instances: u32, params: impl Into<DrawParams>) {
    let local = params.into().to_transform();
    context::with(|ctx| {
        ctx.gfx
            .push_mesh(&mut *ctx.backend, mesh, &local, instances)
    });
}
