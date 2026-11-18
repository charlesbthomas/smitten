use std::cell::RefCell;

use smitten_core::backend::{Backend, BufferId, BufferUsage, Vertex};

use super::{Image, Transform};
use crate::{Color, context};

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
    /// Every vertex is a square dot, [`point_size`](crate::graphics::point_size)
    /// units across and colored by its vertex, as LÖVE's `points` mode.
    /// The dots are not turned or stretched by the transform, only moved.
    Points,
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
#[derive(Debug)]
pub struct Mesh {
    vertices: Vec<MeshVertex>,
    mode: MeshDrawMode,
    texture: Option<Image>,
    /// The order to visit the vertices in, if not first to last.
    vertex_map: Option<Vec<u32>>,
    /// The part of the visiting order that is drawn, as start and count.
    draw_range: Option<(usize, usize)>,
    /// The copy of the mesh kept on the GPU, brought up to date when it is
    /// drawn.
    gpu: RefCell<GpuMesh>,
}

impl Clone for Mesh {
    /// The copy gets GPU buffers of its own the first time it is drawn.
    fn clone(&self) -> Self {
        Mesh {
            vertices: self.vertices.clone(),
            mode: self.mode,
            texture: self.texture.clone(),
            vertex_map: self.vertex_map.clone(),
            draw_range: self.draw_range,
            gpu: RefCell::default(),
        }
    }
}

/// A mesh's vertices and triangle indices in GPU buffers.
#[derive(Debug)]
struct GpuMesh {
    vertices: GpuBuffer,
    indices: GpuBuffer,
    /// How many indices the index buffer holds for the current triangles.
    index_count: u32,
    /// Whether the vertices or the triangles changed since the last upload.
    vertices_stale: bool,
    indices_stale: bool,
    /// The frame that last drew from the buffers. A buffer still waiting
    /// to be drawn in this frame is replaced rather than written to, so
    /// the earlier draw keeps what it was given.
    drawn_in: Option<u64>,
}

impl Default for GpuMesh {
    /// Nothing uploaded yet.
    fn default() -> Self {
        GpuMesh {
            vertices: GpuBuffer::default(),
            indices: GpuBuffer::default(),
            index_count: 0,
            vertices_stale: true,
            indices_stale: true,
            drawn_in: None,
        }
    }
}

/// One GPU buffer and how many bytes it has room for.
#[derive(Debug, Default)]
struct GpuBuffer {
    id: Option<BufferId>,
    capacity: usize,
}

impl GpuBuffer {
    /// Puts `bytes` in the buffer, making a new one if it is missing, too
    /// small or `busy`. A buffer it replaces goes in `retired`.
    fn fill(
        &mut self,
        backend: &mut dyn Backend,
        usage: BufferUsage,
        bytes: &[u8],
        busy: bool,
        retired: &mut Vec<BufferId>,
    ) -> Result<(), String> {
        if let Some(id) = self.id
            && !busy
            && bytes.len() <= self.capacity
        {
            return backend.update_buffer(id, 0, bytes);
        }
        // Room to grow, so a mesh that keeps getting longer is not given a
        // new buffer every time.
        let capacity = bytes.len().next_power_of_two();
        let mut padded = Vec::with_capacity(capacity);
        padded.extend_from_slice(bytes);
        padded.resize(capacity, 0);
        let id = backend.create_buffer(usage, &padded)?;
        retired.extend(self.id.replace(id));
        self.capacity = capacity;
        Ok(())
    }
}

impl Drop for GpuMesh {
    fn drop(&mut self) {
        let ids: Vec<BufferId> = [self.vertices.id, self.indices.id]
            .into_iter()
            .flatten()
            .collect();
        if !ids.is_empty() {
            // A frame may still be waiting to draw from them. If the game
            // has already shut down, the backend freed everything.
            context::try_with(|ctx| ctx.gfx.retire_buffers(&ids));
        }
    }
}

/// Where a mesh's geometry is on the GPU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GpuGeometry {
    pub vertices: BufferId,
    pub indices: BufferId,
    pub index_count: u32,
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
            gpu: RefCell::default(),
        }
    }

    /// Notes that the vertices changed, and with them maybe the triangles.
    fn vertices_changed(&mut self, count_changed: bool) {
        let gpu = self.gpu.get_mut();
        gpu.vertices_stale = true;
        gpu.indices_stale |= count_changed;
    }

    /// Notes that which triangles are drawn changed.
    fn triangles_changed(&mut self) {
        self.gpu.get_mut().indices_stale = true;
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
        self.vertices_changed(false);
    }

    /// All the vertices in order.
    pub fn vertices(&self) -> &[MeshVertex] {
        &self.vertices
    }

    /// Replaces all the vertices. The count may change.
    pub fn set_vertices(&mut self, vertices: &[MeshVertex]) {
        let count_changed = self.vertices.len() != vertices.len();
        self.vertices.clear();
        self.vertices.extend_from_slice(vertices);
        self.vertices_changed(count_changed);
    }

    /// Sends changed vertices to the GPU now rather than when the mesh is
    /// next drawn, as LÖVE's `Mesh:flush` does. Drawing does it anyway, so
    /// this is only ever needed to choose when the work happens.
    pub fn flush(&self) {
        if self.mode == MeshDrawMode::Points {
            return;
        }
        context::with(|ctx| {
            let frame = ctx.gfx.frame_number();
            let mut retired = Vec::new();
            if let Err(message) = self.upload(&mut *ctx.backend, frame, &mut retired, false) {
                eprintln!("smitten: could not send a mesh to the GPU: {message}");
            }
            ctx.gfx.retire_buffers(&retired);
        });
    }

    /// How the vertices are joined into triangles.
    pub fn draw_mode(&self) -> MeshDrawMode {
        self.mode
    }

    /// Changes how the vertices are joined into triangles.
    pub fn set_draw_mode(&mut self, mode: MeshDrawMode) {
        self.mode = mode;
        self.triangles_changed();
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
        self.triangles_changed();
    }

    /// The part of the mesh that is drawn, as `(start, count)`.
    pub fn draw_range(&self) -> Option<(usize, usize)> {
        self.draw_range
    }

    /// Draws only `count` vertices from `start`, counted along the vertex
    /// map if there is one. `None` draws everything.
    pub fn set_draw_range(&mut self, range: Option<(usize, usize)>) {
        self.draw_range = range;
        self.triangles_changed();
    }

    /// The indices of the vertices that are drawn, in visiting order:
    /// through the vertex map if there is one, skipping indices with no
    /// vertex, and limited to the draw range.
    fn visited(&self) -> Vec<u32> {
        let count = self.vertices.len() as u32;
        let visited: Vec<u32> = match &self.vertex_map {
            Some(map) => map.iter().copied().filter(|&index| index < count).collect(),
            None => (0..count).collect(),
        };
        match self.draw_range {
            Some((start, count)) => {
                let start = start.min(visited.len());
                let end = start.saturating_add(count).min(visited.len());
                visited[start..end].to_vec()
            }
            None => visited,
        }
    }

    /// The vertex indices of the triangles the mesh is made of, three
    /// each. Empty for [`MeshDrawMode::Points`], which has none.
    fn triangle_indices(&self) -> Vec<u32> {
        let visited = self.visited();
        match self.mode {
            MeshDrawMode::Triangles => {
                let whole = visited.len() - visited.len() % 3;
                visited[..whole].to_vec()
            }
            MeshDrawMode::Fan => visited
                .get(1..)
                .unwrap_or_default()
                .windows(2)
                .flat_map(|pair| [visited[0], pair[0], pair[1]])
                .collect(),
            MeshDrawMode::Strip => visited.windows(3).flatten().copied().collect(),
            MeshDrawMode::Points => Vec::new(),
        }
    }

    /// Brings the GPU copy of the mesh up to date for a draw in `frame`,
    /// or for later with `for_draw` off. Returns where it is, or `None`
    /// if there are no triangles to draw. Buffers it replaces go in
    /// `retired`.
    pub(crate) fn upload(
        &self,
        backend: &mut dyn Backend,
        frame: u64,
        retired: &mut Vec<BufferId>,
        for_draw: bool,
    ) -> Result<Option<GpuGeometry>, String> {
        let mut gpu = self.gpu.borrow_mut();
        let gpu = &mut *gpu;
        let busy = gpu.drawn_in == Some(frame);
        if gpu.indices_stale {
            let indices = self.triangle_indices();
            if !indices.is_empty() {
                let bytes: Vec<u8> = indices
                    .iter()
                    .flat_map(|index| index.to_le_bytes())
                    .collect();
                gpu.indices
                    .fill(backend, BufferUsage::Index, &bytes, busy, retired)?;
            }
            gpu.index_count = indices.len() as u32;
            gpu.indices_stale = false;
        }
        if gpu.vertices_stale && !self.vertices.is_empty() {
            let bytes: Vec<u8> = self
                .vertices
                .iter()
                .flat_map(|vertex| {
                    let MeshVertex { x, y, u, v, color } = *vertex;
                    [x, y, u, v, color.r, color.g, color.b, color.a]
                })
                .flat_map(f32::to_le_bytes)
                .collect();
            gpu.vertices
                .fill(backend, BufferUsage::Vertex, &bytes, busy, retired)?;
            gpu.vertices_stale = false;
        }
        let (Some(vertices), Some(indices)) = (gpu.vertices.id, gpu.indices.id) else {
            return Ok(None);
        };
        if gpu.index_count == 0 {
            return Ok(None);
        }
        if for_draw {
            gpu.drawn_in = Some(frame);
        }
        Ok(Some(GpuGeometry {
            vertices,
            indices,
            index_count: gpu.index_count,
        }))
    }

    /// The mesh as a list of separate triangles, placed by `transform`
    /// and tinted by `tint`.
    ///
    /// `point_size` is how wide a dot of [`MeshDrawMode::Points`] is, in
    /// pixels of the target.
    pub(crate) fn write_vertices(
        &self,
        transform: &Transform,
        tint: Color,
        point_size: f32,
        out: &mut Vec<Vertex>,
    ) {
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
            MeshDrawMode::Triangles | MeshDrawMode::Fan | MeshDrawMode::Strip => {
                out.extend(
                    self.triangle_indices()
                        .into_iter()
                        .map(|index| place(&self.vertices[index as usize])),
                );
            }
            MeshDrawMode::Points => {
                let half = point_size / 2.0;
                for index in self.visited() {
                    let center = place(&self.vertices[index as usize]);
                    let [x, y] = center.position;
                    let corner = |dx: f32, dy: f32| Vertex {
                        position: [x + dx, y + dy],
                        ..center
                    };
                    out.extend([
                        corner(-half, -half),
                        corner(half, -half),
                        corner(half, half),
                        corner(-half, -half),
                        corner(half, half),
                        corner(-half, half),
                    ]);
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
        mesh.write_vertices(&Transform::IDENTITY, Color::WHITE, 1.0, &mut out);
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
    fn points_are_squares_around_each_moved_vertex() {
        let mesh = Mesh::new(&corners(2), MeshDrawMode::Points);
        let mut transform = Transform::IDENTITY;
        transform.translate(10.0, 0.0);
        transform.scale(5.0, 5.0);
        let mut out = Vec::new();
        mesh.write_vertices(&transform, Color::WHITE, 4.0, &mut out);
        assert_eq!(out.len(), 12);
        let xs: Vec<f32> = out[..6].iter().map(|vertex| vertex.position[0]).collect();
        // The dot is not scaled with the transform.
        assert_eq!(xs, [8.0, 12.0, 12.0, 8.0, 12.0, 8.0]);
        assert_eq!(out[6].position, [13.0, -2.0]);
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
        mesh.write_vertices(&transform, half, 1.0, &mut out);

        assert_eq!(out[0].color, [0.5, 0.5, 0.0, 0.5]);
        assert_eq!(out[1].position, [11.0, 20.0]);
        assert_eq!(out[1].uv, [0.25, 0.75]);
    }
}
