use smitten_core::backend::Vertex;

use super::{Image, Quad, Transform};
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
/// Unlike LÖVE's, a batch grows as needed: the size given when creating
/// it only reserves memory up front.
#[derive(Debug, Clone)]
pub struct SpriteBatch {
    image: Image,
    /// Six vertices per sprite, in the batch's own coordinates.
    vertices: Vec<Vertex>,
    /// Tints the sprites added or changed from now on.
    color: Color,
}

const VERTICES_PER_SPRITE: usize = 6;

impl SpriteBatch {
    /// Creates an empty batch that draws `image`, with room reserved for
    /// `capacity` sprites.
    pub fn new(image: &Image, capacity: usize) -> SpriteBatch {
        SpriteBatch {
            image: image.clone(),
            vertices: Vec::with_capacity(capacity * VERTICES_PER_SPRITE),
            color: Color::WHITE,
        }
    }

    /// Adds the whole image with its top-left corner at `(x, y)` and
    /// returns the sprite's index, for use with the `set` methods.
    pub fn add(&mut self, x: f32, y: f32) -> usize {
        self.add_ex(x, y, 0.0, 1.0, 1.0, 0.0, 0.0)
    }

    /// Adds the whole image with the full set of LÖVE's `draw` arguments.
    /// See [`crate::graphics::draw_ex`] for what they mean.
    #[allow(clippy::too_many_arguments)] // Mirrors SpriteBatch:add.
    pub fn add_ex(&mut self, x: f32, y: f32, r: f32, sx: f32, sy: f32, ox: f32, oy: f32) -> usize {
        let quad = super::whole_image(&self.image);
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
        let index = self.count();
        let sprite = self.sprite(quad, &Transform::from_draw_args(x, y, r, sx, sy, ox, oy));
        self.vertices.extend_from_slice(&sprite);
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
        let quad = super::whole_image(&self.image);
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
        let count = self.count();
        assert!(
            index < count,
            "sprite index {index} is out of range: the batch holds {count} sprites"
        );
        let sprite = self.sprite(quad, &Transform::from_draw_args(x, y, r, sx, sy, ox, oy));
        let start = index * VERTICES_PER_SPRITE;
        self.vertices[start..start + VERTICES_PER_SPRITE].copy_from_slice(&sprite);
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
        self.vertices.clear();
    }

    /// How many sprites the batch holds.
    pub fn count(&self) -> usize {
        self.vertices.len() / VERTICES_PER_SPRITE
    }

    /// The image the batch draws.
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// Draws the batch with a different image from now on. Quads made for
    /// the old image's size will show the same fraction of the new one.
    pub fn set_image(&mut self, image: &Image) {
        self.image = image.clone();
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
        out.reserve(self.vertices.len());
        out.extend(self.vertices.iter().map(|vertex| {
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

    fn batch() -> SpriteBatch {
        let image = Image::from_texture(TextureId(1), 64, 32, Filter::Nearest);
        SpriteBatch::new(&image, 4)
    }

    fn drawn(batch: &SpriteBatch, transform: &Transform, tint: Color) -> Vec<Vertex> {
        let mut out = Vec::new();
        batch.write_vertices(transform, tint, &mut out);
        out
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
}
