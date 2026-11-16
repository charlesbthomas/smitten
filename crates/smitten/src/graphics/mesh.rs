use smitten_core::backend::Vertex;

use super::{Image, Transform};
use crate::Color;

/// One corner of a [`Mesh`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshVertex {
    /// Position in the mesh's own coordinates.
    pub x: f32,
    pub y: f32,
    /// Where in the texture this corner is, from 0 to 1 across and down.
    pub u: f32,
    pub v: f32,
    /// Multiplied with the texture, and blended across each triangle.
    pub color: Color,
}

impl MeshVertex {
    /// A white corner at `(x, y)` showing the texture at `(u, v)`.
    pub fn new(x: f32, y: f32, u: f32, v: f32) -> MeshVertex {
        MeshVertex {
            x,
            y,
            u,
            v,
            color: Color::WHITE,
        }
    }

    /// A corner at `(x, y)` with a color and no particular place in a
    /// texture, for meshes drawn without one.
    pub fn colored(x: f32, y: f32, color: Color) -> MeshVertex {
        MeshVertex {
            x,
            y,
            u: 0.0,
            v: 0.0,
            color,
        }
    }
}

/// How a mesh's vertices are joined into triangles, as LÖVE's
/// `MeshDrawMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MeshDrawMode {
    /// Every triangle shares the first vertex, like a fan. Good for
    /// convex shapes and circles.
    #[default]
    Fan,
    /// Each vertex after the second makes a triangle with the two before
    /// it. Good for ribbons and trails.
    Strip,
    /// Every three vertices are a separate triangle.
    Triangles,
}

/// A shape made of triangles with a position, texture coordinate and color
/// at every corner. The equivalent of LÖVE's `Mesh`.
///
/// Use it for what images and shapes cannot do: gradients, a texture
/// stretched over an arbitrary shape, trails, or terrain. Draw it with
/// [`crate::graphics::draw_mesh`].
///
/// ```no_run
/// # use smitten::graphics::{self, MeshDrawMode, MeshVertex};
/// # use smitten::Color;
/// // A square that fades from red on the left to blue on the right.
/// let red = Color::rgb(1.0, 0.0, 0.0);
/// let blue = Color::rgb(0.0, 0.0, 1.0);
/// let gradient = graphics::new_mesh(
///     &[
///         MeshVertex::colored(0.0, 0.0, red),
///         MeshVertex::colored(100.0, 0.0, blue),
///         MeshVertex::colored(100.0, 100.0, blue),
///         MeshVertex::colored(0.0, 100.0, red),
///     ],
///     MeshDrawMode::Fan,
/// );
/// graphics::draw_mesh(&gradient, 50.0, 50.0);
/// ```
#[derive(Debug, Clone)]
pub struct Mesh {
    vertices: Vec<MeshVertex>,
    mode: MeshDrawMode,
    texture: Option<Image>,
    /// The order to visit the vertices in, if not first to last.
    vertex_map: Option<Vec<u32>>,
    /// The part of the visiting order that is drawn, as start and count.
    draw_range: Option<(usize, usize)>,
}

impl Mesh {
    /// Creates a mesh from its vertices.
    pub fn new(vertices: &[MeshVertex], mode: MeshDrawMode) -> Mesh {
        Mesh {
            vertices: vertices.to_vec(),
            mode,
            texture: None,
            vertex_map: None,
            draw_range: None,
        }
    }

    /// How many vertices the mesh has.
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// The vertex at `index`, counting from zero.
    pub fn vertex(&self, index: usize) -> Option<MeshVertex> {
        self.vertices.get(index).copied()
    }

    /// Replaces the vertex at `index`. Panics if there is none.
    pub fn set_vertex(&mut self, index: usize, vertex: MeshVertex) {
        self.vertices[index] = vertex;
    }

    /// All the vertices in order.
    pub fn vertices(&self) -> &[MeshVertex] {
        &self.vertices
    }

    /// Replaces all the vertices. The count may change.
    pub fn set_vertices(&mut self, vertices: &[MeshVertex]) {
        self.vertices.clear();
        self.vertices.extend_from_slice(vertices);
    }

    /// How the vertices are joined into triangles.
    pub fn draw_mode(&self) -> MeshDrawMode {
        self.mode
    }

    /// Changes how the vertices are joined into triangles.
    pub fn set_draw_mode(&mut self, mode: MeshDrawMode) {
        self.mode = mode;
    }

    /// The image stretched over the mesh, if any.
    pub fn texture(&self) -> Option<&Image> {
        self.texture.as_ref()
    }

    /// Sets the image stretched over the mesh by its vertices' `u` and
    /// `v`, or `None` to draw with the vertex colors alone.
    pub fn set_texture(&mut self, image: Option<&Image>) {
        self.texture = image.cloned();
    }

    /// The order the vertices are visited in, if one was set.
    pub fn vertex_map(&self) -> Option<&[u32]> {
        self.vertex_map.as_deref()
    }

    /// Sets the order to visit the vertices in, by index. A vertex can be
    /// named more than once, so corners shared between triangles are
    /// stored once. `None` goes back to first to last.
    ///
    /// An index with no vertex is skipped when drawing.
    pub fn set_vertex_map(&mut self, map: Option<&[u32]>) {
        self.vertex_map = map.map(<[u32]>::to_vec);
    }

    /// The part of the mesh that is drawn, as `(start, count)`.
    pub fn draw_range(&self) -> Option<(usize, usize)> {
        self.draw_range
    }

    /// Draws only `count` vertices from `start`, counted along the vertex
    /// map if there is one. `None` draws everything.
    pub fn set_draw_range(&mut self, range: Option<(usize, usize)>) {
        self.draw_range = range;
    }

    /// The mesh as a list of separate triangles, placed by `transform`
    /// and tinted by `tint`.
    pub(crate) fn write_vertices(&self, transform: &Transform, tint: Color, out: &mut Vec<Vertex>) {
        // The vertices in visiting order.
        let visited: Vec<MeshVertex> = match &self.vertex_map {
            Some(map) => map
                .iter()
                .filter_map(|&index| self.vertices.get(index as usize).copied())
                .collect(),
            None => self.vertices.clone(),
        };
        let visited = match self.draw_range {
            Some((start, count)) => {
                let start = start.min(visited.len());
                let end = start.saturating_add(count).min(visited.len());
                &visited[start..end]
            }
            None => &visited[..],
        };

        let place = |vertex: &MeshVertex| {
            let (x, y) = transform.apply(vertex.x, vertex.y);
            Vertex {
                position: [x, y],
                uv: [vertex.u, vertex.v],
                color: [
                    vertex.color.r * tint.r,
                    vertex.color.g * tint.g,
                    vertex.color.b * tint.b,
                    vertex.color.a * tint.a,
                ],
            }
        };
        match self.mode {
            MeshDrawMode::Triangles => {
                let whole = visited.len() - visited.len() % 3;
                out.extend(visited[..whole].iter().map(place));
            }
            MeshDrawMode::Fan => {
                for pair in visited.get(1..).unwrap_or_default().windows(2) {
                    out.extend([&visited[0], &pair[0], &pair[1]].map(place));
                }
            }
            MeshDrawMode::Strip => {
                for triple in visited.windows(3) {
                    out.extend(triple.iter().map(place));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corners(count: usize) -> Vec<MeshVertex> {
        (0..count)
            .map(|index| MeshVertex::new(index as f32, 0.0, 0.0, 0.0))
            .collect()
    }

    /// The x positions of the triangles a mesh turns into.
    fn triangles(mesh: &Mesh) -> Vec<f32> {
        let mut out = Vec::new();
        mesh.write_vertices(&Transform::IDENTITY, Color::WHITE, &mut out);
        assert_eq!(out.len() % 3, 0);
        out.iter().map(|vertex| vertex.position[0]).collect()
    }

    #[test]
    fn a_fan_shares_its_first_vertex() {
        let mesh = Mesh::new(&corners(5), MeshDrawMode::Fan);
        assert_eq!(
            triangles(&mesh),
            [0.0, 1.0, 2.0, 0.0, 2.0, 3.0, 0.0, 3.0, 4.0]
        );
    }

    #[test]
    fn a_strip_joins_each_vertex_to_the_two_before() {
        let mesh = Mesh::new(&corners(5), MeshDrawMode::Strip);
        assert_eq!(
            triangles(&mesh),
            [0.0, 1.0, 2.0, 1.0, 2.0, 3.0, 2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn separate_triangles_drop_leftover_vertices() {
        let mesh = Mesh::new(&corners(7), MeshDrawMode::Triangles);
        assert_eq!(triangles(&mesh), [0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn too_few_vertices_draw_nothing() {
        for mode in [
            MeshDrawMode::Fan,
            MeshDrawMode::Strip,
            MeshDrawMode::Triangles,
        ] {
            assert!(triangles(&Mesh::new(&corners(2), mode)).is_empty());
            assert!(triangles(&Mesh::new(&[], mode)).is_empty());
        }
    }

    #[test]
    fn a_vertex_map_reuses_vertices_and_skips_bad_indices() {
        let mut mesh = Mesh::new(&corners(4), MeshDrawMode::Triangles);
        mesh.set_vertex_map(Some(&[0, 1, 2, 0, 2, 3, 99]));
        assert_eq!(triangles(&mesh), [0.0, 1.0, 2.0, 0.0, 2.0, 3.0]);
        mesh.set_vertex_map(None);
        assert_eq!(triangles(&mesh), [0.0, 1.0, 2.0]);
    }

    #[test]
    fn a_draw_range_limits_what_is_drawn() {
        let mut mesh = Mesh::new(&corners(9), MeshDrawMode::Triangles);
        mesh.set_draw_range(Some((3, 3)));
        assert_eq!(triangles(&mesh), [3.0, 4.0, 5.0]);
        // A range past the end draws what there is.
        mesh.set_draw_range(Some((6, 100)));
        assert_eq!(triangles(&mesh), [6.0, 7.0, 8.0]);
        mesh.set_draw_range(Some((100, 3)));
        assert!(triangles(&mesh).is_empty());
    }

    #[test]
    fn vertex_colors_are_multiplied_by_the_tint_and_positions_transformed() {
        let half = Color::rgba(0.5, 1.0, 1.0, 0.5);
        let vertices = [
            MeshVertex::colored(0.0, 0.0, Color::rgba(1.0, 0.5, 0.0, 1.0)),
            MeshVertex::new(1.0, 0.0, 0.25, 0.75),
            MeshVertex::new(0.0, 1.0, 0.0, 0.0),
        ];
        let mesh = Mesh::new(&vertices, MeshDrawMode::Triangles);
        let mut transform = Transform::IDENTITY;
        transform.translate(10.0, 20.0);
        let mut out = Vec::new();
        mesh.write_vertices(&transform, half, &mut out);

        assert_eq!(out[0].color, [0.5, 0.5, 0.0, 0.5]);
        assert_eq!(out[1].position, [11.0, 20.0]);
        assert_eq!(out[1].uv, [0.25, 0.75]);
    }
}
