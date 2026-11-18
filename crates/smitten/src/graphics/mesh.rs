use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use smitten_core::backend::{
    Backend, BufferId, BufferUsage, IndexDataType, MAX_VERTEX_ATTRIBUTES, MAX_VERTEX_BUFFERS,
    VERTEX_COLOR, VERTEX_POSITION, VERTEX_TEX_COORD, Vertex, VertexBufferLayout, VertexDataType,
    VertexLayout, VertexLayoutAttribute, VertexLayoutId, VertexStep,
};

use super::{Image, Transform};
use crate::{Color, context};

/// One named value of every vertex in a mesh's vertex format, as an entry
/// of LÖVE's vertex format table.
///
/// The built-in shader reads three by name, as LÖVE's does:
/// `VertexPosition` (x and y, and z for depth), `VertexTexCoord` (u and v)
/// and `VertexColor`. Other names are for custom vertex shaders.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VertexAttribute {
    pub name: String,
    pub data_type: VertexDataType,
    /// How many values it has, 1 to 4.
    pub components: u8,
}

impl VertexAttribute {
    pub fn new(name: impl Into<String>, data_type: VertexDataType, components: u8) -> Self {
        VertexAttribute {
            name: name.into(),
            data_type,
            components,
        }
    }
}

/// Why a mesh could not be made or changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshError(String);

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MeshError {}

fn error(message: impl Into<String>) -> MeshError {
    MeshError(message.into())
}

/// A vertex format with where each attribute sits in a vertex.
#[derive(Debug, Clone, PartialEq)]
struct Format {
    attributes: Vec<VertexAttribute>,
    offsets: Vec<u32>,
    /// How many bytes one vertex takes.
    stride: u32,
    /// The index of the position, texture coordinate and color attributes.
    position: Option<usize>,
    tex_coord: Option<usize>,
    color: Option<usize>,
}

impl Format {
    fn new(attributes: &[VertexAttribute]) -> Result<Format, MeshError> {
        if attributes.is_empty() {
            return Err(error("a vertex format needs at least one attribute"));
        }
        if attributes.len() > MAX_VERTEX_ATTRIBUTES {
            return Err(error(format!(
                "a vertex format can have at most {MAX_VERTEX_ATTRIBUTES} attributes"
            )));
        }
        let mut offsets = Vec::with_capacity(attributes.len());
        let mut stride = 0;
        for (index, attribute) in attributes.iter().enumerate() {
            let name = &attribute.name;
            if name.is_empty() {
                return Err(error("a vertex attribute needs a name"));
            }
            if attributes[..index].iter().any(|other| other.name == *name) {
                return Err(error(format!("vertex attribute `{name}` is named twice")));
            }
            if !(1..=4).contains(&attribute.components) {
                return Err(error(format!(
                    "vertex attribute `{name}` must have 1 to 4 components, not {}",
                    attribute.components
                )));
            }
            offsets.push(stride);
            stride += attribute.data_type.size(attribute.components);
        }
        let find = |name: &str| {
            attributes
                .iter()
                .position(|attribute| attribute.name == name)
        };
        Ok(Format {
            position: find(VERTEX_POSITION),
            tex_coord: find(VERTEX_TEX_COORD),
            color: find(VERTEX_COLOR),
            attributes: attributes.to_vec(),
            offsets,
            stride,
        })
    }

    /// The format of [`Mesh::new`]: a position and a texture coordinate
    /// of two floats each and a color of four, laid out as [`Vertex`].
    fn standard() -> Format {
        Format::new(&[
            VertexAttribute::new(VERTEX_POSITION, VertexDataType::Float, 2),
            VertexAttribute::new(VERTEX_TEX_COORD, VertexDataType::Float, 2),
            VertexAttribute::new(VERTEX_COLOR, VertexDataType::Float, 4),
        ])
        .expect("the standard format is valid")
    }

    /// How many values a whole vertex has.
    fn value_count(&self) -> usize {
        self.attributes
            .iter()
            .map(|attribute| usize::from(attribute.components))
            .sum()
    }

    /// The values of attribute `index` of `vertex`, one vertex's bytes.
    fn read(&self, vertex: &[u8], index: usize) -> Vec<f32> {
        let attribute = &self.attributes[index];
        let start = self.offsets[index] as usize;
        (0..usize::from(attribute.components))
            .map(|component| match attribute.data_type {
                VertexDataType::Float => {
                    let at = start + component * 4;
                    f32::from_le_bytes(vertex[at..at + 4].try_into().unwrap())
                }
                VertexDataType::UNorm8 => f32::from(vertex[start + component]) / 255.0,
                VertexDataType::UNorm16 => {
                    let at = start + component * 2;
                    f32::from(u16::from_le_bytes(vertex[at..at + 2].try_into().unwrap())) / 65535.0
                }
            })
            .collect()
    }

    /// Stores `values` as attribute `index` of `vertex`. Missing values
    /// are left as they were.
    fn write(&self, vertex: &mut [u8], index: usize, values: &[f32]) {
        let attribute = &self.attributes[index];
        let start = self.offsets[index] as usize;
        for (component, &value) in values
            .iter()
            .take(usize::from(attribute.components))
            .enumerate()
        {
            match attribute.data_type {
                VertexDataType::Float => {
                    let at = start + component * 4;
                    vertex[at..at + 4].copy_from_slice(&value.to_le_bytes());
                }
                VertexDataType::UNorm8 => {
                    vertex[start + component] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                VertexDataType::UNorm16 => {
                    let at = start + component * 2;
                    let value = (value.clamp(0.0, 1.0) * 65535.0).round() as u16;
                    vertex[at..at + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
}

/// A mesh's vertex data, which other meshes can attach attributes from,
/// and its copy on the GPU.
#[derive(Debug)]
struct VertexStore {
    format: Format,
    /// Every vertex's bytes, one after the other.
    data: Vec<u8>,
    buffer: GpuBuffer,
    /// Whether `data` changed since it was uploaded.
    stale: bool,
    /// The frame that last drew from the buffer. A buffer still waiting to
    /// be drawn in this frame is replaced rather than written to, so the
    /// earlier draw keeps what it was given.
    drawn_in: Option<u64>,
}

impl VertexStore {
    fn new(format: Format, data: Vec<u8>) -> VertexStore {
        VertexStore {
            format,
            data,
            buffer: GpuBuffer::default(),
            stale: true,
            drawn_in: None,
        }
    }

    fn count(&self) -> usize {
        self.data.len() / self.format.stride as usize
    }

    fn vertex(&self, index: usize) -> &[u8] {
        let stride = self.format.stride as usize;
        &self.data[index * stride..(index + 1) * stride]
    }

    fn vertex_mut(&mut self, index: usize) -> &mut [u8] {
        let stride = self.format.stride as usize;
        self.stale = true;
        &mut self.data[index * stride..(index + 1) * stride]
    }

    /// The standard view of vertex `index`: the attributes the built-in
    /// shader reads, with LÖVE's defaults for missing ones.
    fn mesh_vertex(&self, index: usize) -> MeshVertex {
        let vertex = self.vertex(index);
        let read = |attribute: Option<usize>, defaults: [f32; 4]| {
            let mut values = defaults;
            if let Some(attribute) = attribute {
                for (value, read) in values.iter_mut().zip(self.format.read(vertex, attribute)) {
                    *value = read;
                }
            }
            values
        };
        let [x, y, ..] = read(self.format.position, [0.0, 0.0, 0.0, 1.0]);
        let [u, v, ..] = read(self.format.tex_coord, [0.0; 4]);
        let [r, g, b, a] = read(self.format.color, [1.0; 4]);
        MeshVertex {
            x,
            y,
            u,
            v,
            color: Color::rgba(r, g, b, a),
        }
    }

    /// Stores the attributes of `vertex` that the format has.
    fn set_mesh_vertex(&mut self, index: usize, vertex: MeshVertex) {
        let format = self.format.clone();
        let bytes = self.vertex_mut(index);
        let MeshVertex { x, y, u, v, color } = vertex;
        if let Some(attribute) = format.position {
            format.write(bytes, attribute, &[x, y]);
        }
        if let Some(attribute) = format.tex_coord {
            format.write(bytes, attribute, &[u, v]);
        }
        if let Some(attribute) = format.color {
            format.write(bytes, attribute, &[color.r, color.g, color.b, color.a]);
        }
    }

    /// Brings the GPU copy up to date for a draw in `frame`, or for later
    /// without `for_draw`. Returns the buffer, or `None` with no vertices.
    fn upload(
        &mut self,
        backend: &mut dyn Backend,
        frame: u64,
        retired: &mut Vec<BufferId>,
        for_draw: bool,
    ) -> Result<Option<BufferId>, String> {
        if self.data.is_empty() {
            return Ok(None);
        }
        if self.stale || self.buffer.id.is_none() {
            let busy = self.drawn_in == Some(frame);
            self.buffer
                .fill(backend, BufferUsage::Vertex, &self.data, busy, retired)?;
            self.stale = false;
        }
        if for_draw {
            self.drawn_in = Some(frame);
        }
        Ok(self.buffer.id)
    }
}

impl Drop for VertexStore {
    fn drop(&mut self) {
        retire(self.buffer.id);
    }
}

/// Frees a buffer once no queued frame can draw from it. If the game has
/// already shut down, the backend freed everything.
fn retire(buffer: Option<BufferId>) {
    if let Some(buffer) = buffer {
        context::try_with(|ctx| ctx.gfx.retire_buffers(&[buffer]));
    }
}

/// An attribute taken from another mesh's vertices.
#[derive(Debug, Clone)]
struct Attachment {
    /// The attribute's name in the other mesh.
    name: String,
    source: Rc<RefCell<VertexStore>>,
    step: VertexStep,
    /// The name shaders know it by when this mesh is drawn.
    attach_name: String,
}

/// A mesh's triangle indices on the GPU, and the layout it was last drawn
/// with.
#[derive(Debug)]
struct GpuIndices {
    buffer: GpuBuffer,
    /// How many indices the buffer holds for the current triangles, and
    /// their type.
    count: u32,
    index_type: IndexDataType,
    stale: bool,
    drawn_in: Option<u64>,
    layout: Option<(VertexLayout, VertexLayoutId)>,
}

impl Default for GpuIndices {
    /// Nothing uploaded yet.
    fn default() -> Self {
        GpuIndices {
            buffer: GpuBuffer::default(),
            count: 0,
            index_type: IndexDataType::U32,
            stale: true,
            drawn_in: None,
            layout: None,
        }
    }
}

impl Drop for GpuIndices {
    fn drop(&mut self) {
        retire(self.buffer.id);
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

/// Where a mesh's geometry is on the GPU, for one draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GpuGeometry {
    pub layout: VertexLayoutId,
    pub buffers: [Option<BufferId>; MAX_VERTEX_BUFFERS],
    pub vertex_count: u32,
    pub indices: BufferId,
    pub index_type: IndexDataType,
    pub index_count: u32,
}

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
/// A mesh lives on the GPU: its vertices are sent when they change, not
/// every frame, and the GPU moves them into place. Its vertices can have
/// a format of their own, made with [`Mesh::with_format`], with extra
/// attributes for custom vertex shaders.
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
pub struct Mesh {
    /// Shared with meshes that attach attributes from this one.
    store: Rc<RefCell<VertexStore>>,
    mode: MeshDrawMode,
    texture: Option<Image>,
    /// The order to visit the vertices in, if not first to last.
    vertex_map: Option<Vec<u32>>,
    /// The index type asked for with the vertex map, if any.
    index_type: Option<IndexDataType>,
    /// The part of the visiting order that is drawn, as start and count.
    draw_range: Option<(usize, usize)>,
    attachments: Vec<Attachment>,
    /// Attribute names turned off with `set_attribute_enabled`.
    disabled: Vec<String>,
    gpu: RefCell<GpuIndices>,
}

impl Clone for Mesh {
    /// The copy has vertices of its own, and attaches the same attributes
    /// from the same meshes. It gets GPU buffers the first time it is
    /// drawn.
    fn clone(&self) -> Self {
        let store = self.store.borrow();
        Mesh {
            store: Rc::new(RefCell::new(VertexStore::new(
                store.format.clone(),
                store.data.clone(),
            ))),
            mode: self.mode,
            texture: self.texture.clone(),
            vertex_map: self.vertex_map.clone(),
            index_type: self.index_type,
            draw_range: self.draw_range,
            attachments: self.attachments.clone(),
            disabled: self.disabled.clone(),
            gpu: RefCell::default(),
        }
    }
}

impl fmt::Debug for Mesh {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let store = self.store.borrow();
        f.debug_struct("Mesh")
            .field("vertex_count", &store.count())
            .field("format", &store.format.attributes)
            .field("mode", &self.mode)
            .field("texture", &self.texture)
            .field("vertex_map", &self.vertex_map)
            .field("draw_range", &self.draw_range)
            .finish_non_exhaustive()
    }
}

impl Mesh {
    /// Creates a mesh from its vertices, in the standard format: a
    /// position and texture coordinate of two floats each, and a color.
    pub fn new(vertices: &[MeshVertex], mode: MeshDrawMode) -> Mesh {
        let format = Format::standard();
        let data = vec![0; vertices.len() * format.stride as usize];
        let mut store = VertexStore::new(format, data);
        for (index, &vertex) in vertices.iter().enumerate() {
            store.set_mesh_vertex(index, vertex);
        }
        Mesh::from_store(store, mode)
    }

    /// Creates a mesh whose vertices have `format`, as LÖVE's
    /// `newMesh(vertexformat, vertices)`. Each vertex is given as all its
    /// values in the format's order, bytes and 16-bit values as 0.0 to
    /// 1.0, and must have as many as the format has components.
    ///
    /// ```no_run
    /// # use smitten::graphics::{Mesh, MeshDrawMode, VertexAttribute, VertexDataType};
    /// let format = [
    ///     VertexAttribute::new("VertexPosition", VertexDataType::Float, 2),
    ///     VertexAttribute::new("VertexColor", VertexDataType::UNorm8, 4),
    ///     VertexAttribute::new("Weight", VertexDataType::Float, 1),
    /// ];
    /// let mesh = Mesh::with_format(
    ///     &format,
    ///     &[
    ///         &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.5],
    ///         &[100.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0],
    ///         &[0.0, 100.0, 0.0, 0.0, 1.0, 1.0, 0.0],
    ///     ],
    ///     MeshDrawMode::Triangles,
    /// )
    /// .unwrap();
    /// ```
    pub fn with_format(
        format: &[VertexAttribute],
        vertices: &[&[f32]],
        mode: MeshDrawMode,
    ) -> Result<Mesh, MeshError> {
        let mut mesh = Mesh::with_format_and_count(format, vertices.len(), mode)?;
        for (index, values) in vertices.iter().enumerate() {
            mesh.set_vertex_values(index, values)?;
        }
        Ok(mesh)
    }

    /// Creates a mesh with `count` vertices in `format`, every value 0, as
    /// LÖVE's `newMesh(vertexformat, vertexcount)`.
    pub fn with_format_and_count(
        format: &[VertexAttribute],
        count: usize,
        mode: MeshDrawMode,
    ) -> Result<Mesh, MeshError> {
        let format = Format::new(format)?;
        let data = vec![0; count * format.stride as usize];
        Ok(Mesh::from_store(VertexStore::new(format, data), mode))
    }

    fn from_store(store: VertexStore, mode: MeshDrawMode) -> Mesh {
        Mesh {
            store: Rc::new(RefCell::new(store)),
            mode,
            texture: None,
            vertex_map: None,
            index_type: None,
            draw_range: None,
            attachments: Vec::new(),
            disabled: Vec::new(),
            gpu: RefCell::default(),
        }
    }

    /// Notes that which triangles are drawn may have changed.
    fn triangles_changed(&mut self) {
        self.gpu.get_mut().stale = true;
    }

    /// The format of the mesh's vertices, as LÖVE's `getVertexFormat`.
    pub fn vertex_format(&self) -> Vec<VertexAttribute> {
        self.store.borrow().format.attributes.clone()
    }

    /// How many vertices the mesh has.
    pub fn vertex_count(&self) -> usize {
        self.store.borrow().count()
    }

    /// The vertex at `index`, counting from zero. For a mesh with its own
    /// format, the position, texture coordinate and color it has, with
    /// the defaults the built-in shader uses for those it lacks.
    pub fn vertex(&self, index: usize) -> Option<MeshVertex> {
        let store = self.store.borrow();
        (index < store.count()).then(|| store.mesh_vertex(index))
    }

    /// Replaces the vertex at `index`. Panics if there is none. For a mesh
    /// with its own format, only the position, texture coordinate and
    /// color it has are set.
    pub fn set_vertex(&mut self, index: usize, vertex: MeshVertex) {
        let mut store = self.store.borrow_mut();
        assert!(
            index < store.count(),
            "vertex {index} is past the end of a mesh with {} vertices",
            store.count()
        );
        store.set_mesh_vertex(index, vertex);
    }

    /// All the vertices in order, as [`Mesh::vertex`] gives them.
    pub fn vertices(&self) -> Vec<MeshVertex> {
        let store = self.store.borrow();
        (0..store.count())
            .map(|index| store.mesh_vertex(index))
            .collect()
    }

    /// Replaces all the vertices. The count may change. For a mesh with
    /// its own format, values other than the position, texture coordinate
    /// and color are kept for vertices that were there, and 0 for new ones.
    pub fn set_vertices(&mut self, vertices: &[MeshVertex]) {
        let count_changed = {
            let mut store = self.store.borrow_mut();
            let count_changed = store.count() != vertices.len();
            let length = vertices.len() * store.format.stride as usize;
            store.data.resize(length, 0);
            store.stale = true;
            for (index, &vertex) in vertices.iter().enumerate() {
                store.set_mesh_vertex(index, vertex);
            }
            count_changed
        };
        if count_changed {
            self.triangles_changed();
        }
    }

    /// All the values of the vertex at `index`, in the format's order, as
    /// LÖVE's `getVertex` gives them.
    pub fn vertex_values(&self, index: usize) -> Option<Vec<f32>> {
        let store = self.store.borrow();
        (index < store.count()).then(|| {
            (0..store.format.attributes.len())
                .flat_map(|attribute| store.format.read(store.vertex(index), attribute))
                .collect()
        })
    }

    /// Replaces all the values of the vertex at `index`, given in the
    /// format's order, as LÖVE's `setVertex`. There must be as many as
    /// the format has components.
    pub fn set_vertex_values(&mut self, index: usize, values: &[f32]) -> Result<(), MeshError> {
        let mut store = self.store.borrow_mut();
        if index >= store.count() {
            return Err(error(format!(
                "vertex {index} is past the end of a mesh with {} vertices",
                store.count()
            )));
        }
        let expected = store.format.value_count();
        if values.len() != expected {
            return Err(error(format!(
                "a vertex of this mesh has {expected} values, not {}",
                values.len()
            )));
        }
        let format = store.format.clone();
        let bytes = store.vertex_mut(index);
        let mut rest = values;
        for (attribute, entry) in format.attributes.iter().enumerate() {
            let (these, others) = rest.split_at(usize::from(entry.components));
            format.write(bytes, attribute, these);
            rest = others;
        }
        Ok(())
    }

    /// The values of one attribute of one vertex, both counted from zero,
    /// as LÖVE's `getVertexAttribute`.
    pub fn vertex_attribute(&self, vertex: usize, attribute: usize) -> Option<Vec<f32>> {
        let store = self.store.borrow();
        (vertex < store.count() && attribute < store.format.attributes.len())
            .then(|| store.format.read(store.vertex(vertex), attribute))
    }

    /// Replaces the values of one attribute of one vertex, as LÖVE's
    /// `setVertexAttribute`. There must be as many as it has components.
    pub fn set_vertex_attribute(
        &mut self,
        vertex: usize,
        attribute: usize,
        values: &[f32],
    ) -> Result<(), MeshError> {
        let mut store = self.store.borrow_mut();
        if vertex >= store.count() {
            return Err(error(format!(
                "vertex {vertex} is past the end of a mesh with {} vertices",
                store.count()
            )));
        }
        let Some(entry) = store.format.attributes.get(attribute) else {
            return Err(error(format!(
                "the mesh's vertices have no attribute {attribute}"
            )));
        };
        if values.len() != usize::from(entry.components) {
            return Err(error(format!(
                "attribute `{}` has {} values, not {}",
                entry.name,
                entry.components,
                values.len()
            )));
        }
        let format = store.format.clone();
        format.write(store.vertex_mut(vertex), attribute, values);
        Ok(())
    }

    /// Draws attribute `name` of `mesh`'s vertices as part of this mesh,
    /// one value per vertex, as LÖVE's `attachAttribute(name, mesh)`. It
    /// replaces an attribute of the same name this mesh has, and follows
    /// later changes to `mesh`. `mesh` must have at least as many vertices
    /// as this one when it is drawn.
    pub fn attach_attribute(&mut self, name: &str, mesh: &Mesh) -> Result<(), MeshError> {
        self.attach_attribute_ex(name, mesh, VertexStep::PerVertex, name)
    }

    /// As [`attach_attribute`](Self::attach_attribute), with LÖVE's other
    /// arguments: `step` says whether the values advance with each vertex
    /// or with each instance drawn, and `attach_name` is the name shaders
    /// know the attribute by here. Attaching again under the same
    /// `attach_name` replaces it.
    pub fn attach_attribute_ex(
        &mut self,
        name: &str,
        mesh: &Mesh,
        step: VertexStep,
        attach_name: &str,
    ) -> Result<(), MeshError> {
        let has_it = mesh
            .store
            .borrow()
            .format
            .attributes
            .iter()
            .any(|attribute| attribute.name == name);
        if !has_it {
            return Err(error(format!(
                "the mesh being attached has no attribute `{name}`"
            )));
        }
        let replacing = self
            .attachments
            .iter()
            .any(|attachment| attachment.attach_name == attach_name);
        if !replacing && self.attachments.len() + 1 >= MAX_VERTEX_BUFFERS {
            return Err(error(format!(
                "a mesh can have at most {} attached attributes",
                MAX_VERTEX_BUFFERS - 1
            )));
        }
        self.attachments
            .retain(|attachment| attachment.attach_name != attach_name);
        self.attachments.push(Attachment {
            name: name.to_string(),
            source: Rc::clone(&mesh.store),
            step,
            attach_name: attach_name.to_string(),
        });
        Ok(())
    }

    /// Stops drawing an attribute attached under `attach_name`, as LÖVE's
    /// `detachAttribute`. Returns whether there was one.
    pub fn detach_attribute(&mut self, attach_name: &str) -> bool {
        let before = self.attachments.len();
        self.attachments
            .retain(|attachment| attachment.attach_name != attach_name);
        self.attachments.len() != before
    }

    /// Turns an attribute of the mesh, its own or attached, off or back
    /// on, as LÖVE's `setAttributeEnabled`. A shader reads an attribute
    /// that is off as its default: (0, 0, 0, 1) for the position, (0, 0)
    /// for the texture coordinate and white for the color.
    pub fn set_attribute_enabled(&mut self, name: &str, enabled: bool) {
        self.disabled.retain(|disabled| disabled != name);
        if !enabled {
            self.disabled.push(name.to_string());
        }
    }

    /// Whether an attribute is on, as LÖVE's `isAttributeEnabled`. Names
    /// the mesh does not have count as on.
    pub fn is_attribute_enabled(&self, name: &str) -> bool {
        !self.disabled.iter().any(|disabled| disabled == name)
    }

    /// Sends changed vertices to the GPU now rather than when the mesh is
    /// next drawn, as LÖVE's `Mesh:flush` does. Drawing does it anyway, so
    /// this is only ever needed to choose when the work happens.
    pub fn flush(&self) {
        context::with(|ctx| {
            let frame = ctx.gfx.frame_number();
            let mut retired = Vec::new();
            let uploaded =
                self.store
                    .borrow_mut()
                    .upload(&mut *ctx.backend, frame, &mut retired, false);
            if let Err(message) = uploaded {
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
        self.index_type = None;
        self.triangles_changed();
    }

    /// As [`set_vertex_map`](Self::set_vertex_map), storing the indices on
    /// the GPU as `data_type`, as LÖVE's `setVertexMap(map, datatype)`.
    /// With [`IndexDataType::U16`], indices above 65,535 are skipped.
    pub fn set_vertex_map_ex(&mut self, map: Option<&[u32]>, data_type: IndexDataType) {
        self.set_vertex_map(map);
        self.index_type = Some(data_type);
    }

    /// How the mesh's indices are stored on the GPU: as asked for with
    /// [`set_vertex_map_ex`](Self::set_vertex_map_ex), or otherwise 16-bit
    /// when the mesh has at most 65,536 vertices, as in LÖVE.
    pub fn index_data_type(&self) -> IndexDataType {
        self.index_type.unwrap_or(if self.vertex_count() <= 65_536 {
            IndexDataType::U16
        } else {
            IndexDataType::U32
        })
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
    /// vertex or too large for the index type, and limited to the draw
    /// range.
    fn visited(&self) -> Vec<u32> {
        let count = self.vertex_count() as u32;
        let limit = match self.index_data_type() {
            IndexDataType::U16 => count.min(65_536),
            IndexDataType::U32 => count,
        };
        let visited: Vec<u32> = match &self.vertex_map {
            Some(map) => map.iter().copied().filter(|&index| index < limit).collect(),
            None => (0..limit).collect(),
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

    /// The attachments whose attribute is on, which are the ones drawn.
    fn enabled_attachments(&self) -> impl Iterator<Item = &Attachment> {
        self.attachments
            .iter()
            .filter(|attachment| self.is_attribute_enabled(&attachment.attach_name))
    }

    /// How the vertex buffers of a draw are laid out: this mesh's own
    /// vertices in the first, then one for each attachment. Attributes
    /// that are off, or replaced by an attachment, are left out.
    fn draw_layout(&self) -> VertexLayout {
        let attached = |name: &str| {
            self.attachments
                .iter()
                .any(|attachment| attachment.attach_name == name)
        };
        let store = self.store.borrow();
        let format = &store.format;
        let mut layout = VertexLayout {
            buffers: vec![VertexBufferLayout {
                stride: format.stride,
                step: VertexStep::PerVertex,
            }],
            attributes: format
                .attributes
                .iter()
                .zip(&format.offsets)
                .filter(|(attribute, _)| {
                    self.is_attribute_enabled(&attribute.name) && !attached(&attribute.name)
                })
                .map(|(attribute, &offset)| VertexLayoutAttribute {
                    name: attribute.name.clone(),
                    buffer: 0,
                    offset,
                    data_type: attribute.data_type,
                    components: attribute.components,
                })
                .collect(),
        };
        for attachment in self.enabled_attachments() {
            let slot = layout.buffers.len() as u32;
            let source = attachment.source.borrow();
            let format = &source.format;
            layout.buffers.push(VertexBufferLayout {
                stride: format.stride,
                step: attachment.step,
            });
            let index = format
                .attributes
                .iter()
                .position(|attribute| attribute.name == attachment.name)
                .expect("checked when attached");
            let attribute = &format.attributes[index];
            layout.attributes.push(VertexLayoutAttribute {
                name: attachment.attach_name.clone(),
                buffer: slot,
                offset: format.offsets[index],
                data_type: attribute.data_type,
                components: attribute.components,
            });
        }
        layout
    }

    /// Brings the GPU copy of the mesh, and of the meshes it attaches
    /// attributes from, up to date for a draw in `frame`. Returns where
    /// it all is, or `None` if there are no triangles to draw. Buffers it
    /// replaces go in `retired`.
    ///
    /// Panics if an attached mesh has too few vertices, as LÖVE errors.
    pub(crate) fn upload(
        &self,
        backend: &mut dyn Backend,
        frame: u64,
        retired: &mut Vec<BufferId>,
        instances: u32,
    ) -> Result<Option<GpuGeometry>, String> {
        let vertex_count = self.vertex_count();
        for attachment in self.enabled_attachments() {
            let source_count = attachment.source.borrow().count();
            if attachment.step == VertexStep::PerInstance && source_count < instances as usize {
                panic!(
                    "attribute `{}` is attached per instance from a mesh with {source_count} \
                     vertices, fewer than the {instances} instances drawn",
                    attachment.attach_name
                );
            }
            if attachment.step == VertexStep::PerVertex && source_count < vertex_count {
                panic!(
                    "attribute `{}` is attached from a mesh with {source_count} vertices, \
                     fewer than the {vertex_count} of the mesh drawn",
                    attachment.attach_name
                );
            }
        }
        let mut gpu = self.gpu.borrow_mut();
        let gpu = &mut *gpu;
        if gpu.stale {
            let index_type = self.index_data_type();
            let indices = self.triangle_indices();
            if !indices.is_empty() {
                let bytes: Vec<u8> = match index_type {
                    IndexDataType::U16 => indices
                        .iter()
                        .flat_map(|&index| (index as u16).to_le_bytes())
                        .collect(),
                    IndexDataType::U32 => indices
                        .iter()
                        .flat_map(|index| index.to_le_bytes())
                        .collect(),
                };
                let busy = gpu.drawn_in == Some(frame);
                gpu.buffer
                    .fill(backend, BufferUsage::Index, &bytes, busy, retired)?;
            }
            gpu.count = indices.len() as u32;
            gpu.index_type = index_type;
            gpu.stale = false;
        }
        let Some(indices) = gpu.buffer.id.filter(|_| gpu.count > 0) else {
            return Ok(None);
        };
        let mut buffers = [None; MAX_VERTEX_BUFFERS];
        let own = self
            .store
            .borrow_mut()
            .upload(backend, frame, retired, true)?;
        let Some(own) = own else {
            return Ok(None);
        };
        buffers[0] = Some(own);
        for (slot, attachment) in self.enabled_attachments().enumerate() {
            let source = attachment
                .source
                .borrow_mut()
                .upload(backend, frame, retired, true)?;
            // Only a mesh with no vertices has no buffer, and one with too
            // few is ruled out above, so this is an instance buffer for
            // instances there are none of.
            let Some(source) = source else {
                return Ok(None);
            };
            buffers[slot + 1] = Some(source);
        }
        gpu.drawn_in = Some(frame);
        let layout = self.draw_layout();
        let layout_id = match &gpu.layout {
            Some((known, id)) if *known == layout => *id,
            _ => {
                let id = backend.create_vertex_layout(&layout)?;
                gpu.layout = Some((layout, id));
                id
            }
        };
        Ok(Some(GpuGeometry {
            layout: layout_id,
            buffers,
            vertex_count: vertex_count as u32,
            indices,
            index_type: gpu.index_type,
            index_count: gpu.count,
        }))
    }

    /// The mesh as a list of separate triangles, placed by `transform`
    /// and tinted by `tint`, for drawing without the GPU's help. Attached
    /// attributes are not drawn this way.
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
        let store = self.store.borrow();
        let place = |index: u32| {
            let vertex = store.mesh_vertex(index as usize);
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
                out.extend(self.triangle_indices().into_iter().map(place));
            }
            MeshDrawMode::Points => {
                let half = point_size / 2.0;
                for index in self.visited() {
                    let center = place(index);
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

    fn custom_format() -> Vec<VertexAttribute> {
        vec![
            VertexAttribute::new(VERTEX_POSITION, VertexDataType::Float, 3),
            VertexAttribute::new(VERTEX_COLOR, VertexDataType::UNorm8, 4),
            VertexAttribute::new("Weight", VertexDataType::UNorm16, 1),
        ]
    }

    #[test]
    fn a_custom_format_is_laid_out_with_bytes_padded_to_four() {
        let format = Format::new(&custom_format()).unwrap();
        assert_eq!(format.offsets, [0, 12, 16]);
        assert_eq!(format.stride, 20);
        assert_eq!(format.value_count(), 8);
        assert_eq!(Format::standard().stride, size_of::<Vertex>() as u32);
    }

    #[test]
    fn bad_formats_are_errors() {
        let float =
            |name: &str, components| VertexAttribute::new(name, VertexDataType::Float, components);
        assert!(Format::new(&[]).is_err());
        assert!(Format::new(&[float("A", 0)]).is_err());
        assert!(Format::new(&[float("A", 5)]).is_err());
        assert!(Format::new(&[float("", 1)]).is_err());
        assert!(Format::new(&[float("A", 1), float("A", 2)]).is_err());
    }

    #[test]
    fn values_come_back_as_they_were_stored() {
        let mut mesh = Mesh::with_format(
            &custom_format(),
            &[&[1.0, 2.0, 3.0, 1.0, 0.0, 0.5, 1.0, 0.25]],
            MeshDrawMode::Triangles,
        )
        .unwrap();
        let values = mesh.vertex_values(0).unwrap();
        assert_eq!(&values[..3], [1.0, 2.0, 3.0]);
        // Bytes keep 1/255 steps and 16-bit values 1/65535 steps.
        assert!((values[5] - 128.0 / 255.0).abs() < 1e-6);
        assert!((values[7] - 0.25).abs() < 1e-4);

        mesh.set_vertex_attribute(0, 2, &[0.75]).unwrap();
        assert!((mesh.vertex_attribute(0, 2).unwrap()[0] - 0.75).abs() < 1e-4);
        assert!(mesh.set_vertex_attribute(0, 2, &[0.5, 0.5]).is_err());
        assert!(mesh.set_vertex_attribute(1, 0, &[0.0; 3]).is_err());
        assert!(mesh.set_vertex_attribute(0, 3, &[0.0]).is_err());
        assert!(mesh.set_vertex_values(0, &[0.0; 7]).is_err());
        assert_eq!(mesh.vertex_attribute(0, 3), None);
    }

    #[test]
    fn a_custom_format_reads_and_writes_the_standard_view() {
        let mut mesh =
            Mesh::with_format_and_count(&custom_format(), 2, MeshDrawMode::Triangles).unwrap();
        let mut vertex = MeshVertex::colored(4.0, 5.0, Color::rgba(1.0, 0.0, 1.0, 1.0));
        vertex.u = 0.5;
        mesh.set_vertex(1, vertex);
        let back = mesh.vertex(1).unwrap();
        assert_eq!((back.x, back.y), (4.0, 5.0));
        // No texture coordinate in the format, so it reads as the default.
        assert_eq!((back.u, back.v), (0.0, 0.0));
        assert_eq!(back.color, Color::rgba(1.0, 0.0, 1.0, 1.0));
        // Setting the standard view leaves the rest alone.
        assert_eq!(mesh.vertex_values(1).unwrap()[2], 0.0);
        assert_eq!(mesh.vertex_values(1).unwrap()[7], 0.0);
    }

    #[test]
    fn the_standard_format_keeps_whole_vertices() {
        let vertices = [
            MeshVertex::colored(1.0, 2.0, Color::rgba(0.1, 0.2, 0.3, 0.4)),
            MeshVertex::new(3.0, 4.0, 0.5, 0.25),
        ];
        let mut mesh = Mesh::new(&vertices, MeshDrawMode::Triangles);
        assert_eq!(mesh.vertices(), vertices);
        assert_eq!(
            mesh.vertex_values(1).unwrap(),
            [3.0, 4.0, 0.5, 0.25, 1.0, 1.0, 1.0, 1.0]
        );
        mesh.set_vertices(&vertices[..1]);
        assert_eq!(mesh.vertex_count(), 1);
    }

    #[test]
    fn small_meshes_use_16_bit_indices_unless_told_otherwise() {
        let mut mesh = Mesh::new(&corners(3), MeshDrawMode::Triangles);
        assert_eq!(mesh.index_data_type(), IndexDataType::U16);
        mesh.set_vertex_map_ex(Some(&[0, 1, 2]), IndexDataType::U32);
        assert_eq!(mesh.index_data_type(), IndexDataType::U32);
        mesh.set_vertex_map(Some(&[0, 1, 2]));
        assert_eq!(mesh.index_data_type(), IndexDataType::U16);
    }

    #[test]
    fn attached_attributes_get_buffers_of_their_own_and_replace_the_meshes_own() {
        let mut mesh =
            Mesh::with_format_and_count(&custom_format(), 3, MeshDrawMode::Triangles).unwrap();
        let offsets = Mesh::with_format_and_count(
            &[VertexAttribute::new("Offset", VertexDataType::Float, 2)],
            1,
            MeshDrawMode::Triangles,
        )
        .unwrap();
        let weights = Mesh::with_format_and_count(
            &[VertexAttribute::new("Weight", VertexDataType::Float, 1)],
            3,
            MeshDrawMode::Triangles,
        )
        .unwrap();
        assert!(mesh.attach_attribute("Missing", &offsets).is_err());
        mesh.attach_attribute_ex(
            "Offset",
            &offsets,
            VertexStep::PerInstance,
            "InstanceOffset",
        )
        .unwrap();
        mesh.attach_attribute("Weight", &weights).unwrap();

        let layout = mesh.draw_layout();
        assert_eq!(layout.buffers.len(), 3);
        assert_eq!(layout.buffers[1].step, VertexStep::PerInstance);
        assert_eq!(layout.buffers[2].stride, 4);
        let names: Vec<(&str, u32)> = layout
            .attributes
            .iter()
            .map(|attribute| (attribute.name.as_str(), attribute.buffer))
            .collect();
        // The mesh's own weight gives way to the attached one.
        assert_eq!(
            names,
            [
                (VERTEX_POSITION, 0),
                (VERTEX_COLOR, 0),
                ("InstanceOffset", 1),
                ("Weight", 2)
            ]
        );

        // An attribute that is off is left out, buffer and all.
        mesh.set_attribute_enabled("InstanceOffset", false);
        mesh.set_attribute_enabled(VERTEX_COLOR, false);
        assert!(!mesh.is_attribute_enabled(VERTEX_COLOR));
        let layout = mesh.draw_layout();
        assert_eq!(layout.buffers.len(), 2);
        let names: Vec<&str> = layout.attributes.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, [VERTEX_POSITION, "Weight"]);

        assert!(mesh.detach_attribute("Weight"));
        assert!(!mesh.detach_attribute("Weight"));
        mesh.set_attribute_enabled(VERTEX_COLOR, true);
        let layout = mesh.draw_layout();
        let names: Vec<&str> = layout.attributes.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, [VERTEX_POSITION, VERTEX_COLOR, "Weight"]);
    }
}
