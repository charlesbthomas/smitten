use std::cell::RefCell;
use std::ops::Range;

use smitten_core::Filter;
use smitten_core::backend::{TextureId, Vertex};

use super::{ArrayImage, Image, Mesh, MeshDrawMode, MeshError, MeshVertex, Quad, Transform};
use crate::Color;

/// Many copies of one image, or of parts of it, kept ready to draw in one
/// go. The equivalent of LÖVE's `SpriteBatch`.
///
/// Add sprites once, then draw them all every frame with
/// [`crate::graphics::draw_sprite_batch`]. This is the fast way to draw
/// things that rarely change, such as a tile map: the work of placing each
/// sprite is done when it is added instead of on every frame.
///
/// ```no_run
/// # use smitten::graphics::{self, Quad};
/// let tiles = graphics::new_image("tiles.png");
/// let grass = Quad::new(0.0, 0.0, 16.0, 16.0, 64.0, 64.0);
/// let mut map = graphics::new_sprite_batch(&tiles, 1000);
/// for column in 0..40 {
///     map.add_quad(&grass, column as f32 * 16.0, 0.0);
/// }
/// // Every frame, in `draw`:
/// graphics::draw_sprite_batch(&map, 0.0, 0.0);
/// ```
///
/// A batch can also draw the layers of an [`ArrayImage`], made with
/// [`SpriteBatch::from_array_image`]. Each sprite then picks its layer,
/// with [`SpriteBatch::add_layer`] and friends, so a tile map can draw
/// tiles from many layers in one batch.
///
/// Unlike LÖVE's, a batch grows as needed: the size given when creating
/// it only reserves memory up front.
///
/// A shader can read more per-vertex values than the batch has, taken
/// from a [`Mesh`] with [`SpriteBatch::attach_attribute`].
#[derive(Debug, Clone)]
pub struct SpriteBatch {
    source: Source,
    /// Six vertices per sprite, in the batch's own coordinates.
    vertices: Vec<Vertex>,
    /// The layer each sprite shows. Always 0 for a batch of an [`Image`].
    layers: Vec<u32>,
    /// Tints the sprites added or changed from now on.
    color: Color,
    /// The sprites drawn, as start and count.
    draw_range: Option<(usize, usize)>,
    usage: SpriteBatchUsage,
    /// Attributes taken from meshes, as attribute name and mesh.
    attachments: Vec<(String, Mesh)>,
    /// With attachments, the sprites as a mesh that attaches them, made
    /// when first drawn after a change.
    mesh: RefCell<Option<Mesh>>,
}

/// How often a batch's sprites are expected to change, the equivalent of
/// LÖVE's `SpriteBatchUsage`.
///
/// It is only a hint. Smitten sends a batch's sprites to the GPU each
/// time it is drawn whatever the usage, so every usage is equally fast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SpriteBatchUsage {
    /// Changed now and then. LÖVE's default.
    #[default]
    Dynamic,
    /// Set up once and rarely changed, such as a tile map.
    Static,
    /// Rebuilt every frame.
    Stream,
}

/// What a batch draws.
#[derive(Debug, Clone)]
enum Source {
    Image(Image),
    Array(ArrayImage),
}

const VERTICES_PER_SPRITE: usize = 6;

impl SpriteBatch {
    /// Creates an empty batch that draws `image`, with room reserved for
    /// `capacity` sprites.
    pub fn new(image: &Image, capacity: usize) -> SpriteBatch {
        SpriteBatch::with_usage(image, capacity, SpriteBatchUsage::default())
    }

    /// Creates an empty batch as [`SpriteBatch::new`] does, with a usage
    /// hint, the third argument of LÖVE's `newSpriteBatch`.
    pub fn with_usage(image: &Image, capacity: usize, usage: SpriteBatchUsage) -> SpriteBatch {
        SpriteBatch::with_source(Source::Image(image.clone()), capacity, usage)
    }

    /// Creates an empty batch that draws layers of `image`, with room
    /// reserved for `capacity` sprites, the equivalent of `newSpriteBatch`
    /// with an `ArrayImage`. Add sprites with [`SpriteBatch::add_layer`];
    /// the plain `add` methods show layer 0.
    pub fn from_array_image(
        image: &ArrayImage,
        capacity: usize,
        usage: SpriteBatchUsage,
    ) -> SpriteBatch {
        SpriteBatch::with_source(Source::Array(image.clone()), capacity, usage)
    }

    fn with_source(source: Source, capacity: usize, usage: SpriteBatchUsage) -> SpriteBatch {
        SpriteBatch {
            source,
            vertices: Vec::with_capacity(capacity * VERTICES_PER_SPRITE),
            layers: Vec::with_capacity(capacity),
            color: Color::WHITE,
            draw_range: None,
            usage,
            attachments: Vec::new(),
            mesh: RefCell::new(None),
        }
    }

    /// The usage hint the batch was created with.
    pub fn usage(&self) -> SpriteBatchUsage {
        self.usage
    }

    /// How many sprites the batch has room for before it next grows, the
    /// equivalent of `SpriteBatch:getBufferSize`. It is at least the size
    /// it was created with.
    pub fn buffer_size(&self) -> usize {
        self.vertices.capacity() / VERTICES_PER_SPRITE
    }

    /// The sprites that are drawn, as `(start, count)`, or `None` for all
    /// of them.
    pub fn draw_range(&self) -> Option<(usize, usize)> {
        self.draw_range
    }

    /// Draws only `count` sprites from index `start`, the equivalent of
    /// `SpriteBatch:setDrawRange`. `None` draws them all again. Sprites
    /// outside the range are kept, just not drawn.
    pub fn set_draw_range(&mut self, range: Option<(usize, usize)>) {
        self.changed();
        self.draw_range = range;
    }

    /// Does nothing: a batch's sprites are read straight from it each time
    /// it is drawn. LÖVE's `SpriteBatch:flush` sends changed sprites to
    /// the GPU early, so ported code can keep calling this.
    pub fn flush(&self) {}

    /// Adds the whole image with its top-left corner at `(x, y)` and
    /// returns the sprite's index, for use with the `set` methods.
    pub fn add(&mut self, x: f32, y: f32) -> usize {
        self.add_ex(x, y, 0.0, 1.0, 1.0, 0.0, 0.0)
    }

    /// Adds the whole image with the full set of LÖVE's `draw` arguments.
    /// See [`crate::graphics::draw_ex`] for what they mean.
    #[allow(clippy::too_many_arguments)] // Mirrors SpriteBatch:add.
    pub fn add_ex(&mut self, x: f32, y: f32, r: f32, sx: f32, sy: f32, ox: f32, oy: f32) -> usize {
        let quad = self.whole();
        self.add_quad_ex(&quad, x, y, r, sx, sy, ox, oy)
    }

    /// Adds the part of the image described by `quad` at `(x, y)`.
    pub fn add_quad(&mut self, quad: &Quad, x: f32, y: f32) -> usize {
        self.add_quad_ex(quad, x, y, 0.0, 1.0, 1.0, 0.0, 0.0)
    }

    /// Adds part of the image with the full set of LÖVE's `draw`
    /// arguments.
    #[allow(clippy::too_many_arguments)] // Mirrors SpriteBatch:add.
    pub fn add_quad_ex(
        &mut self,
        quad: &Quad,
        x: f32,
        y: f32,
        r: f32,
        sx: f32,
        sy: f32,
        ox: f32,
        oy: f32,
    ) -> usize {
        self.push_sprite(0, quad, &Transform::from_draw_args(x, y, r, sx, sy, ox, oy))
    }

    /// Adds the whole of layer `layer` of the batch's [`ArrayImage`] at
    /// `(x, y)` and returns the sprite's index, the equivalent of
    /// `SpriteBatch:addLayer`.
    ///
    /// Panics if the batch draws a plain [`Image`], or the layer does not
    /// exist.
    pub fn add_layer(&mut self, layer: u32, x: f32, y: f32) -> usize {
        self.add_layer_ex(layer, x, y, 0.0, 1.0, 1.0, 0.0, 0.0)
    }

    /// Adds a whole layer with the full set of LÖVE's `draw` arguments.
    #[allow(clippy::too_many_arguments)] // Mirrors SpriteBatch:addLayer.
    pub fn add_layer_ex(
        &mut self,
        layer: u32,
        x: f32,
        y: f32,
        r: f32,
        sx: f32,
        sy: f32,
        ox: f32,
        oy: f32,
    ) -> usize {
        let quad = self.whole();
        self.add_layer_quad_ex(layer, &quad, x, y, r, sx, sy, ox, oy)
    }

    /// Adds the part of layer `layer` described by `quad` at `(x, y)`.
    pub fn add_layer_quad(&mut self, layer: u32, quad: &Quad, x: f32, y: f32) -> usize {
        self.add_layer_quad_ex(layer, quad, x, y, 0.0, 1.0, 1.0, 0.0, 0.0)
    }

    /// Adds part of a layer with the full set of LÖVE's `draw` arguments.
    #[allow(clippy::too_many_arguments)] // Mirrors SpriteBatch:addLayer.
    pub fn add_layer_quad_ex(
        &mut self,
        layer: u32,
        quad: &Quad,
        x: f32,
        y: f32,
        r: f32,
        sx: f32,
        sy: f32,
        ox: f32,
        oy: f32,
    ) -> usize {
        self.check_layer(layer);
        self.push_sprite(
            layer,
            quad,
            &Transform::from_draw_args(x, y, r, sx, sy, ox, oy),
        )
    }

    fn push_sprite(&mut self, layer: u32, quad: &Quad, transform: &Transform) -> usize {
        let index = self.count();
        let sprite = self.sprite(quad, transform);
        self.changed();
        self.vertices.extend_from_slice(&sprite);
        self.layers.push(layer);
        index
    }

    /// Moves the sprite at `index` so it shows the whole image at
    /// `(x, y)`. Its tint becomes the batch's current color.
    ///
    /// Panics if no sprite has that index.
    pub fn set(&mut self, index: usize, x: f32, y: f32) {
        self.set_ex(index, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
    }

    /// Replaces the sprite at `index`, with the full set of LÖVE's `draw`
    /// arguments.
    #[allow(clippy::too_many_arguments)] // Mirrors SpriteBatch:set.
    pub fn set_ex(
        &mut self,
        index: usize,
        x: f32,
        y: f32,
        r: f32,
        sx: f32,
        sy: f32,
        ox: f32,
        oy: f32,
    ) {
        let quad = self.whole();
        self.set_quad_ex(index, &quad, x, y, r, sx, sy, ox, oy);
    }

    /// Replaces the sprite at `index` with the part of the image described
    /// by `quad`, at `(x, y)`.
    pub fn set_quad(&mut self, index: usize, quad: &Quad, x: f32, y: f32) {
        self.set_quad_ex(index, quad, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
    }

    /// Replaces the sprite at `index` with part of the image, with the
    /// full set of LÖVE's `draw` arguments.
    #[allow(clippy::too_many_arguments)] // Mirrors SpriteBatch:set.
    pub fn set_quad_ex(
        &mut self,
        index: usize,
        quad: &Quad,
        x: f32,
        y: f32,
        r: f32,
        sx: f32,
        sy: f32,
        ox: f32,
        oy: f32,
    ) {
        self.replace_sprite(
            index,
            0,
            quad,
            &Transform::from_draw_args(x, y, r, sx, sy, ox, oy),
        );
    }

    /// Replaces the sprite at `index` with the whole of layer `layer` at
    /// `(x, y)`, the equivalent of `SpriteBatch:setLayer`.
    ///
    /// Panics if no sprite has that index, the batch draws a plain
    /// [`Image`], or the layer does not exist.
    pub fn set_layer(&mut self, index: usize, layer: u32, x: f32, y: f32) {
        self.set_layer_ex(index, layer, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
    }

    /// Replaces the sprite at `index` with a whole layer, with the full
    /// set of LÖVE's `draw` arguments.
    #[allow(clippy::too_many_arguments)] // Mirrors SpriteBatch:setLayer.
    pub fn set_layer_ex(
        &mut self,
        index: usize,
        layer: u32,
        x: f32,
        y: f32,
        r: f32,
        sx: f32,
        sy: f32,
        ox: f32,
        oy: f32,
    ) {
        let quad = self.whole();
        self.set_layer_quad_ex(index, layer, &quad, x, y, r, sx, sy, ox, oy);
    }

    /// Replaces the sprite at `index` with the part of layer `layer`
    /// described by `quad`, at `(x, y)`.
    pub fn set_layer_quad(&mut self, index: usize, layer: u32, quad: &Quad, x: f32, y: f32) {
        self.set_layer_quad_ex(index, layer, quad, x, y, 0.0, 1.0, 1.0, 0.0, 0.0);
    }

    /// Replaces the sprite at `index` with part of a layer, with the full
    /// set of LÖVE's `draw` arguments.
    #[allow(clippy::too_many_arguments)] // Mirrors SpriteBatch:setLayer.
    pub fn set_layer_quad_ex(
        &mut self,
        index: usize,
        layer: u32,
        quad: &Quad,
        x: f32,
        y: f32,
        r: f32,
        sx: f32,
        sy: f32,
        ox: f32,
        oy: f32,
    ) {
        self.check_layer(layer);
        self.replace_sprite(
            index,
            layer,
            quad,
            &Transform::from_draw_args(x, y, r, sx, sy, ox, oy),
        );
    }

    /// The layer the sprite at `index` shows. Always 0 in a batch of a
    /// plain [`Image`].
    ///
    /// Panics if no sprite has that index.
    pub fn layer(&self, index: usize) -> u32 {
        self.check_index(index);
        self.layers[index]
    }

    fn check_index(&self, index: usize) {
        let count = self.count();
        assert!(
            index < count,
            "sprite index {index} is out of range: the batch holds {count} sprites"
        );
    }

    fn check_layer(&self, layer: u32) {
        let Source::Array(image) = &self.source else {
            panic!("only a sprite batch of an ArrayImage has layers");
        };
        assert!(
            layer < image.layer_count(),
            "layer {layer} does not exist: the array image has {} layers",
            image.layer_count()
        );
    }

    fn replace_sprite(&mut self, index: usize, layer: u32, quad: &Quad, transform: &Transform) {
        self.check_index(index);
        let sprite = self.sprite(quad, transform);
        let start = index * VERTICES_PER_SPRITE;
        self.changed();
        self.vertices[start..start + VERTICES_PER_SPRITE].copy_from_slice(&sprite);
        self.layers[index] = layer;
    }

    /// Tints the sprites added or changed from now on. Sprites already in
    /// the batch keep their tint. The color set with
    /// [`crate::graphics::set_color`] when the batch is drawn tints all of
    /// them on top of this.
    pub fn set_color(&mut self, color: Color) {
        self.color = color;
    }

    /// The tint given to sprites added from now on. White by default.
    pub fn color(&self) -> Color {
        self.color
    }

    /// Removes every sprite. The reserved memory is kept, so a batch that
    /// is rebuilt every frame does not allocate.
    pub fn clear(&mut self) {
        self.changed();
        self.vertices.clear();
        self.layers.clear();
    }

    /// How many sprites the batch holds.
    pub fn count(&self) -> usize {
        self.vertices.len() / VERTICES_PER_SPRITE
    }

    /// The image the batch draws, or `None` if it draws an
    /// [`ArrayImage`].
    pub fn image(&self) -> Option<&Image> {
        match &self.source {
            Source::Image(image) => Some(image),
            Source::Array(_) => None,
        }
    }

    /// The array image the batch draws, or `None` if it draws a plain
    /// [`Image`].
    pub fn array_image(&self) -> Option<&ArrayImage> {
        match &self.source {
            Source::Array(image) => Some(image),
            Source::Image(_) => None,
        }
    }

    /// Draws the batch with a different image from now on. Quads made for
    /// the old image's size will show the same fraction of the new one.
    /// Every sprite shows layer 0 if the batch drew an [`ArrayImage`].
    pub fn set_image(&mut self, image: &Image) {
        self.source = Source::Image(image.clone());
        self.changed();
        self.layers.fill(0);
    }

    /// Draws the batch with a different array image from now on. Sprites
    /// keep their layers, so the new image needs at least as many.
    ///
    /// Panics if a sprite shows a layer the new image does not have.
    pub fn set_array_image(&mut self, image: &ArrayImage) {
        if let Some(&highest) = self.layers.iter().max() {
            assert!(
                highest < image.layer_count(),
                "a sprite shows layer {highest}, but the array image has {} layers",
                image.layer_count()
            );
        }
        self.source = Source::Array(image.clone());
    }

    /// Draws the batch with the per-vertex attribute `name` of `mesh` as
    /// well as its own, the equivalent of LÖVE's
    /// `SpriteBatch:attachAttribute`. A shader reads it from its
    /// `Attributes` struct, as it would for a mesh; see
    /// [`Mesh::attach_attribute`].
    ///
    /// Each sprite has four vertices, in LÖVE's order: sprite `i` takes
    /// the mesh's vertices `4 * i` to `4 * i + 3`, for its top-left,
    /// bottom-left, top-right and bottom-right corners as the quad is
    /// drawn. The mesh needs at least four vertices for each sprite in the
    /// batch when it is drawn, or drawing panics. Changing the mesh's
    /// vertices changes what is drawn from the next draw on.
    ///
    /// Attaching another attribute of the same name replaces the first.
    /// Fails if the mesh has no attribute `name`.
    pub fn attach_attribute(&mut self, name: &str, mesh: &Mesh) -> Result<(), MeshError> {
        if !mesh
            .vertex_format()
            .iter()
            .any(|attribute| attribute.name == name)
        {
            return Err(super::mesh::error(format!(
                "the mesh being attached has no attribute `{name}`"
            )));
        }
        let mut attachments = self.attachments.clone();
        attachments.retain(|(attached, _)| attached != name);
        attachments.push((name.to_string(), mesh.share()));
        // Checks the count of attributes the way a mesh does.
        let mut probe = Mesh::new(&[], MeshDrawMode::Triangles);
        for (name, mesh) in &attachments {
            probe.attach_attribute(name, mesh)?;
        }
        self.attachments = attachments;
        self.changed();
        Ok(())
    }

    /// Stops drawing an attribute attached with
    /// [`SpriteBatch::attach_attribute`]. Returns whether there was one.
    pub fn detach_attribute(&mut self, name: &str) -> bool {
        let before = self.attachments.len();
        self.attachments.retain(|(attached, _)| attached != name);
        self.changed();
        self.attachments.len() != before
    }

    /// Notes that the sprites drawn changed, so the mesh drawn for
    /// attachments is made again.
    fn changed(&mut self) {
        *self.mesh.get_mut() = None;
    }

    /// With attached attributes, the drawn sprites as a mesh of separate
    /// triangles in the batch's own coordinates, four vertices to a
    /// sprite, that attaches them. Its triangles are in the order of
    /// [`SpriteBatch::layer_runs`], six indices to a sprite. `None`
    /// without attachments.
    pub(crate) fn attached_mesh(&self) -> Option<std::cell::Ref<'_, Mesh>> {
        if self.attachments.is_empty() {
            return None;
        }
        if self.mesh.borrow().is_none() {
            let corners: Vec<MeshVertex> = self
                .vertices
                .chunks_exact(VERTICES_PER_SPRITE)
                // The six vertices are top-left, top-right, bottom-right,
                // top-left, bottom-right, bottom-left.
                .flat_map(|sprite| [sprite[0], sprite[5], sprite[1], sprite[2]])
                .map(|vertex| MeshVertex {
                    x: vertex.position[0],
                    y: vertex.position[1],
                    u: vertex.uv[0],
                    v: vertex.uv[1],
                    color: Color::rgba(
                        vertex.color[0],
                        vertex.color[1],
                        vertex.color[2],
                        vertex.color[3],
                    ),
                })
                .collect();
            let indices: Vec<u32> = self
                .drawn()
                .flat_map(|sprite| {
                    let first = sprite as u32 * 4;
                    [0, 2, 3, 0, 3, 1].map(|corner| first + corner)
                })
                .collect();
            let mut mesh = Mesh::new(&corners, MeshDrawMode::Triangles);
            mesh.set_vertex_map(Some(&indices));
            for (name, source) in &self.attachments {
                mesh.attach_attribute(name, source)
                    .expect("checked when attached");
            }
            *self.mesh.borrow_mut() = Some(mesh);
        }
        Some(std::cell::Ref::map(self.mesh.borrow(), |mesh| {
            mesh.as_ref().expect("made above")
        }))
    }

    /// The texture to draw with, and the filter to sample it with.
    pub(crate) fn texture(&self) -> (TextureId, Filter) {
        match &self.source {
            Source::Image(image) => (image.texture(), image.filter()),
            Source::Array(image) => (image.texture(), image.filter()),
        }
    }

    /// Whether the batch draws layers of an [`ArrayImage`].
    pub(crate) fn is_layered(&self) -> bool {
        matches!(self.source, Source::Array(_))
    }

    /// A quad covering the whole image, or one whole layer.
    fn whole(&self) -> Quad {
        let (width, height) = match &self.source {
            Source::Image(image) => (image.width() as f32, image.height() as f32),
            Source::Array(image) => (image.width() as f32, image.height() as f32),
        };
        Quad::new(0.0, 0.0, width, height, width, height)
    }

    /// The sprites that are drawn, after the draw range.
    fn drawn(&self) -> Range<usize> {
        let count = self.count();
        match self.draw_range {
            Some((start, length)) => {
                let start = start.min(count);
                start..start.saturating_add(length).min(count)
            }
            None => 0..count,
        }
    }

    /// The drawn sprites split into runs that show the same layer, as
    /// `(layer, vertices)` with the vertex range counted from the first
    /// vertex [`SpriteBatch::write_vertices`] writes.
    pub(crate) fn layer_runs(&self) -> Vec<(u32, Range<usize>)> {
        let drawn = self.drawn();
        let mut runs: Vec<(u32, Range<usize>)> = Vec::new();
        for (offset, &layer) in self.layers[drawn].iter().enumerate() {
            let start = offset * VERTICES_PER_SPRITE;
            let end = start + VERTICES_PER_SPRITE;
            match runs.last_mut() {
                Some((last, range)) if *last == layer => range.end = end,
                _ => runs.push((layer, start..end)),
            }
        }
        runs
    }

    fn sprite(&self, quad: &Quad, transform: &Transform) -> [Vertex; VERTICES_PER_SPRITE] {
        let color = [self.color.r, self.color.g, self.color.b, self.color.a];
        let (u0, v0) = (quad.x / quad.sheet_width, quad.y / quad.sheet_height);
        let (u1, v1) = (
            (quad.x + quad.width) / quad.sheet_width,
            (quad.y + quad.height) / quad.sheet_height,
        );
        let corner = |x: f32, y: f32, u: f32, v: f32| {
            let (x, y) = transform.apply(x, y);
            Vertex {
                position: [x, y],
                uv: [u, v],
                color,
            }
        };
        let top_left = corner(0.0, 0.0, u0, v0);
        let top_right = corner(quad.width, 0.0, u1, v0);
        let bottom_right = corner(quad.width, quad.height, u1, v1);
        let bottom_left = corner(0.0, quad.height, u0, v1);
        [
            top_left,
            top_right,
            bottom_right,
            top_left,
            bottom_right,
            bottom_left,
        ]
    }

    /// Appends the batch's triangles to `out`, placed by `transform` and
    /// tinted by `tint`.
    pub(crate) fn write_vertices(&self, transform: &Transform, tint: Color, out: &mut Vec<Vertex>) {
        let drawn = self.drawn();
        let vertices =
            &self.vertices[drawn.start * VERTICES_PER_SPRITE..drawn.end * VERTICES_PER_SPRITE];
        out.reserve(vertices.len());
        out.extend(vertices.iter().map(|vertex| {
            let (x, y) = transform.apply(vertex.position[0], vertex.position[1]);
            Vertex {
                position: [x, y],
                uv: vertex.uv,
                color: [
                    vertex.color[0] * tint.r,
                    vertex.color[1] * tint.g,
                    vertex.color[2] * tint.b,
                    vertex.color[3] * tint.a,
                ],
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smitten_core::Filter;
    use smitten_core::backend::TextureId;

    use crate::graphics::stats::Kind;

    fn batch() -> SpriteBatch {
        let image = Image::from_texture(TextureId(1), 64, 32, Filter::Nearest, Kind::Image);
        SpriteBatch::new(&image, 4)
    }

    fn drawn(batch: &SpriteBatch, transform: &Transform, tint: Color) -> Vec<Vertex> {
        let mut out = Vec::new();
        batch.write_vertices(transform, tint, &mut out);
        out
    }

    #[test]
    fn the_draw_range_limits_what_is_drawn() {
        let mut batch = batch();
        for index in 0..5 {
            batch.add(index as f32 * 100.0, 0.0);
        }
        batch.set_draw_range(Some((1, 2)));
        let out = drawn(&batch, &Transform::IDENTITY, Color::WHITE);
        assert_eq!(out.len(), 2 * VERTICES_PER_SPRITE);
        assert_eq!(out[0].position[0], 100.0);
        assert_eq!(batch.count(), 5);

        batch.set_draw_range(Some((4, 10)));
        assert_eq!(drawn(&batch, &Transform::IDENTITY, Color::WHITE).len(), 6);
        batch.set_draw_range(Some((9, 1)));
        assert!(drawn(&batch, &Transform::IDENTITY, Color::WHITE).is_empty());
        batch.set_draw_range(None);
        assert_eq!(drawn(&batch, &Transform::IDENTITY, Color::WHITE).len(), 30);
    }

    #[test]
    fn attached_attributes_draw_the_sprites_as_a_mesh_of_four_corners_each() {
        use crate::graphics::{VertexAttribute, VertexDataType};

        let mut batch = batch();
        assert!(batch.attached_mesh().is_none());
        batch.add(0.0, 0.0);
        batch.add(100.0, 0.0);
        let shades = Mesh::with_format_and_count(
            &[VertexAttribute::new("Shade", VertexDataType::Float, 1)],
            8,
            MeshDrawMode::Triangles,
        )
        .unwrap();
        assert!(batch.attach_attribute("Missing", &shades).is_err());
        batch.attach_attribute("Shade", &shades).unwrap();
        batch.set_draw_range(Some((1, 1)));

        let mesh = batch.attached_mesh().unwrap();
        assert_eq!(mesh.vertex_count(), 8);
        // Top-left, bottom-left, top-right and bottom-right, as in LÖVE.
        let corners: Vec<(f32, f32)> = (4..8)
            .map(|index| {
                let vertex = mesh.vertex(index).unwrap();
                (vertex.x, vertex.y)
            })
            .collect();
        assert_eq!(
            corners,
            [(100.0, 0.0), (100.0, 32.0), (164.0, 0.0), (164.0, 32.0)]
        );
        assert_eq!(mesh.vertex_map(), Some(&[4, 6, 7, 4, 7, 5][..]));
        drop(mesh);

        // A change makes the mesh again.
        batch.set_draw_range(None);
        assert_eq!(
            batch.attached_mesh().unwrap().vertex_map().unwrap().len(),
            12
        );
        assert!(batch.detach_attribute("Shade"));
        assert!(batch.attached_mesh().is_none());
    }

    #[test]
    fn the_buffer_holds_at_least_what_was_asked_for() {
        assert!(batch().buffer_size() >= 4);
    }

    #[test]
    fn add_returns_indexes_in_order_and_counts_sprites() {
        let mut batch = batch();
        assert_eq!(batch.add(0.0, 0.0), 0);
        assert_eq!(batch.add(10.0, 0.0), 1);
        assert_eq!(batch.count(), 2);
        batch.clear();
        assert_eq!(batch.count(), 0);
        assert_eq!(batch.add(0.0, 0.0), 0);
    }

    #[test]
    fn a_sprite_covers_the_whole_image_where_it_was_added() {
        let mut batch = batch();
        batch.add(100.0, 50.0);
        let vertices = drawn(&batch, &Transform::IDENTITY, Color::WHITE);

        assert_eq!(vertices.len(), 6);
        assert_eq!(vertices[0].position, [100.0, 50.0]);
        assert_eq!(vertices[0].uv, [0.0, 0.0]);
        // The third vertex is the bottom-right corner.
        assert_eq!(vertices[2].position, [164.0, 82.0]);
        assert_eq!(vertices[2].uv, [1.0, 1.0]);
    }

    #[test]
    fn a_quad_picks_part_of_the_image() {
        let mut batch = batch();
        let quad = Quad::new(32.0, 0.0, 16.0, 16.0, 64.0, 32.0);
        batch.add_quad(&quad, 0.0, 0.0);
        let vertices = drawn(&batch, &Transform::IDENTITY, Color::WHITE);

        assert_eq!(vertices[0].uv, [0.5, 0.0]);
        assert_eq!(vertices[2].uv, [0.75, 0.5]);
        assert_eq!(vertices[2].position, [16.0, 16.0]);
    }

    #[test]
    fn set_replaces_one_sprite_and_leaves_the_rest() {
        let mut batch = batch();
        batch.add(0.0, 0.0);
        batch.add(10.0, 0.0);
        batch.set(0, 500.0, 500.0);
        let vertices = drawn(&batch, &Transform::IDENTITY, Color::WHITE);

        assert_eq!(batch.count(), 2);
        assert_eq!(vertices[0].position, [500.0, 500.0]);
        assert_eq!(vertices[6].position, [10.0, 0.0]);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn setting_a_sprite_that_was_never_added_panics() {
        let mut batch = batch();
        batch.add(0.0, 0.0);
        batch.set(1, 0.0, 0.0);
    }

    #[test]
    fn each_sprite_keeps_the_color_it_was_added_with() {
        let mut batch = batch();
        batch.set_color(Color::rgb(1.0, 0.0, 0.0));
        batch.add(0.0, 0.0);
        batch.set_color(Color::WHITE);
        batch.add(0.0, 0.0);
        let vertices = drawn(
            &batch,
            &Transform::IDENTITY,
            Color::rgba(1.0, 1.0, 1.0, 0.5),
        );

        // The tint passed when drawing multiplies each sprite's own color.
        assert_eq!(vertices[0].color, [1.0, 0.0, 0.0, 0.5]);
        assert_eq!(vertices[6].color, [1.0, 1.0, 1.0, 0.5]);
    }

    #[test]
    fn drawing_moves_every_sprite_by_the_transform() {
        let mut batch = batch();
        batch.add(1.0, 2.0);
        let mut transform = Transform::IDENTITY;
        transform.translate(10.0, 20.0);
        transform.scale(2.0, 2.0);
        let vertices = drawn(&batch, &transform, Color::WHITE);

        assert_eq!(vertices[0].position, [12.0, 24.0]);
    }

    fn layered() -> SpriteBatch {
        let image = ArrayImage::for_tests(TextureId(2), 16, 16, 3);
        SpriteBatch::from_array_image(&image, 4, SpriteBatchUsage::default())
    }

    #[test]
    fn sprites_of_an_array_image_keep_their_layers() {
        let mut batch = layered();
        assert_eq!(batch.add_layer(2, 0.0, 0.0), 0);
        assert_eq!(batch.add(16.0, 0.0), 1);
        assert_eq!(batch.layer(0), 2);
        assert_eq!(batch.layer(1), 0);

        batch.set_layer(1, 1, 32.0, 0.0);
        assert_eq!(batch.layer(1), 1);
        let vertices = drawn(&batch, &Transform::IDENTITY, Color::WHITE);
        // A whole layer is the size of the array image.
        assert_eq!(vertices[2].position, [16.0, 16.0]);
        assert_eq!(vertices[6].position, [32.0, 0.0]);
        assert!(batch.image().is_none());
        assert!(batch.array_image().is_some());
    }

    #[test]
    fn neighbouring_sprites_on_one_layer_share_a_run() {
        let mut batch = layered();
        for layer in [0, 0, 1, 1, 1, 0] {
            batch.add_layer(layer, 0.0, 0.0);
        }
        assert_eq!(batch.layer_runs(), [(0, 0..12), (1, 12..30), (0, 30..36)]);
        // Runs are counted from the first sprite drawn.
        batch.set_draw_range(Some((1, 3)));
        assert_eq!(batch.layer_runs(), [(0, 0..6), (1, 6..18)]);
        assert_eq!(drawn(&batch, &Transform::IDENTITY, Color::WHITE).len(), 18);
    }

    #[test]
    #[should_panic(expected = "layer 3 does not exist")]
    fn a_missing_layer_panics() {
        layered().add_layer(3, 0.0, 0.0);
    }

    #[test]
    #[should_panic(expected = "only a sprite batch of an ArrayImage has layers")]
    fn a_plain_image_has_no_layers() {
        batch().add_layer(0, 0.0, 0.0);
    }

    #[test]
    fn switching_to_a_plain_image_shows_layer_zero() {
        let mut batch = layered();
        batch.add_layer(2, 0.0, 0.0);
        let image = Image::from_texture(TextureId(1), 64, 32, Filter::Nearest, Kind::Image);
        batch.set_image(&image);
        assert_eq!(batch.layer(0), 0);
        assert!(!batch.is_layered());
    }
}
