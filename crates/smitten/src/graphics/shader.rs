use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use smitten_core::backend::{
    Backend, BoundTexture, ShaderId, ShaderTexture, TextureKind, UniformField, UniformKind,
};

use super::{ArrayImage, Canvas, CubeImage, GraphicsState, Image, Transform, VolumeImage};
use crate::reload::Asset;
use crate::{Color, context};

/// A program that decides the color of every pixel drawn while it is
/// active, the equivalent of LÖVE's `Shader`.
///
/// A shader is written in WGSL and defines one function, `effect`, which
/// is given the drawing color, the texture coordinate and the pixel's
/// position on the target, and returns the pixel's color:
///
/// ```wgsl
/// fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
///     let texel = textureSample(image, image_sampler, uv);
///     let gray = dot(texel.rgb, vec3<f32>(0.299, 0.587, 0.114));
///     return vec4<f32>(gray, gray, gray, texel.a) * color;
/// }
/// ```
///
/// `image` and `image_sampler` are the picture being drawn. Shapes have a
/// plain white picture, so the same shader works for them.
///
/// To pass values in from the game, declare a struct named `Params` and
/// read it through `params`, then set its members with [`Shader::send`]:
///
/// ```wgsl
/// struct Params {
///     time: f32,
///     tint: vec4<f32>,
/// }
///
/// fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
///     return textureSample(image, image_sampler, uv) * params.tint;
/// }
/// ```
///
/// Members may be `f32`, `i32` or `u32`, vectors of two to four of them
/// such as `vec3<f32>` or `vec2<i32>`, the square matrices `mat2x2<f32>`,
/// `mat3x3<f32>` and `mat4x4<f32>`, or fixed-size arrays of any of these,
/// set with [`Shader::send_array`]. They start out as zero. See
/// [`UniformValue`] for what to send to each.
///
/// WGSL has no booleans in `Params`: declare a `u32` or `i32` and send it
/// a `bool`, which arrives as 1 or 0. The elements of an array in
/// `Params` must be 16 bytes apart, so use `array<vec4<f32>, N>` rather
/// than `array<f32, N>`.
///
/// A shader can read more textures than the one being drawn. Declare each
/// with no binding, use it with the sampler named after it, and send it
/// an [`Image`], [`Canvas`], [`ArrayImage`], [`CubeImage`] or [`VolumeImage`] with
/// [`Shader::send_texture`]. One that has not been sent reads as white.
///
/// ```wgsl
/// var sky: texture_cube<f32>;
///
/// fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
///     let direction = vec3<f32>(uv * 2.0 - 1.0, 1.0);
///     return textureSample(sky, sky_sampler, direction) * color;
/// }
/// ```
///
/// The types it can be are `texture_2d<f32>`, `texture_2d_array<f32>`,
/// `texture_cube<f32>`, `texture_3d<f32>` and `texture_depth_2d`.
///
/// A `texture_depth_2d` is LÖVE's `sampler2DShadow`: its sampler is a
/// `sampler_comparison`, and it takes a readable depth canvas with a
/// depth sample mode set by [`Image::set_depth_sample_mode`]. Until one is
/// sent, every comparison reads as passing. This is how a shadow map is
/// read:
///
/// ```wgsl
/// var shadow: texture_depth_2d;
///
/// struct Params { light_uv: vec2<f32>, light_depth: f32 }
///
/// fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
///     // 1 where nothing nearer the light was drawn, 0 in shadow.
///     let lit = textureSampleCompare(shadow, shadow_sampler, params.light_uv, params.light_depth);
///     return vec4<f32>(color.rgb * (0.4 + 0.6 * lit), color.a);
/// }
/// ```
///
/// A depth canvas without a depth sample mode is sent to a
/// `texture_2d<f32>` instead, and reads as its depth in the red channel.
///
/// When several canvases are drawn to at once with
/// [`set_canvases`](super::set_canvases), `effect`'s color goes to all of
/// them. To give each its own, define `effects` in place of `effect`,
/// returning a struct named `Outputs` with a `vec4<f32>` for each canvas
/// in order, the equivalent of writing `love_Canvases[i]` in LÖVE:
///
/// ```wgsl
/// struct Outputs {
///     albedo: vec4<f32>,
///     normal: vec4<f32>,
/// }
///
/// fn effects(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> Outputs {
///     var out: Outputs;
///     out.albedo = textureSample(image, image_sampler, uv) * color;
///     out.normal = vec4<f32>(0.5, 0.5, 1.0, 1.0);
///     return out;
/// }
/// ```
///
/// A canvas past the last member of `Outputs` gets transparent black, and
/// a member past the last canvas is left out.
///
/// # Vertex shaders
///
/// A shader can also move the vertices of what it draws by defining
/// `position`, the equivalent of LÖVE's vertex shader. It is given the
/// matrix that takes the vertex to the screen and the vertex's position,
/// and returns where on the screen it goes:
///
/// ```wgsl
/// struct Params { time: f32 }
///
/// fn position(transform_projection: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32> {
///     let wave = sin(params.time + vertex_position.x * 0.05) * 4.0;
///     return transform_projection * (vertex_position + vec4<f32>(0.0, wave, 0.0, 0.0));
/// }
/// ```
///
/// A shader with `position` and no `effect` colors pixels as the built-in
/// shader does. `params` and the shader's own textures can be read in both
/// functions, but `position` must sample with `textureSampleLevel`.
///
/// A [`Mesh`](super::Mesh) is given in its own coordinates, with the
/// drawing transform in `transform_projection`. Everything else, such as
/// images, shapes and text, is placed on the CPU first, so its
/// `vertex_position` is already in pixels of the target and
/// `transform_projection` only takes pixels to the screen.
///
/// To read a mesh's own vertex attributes, declare a struct named
/// `Attributes` and read it through `attributes`. Each member is filled
/// from the attribute of the same name, such as `VertexColor` or one
/// attached with [`Mesh::attach_attribute`](super::Mesh::attach_attribute),
/// and must be `f32` or `vec2<f32>` to `vec4<f32>`. One that the vertices
/// lack reads as (0, 0, 0, 1), as in LÖVE.
///
/// When copies of a mesh are drawn with
/// [`draw_instanced`](super::draw_instanced), `instance_index` in
/// `position` is the `u32` number of the copy being drawn, from 0, as
/// LÖVE's `love_InstanceID`. It is 0 in `effect` and for other draws.
///
/// To pass values from `position` to `effect`, declare a struct named
/// `Varyings`, write it through `varyings` in `position` and read it in
/// `effect`. Members may be `f32`, `i32` or `u32`, or vectors of them, at
/// most 14. Floats are blended between the vertices of each triangle;
/// integers are taken from one of its vertices.
///
/// ```wgsl
/// struct Attributes { Offset: vec2<f32> }
/// struct Varyings { height: f32 }
///
/// fn position(transform_projection: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32> {
///     let moved = vertex_position + vec4<f32>(attributes.Offset, 0.0, 0.0);
///     varyings.height = moved.y / 100.0;
///     return transform_projection * moved;
/// }
///
/// fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
///     return textureSample(image, image_sampler, uv) * color * varyings.height;
/// }
/// ```
///
/// Cloning is cheap and gives another handle to the same shader.
#[derive(Clone)]
pub struct Shader {
    inner: Rc<Inner>,
}

pub(crate) struct Inner {
    // In cells so a changed file can be loaded into every handle at once.
    id: Cell<ShaderId>,
    fields: RefCell<Vec<UniformField>>,
    /// The current value of `Params`, laid out as the GPU expects.
    data: RefCell<Vec<u8>>,
    /// The textures the shader declares, and what has been sent to each.
    textures: RefCell<Vec<ShaderTexture>>,
    sent: RefCell<Vec<Option<BoundTexture>>>,
    /// Keeps what was sent alive while the shader may draw with it.
    kept: RefCell<Vec<Option<Rc<dyn Any>>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // If the game has already shut down, the backend freed everything.
        context::try_with(|ctx| ctx.backend.destroy_shader(self.id.get()));
    }
}

impl Shader {
    /// Compiles a shader from WGSL source.
    pub fn new(source: &str) -> Result<Shader, ShaderError> {
        let info =
            context::with(|ctx| ctx.backend.create_shader(source)).map_err(ShaderError::Compile)?;
        Ok(Shader {
            inner: Rc::new(Inner {
                id: Cell::new(info.id),
                fields: RefCell::new(info.fields),
                data: RefCell::new(vec![0; info.uniform_size as usize]),
                sent: RefCell::new(vec![None; info.textures.len()]),
                kept: RefCell::new(vec![None; info.textures.len()]),
                textures: RefCell::new(info.textures),
            }),
        })
    }

    /// Compiles a shader from a WGSL file.
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes,
    /// which normally means inside the game's `assets` folder.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Shader, ShaderError> {
        let path = path.as_ref();
        let source = crate::filesystem::read_string(path).map_err(|source| ShaderError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let shader = Shader::new(&source)?;
        let file = crate::filesystem::resolve(path);
        context::with(|ctx| {
            ctx.reload
                .watch(file, Asset::Shader(Rc::downgrade(&shader.inner)));
        });
        Ok(shader)
    }

    pub(crate) fn from_inner(inner: Rc<Inner>) -> Shader {
        Shader { inner }
    }

    /// Sets the member of `Params` called `name`.
    ///
    /// The value applies to everything drawn with this shader afterwards,
    /// and what was drawn before keeps the value it was drawn with. So
    /// one shader can draw several things in a frame with a different
    /// value for each: send, draw, send again, draw again.
    ///
    /// Panics if the shader has no such member or it has a different type,
    /// which is always a bug in the game. Use [`Shader::try_send`] to
    /// handle the failure instead.
    pub fn send(&self, name: &str, value: impl UniformValue) {
        self.try_send(name, value)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    /// Sets the `vec4<f32>` member of `Params` called
    /// `name` to `color`, the equivalent of `Shader:sendColor`. With
    /// gamma-correct rendering, the color is converted from sRGB to linear
    /// light first, as colors from `set_color` are; [`Shader::send`] sends
    /// it unchanged.
    ///
    /// Panics if there is no such member or it has a different type. Use
    /// [`Shader::try_send_color`] to handle the failure instead.
    pub fn send_color(&self, name: &str, color: Color) {
        self.try_send_color(name, color)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    /// As [`Shader::send_color`], returning an error if there is no such
    /// member or it has a different type.
    pub fn try_send_color(&self, name: &str, color: Color) -> Result<(), ShaderError> {
        let color = if super::is_gamma_correct() {
            use crate::math::gamma_to_linear;
            Color::rgba(
                gamma_to_linear(color.r),
                gamma_to_linear(color.g),
                gamma_to_linear(color.b),
                color.a,
            )
        } else {
            color
        };
        self.try_send(name, color)
    }

    /// Sets the member of `Params` called `name`, returning an error if
    /// there is no such member or it has a different type.
    ///
    /// For an array member, this sets its first element.
    pub fn try_send(&self, name: &str, value: impl UniformValue) -> Result<(), ShaderError> {
        self.try_send_array(name, std::slice::from_ref(&value))
    }

    /// Sets the first elements of the array member of `Params` called
    /// `name`, the equivalent of `shader:send(name, value1, value2, ...)`.
    /// Elements past those sent keep their values.
    ///
    /// ```no_run
    /// # let shader = smitten::graphics::new_shader("");
    /// // struct Params { lights: array<vec4<f32>, 8>, light_count: u32 }
    /// let lights = [[100.0, 80.0, 1.0, 0.0], [300.0, 200.0, 0.5, 0.0]];
    /// shader.send_array("lights", &lights);
    /// shader.send("light_count", lights.len() as u32);
    /// ```
    ///
    /// A member that is not an array counts as an array of one.
    ///
    /// Panics if there is no such member, it has a different type, or it
    /// has room for fewer values than were sent, which is always a bug in
    /// the game. Use [`Shader::try_send_array`] to handle the failure
    /// instead.
    pub fn send_array<V: UniformValue>(&self, name: &str, values: &[V]) {
        self.try_send_array(name, values)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    /// Sets the first elements of the array member of `Params` called
    /// `name`, returning an error if there is no such member, it has a
    /// different type, or it is too short.
    pub fn try_send_array<V: UniformValue>(
        &self,
        name: &str,
        values: &[V],
    ) -> Result<(), ShaderError> {
        {
            let fields = self.inner.fields.borrow();
            let field = fields
                .iter()
                .find(|field| field.name == name)
                .ok_or_else(|| ShaderError::UnknownValue(name.to_string()))?;
            let (count, stride) = match (field.kind, field.array) {
                (Some(kind), None) => (1, kind.size()),
                (_, Some(array)) => (array.count as usize, array.stride as usize),
                (None, None) => (0, 0),
            };
            let mut encoded = Vec::with_capacity(values.len());
            for value in values {
                let bytes = field
                    .kind
                    .and_then(|declared| value.to_uniform(declared).map(|bytes| (declared, bytes)))
                    .ok_or_else(|| ShaderError::WrongType {
                        name: name.to_string(),
                        sent: value.kind(),
                        declared: field.kind,
                    })?;
                encoded.push(bytes);
            }
            if values.len() > count {
                return Err(ShaderError::TooManyValues {
                    name: name.to_string(),
                    sent: values.len(),
                    room: count,
                });
            }
            let mut data = self.inner.data.borrow_mut();
            for (index, (kind, bytes)) in encoded.iter().enumerate() {
                let start = field.offset as usize + index * stride;
                // The backend reported a layout that fits inside the struct.
                data[start..start + kind.size()].copy_from_slice(&bytes[..kind.size()]);
            }
        }
        // If this shader is the active one, later draws must see the change.
        context::with(|ctx| ctx.gfx.refresh_shader(self));
        Ok(())
    }

    /// Sets the matrix member of `Params` called `name`, with the matrix
    /// given as LÖVE's `MatrixLayout` says: a list of rows or of columns.
    ///
    /// [`Shader::send`] takes a matrix as a list of columns, as WGSL and
    /// most Rust math libraries store it. A matrix written out by hand
    /// reads more naturally as rows:
    ///
    /// ```no_run
    /// use smitten::graphics::MatrixLayout;
    /// # let shader = smitten::graphics::new_shader("");
    /// // struct Params { tint: mat3x3<f32> }
    /// let sepia = [
    ///     [0.393, 0.769, 0.189],
    ///     [0.349, 0.686, 0.168],
    ///     [0.272, 0.534, 0.131],
    /// ];
    /// shader.send_matrix("tint", sepia, MatrixLayout::Rows);
    /// ```
    ///
    /// Panics as [`Shader::send`] does.
    pub fn send_matrix<const N: usize>(
        &self,
        name: &str,
        matrix: [[f32; N]; N],
        layout: MatrixLayout,
    ) where
        [[f32; N]; N]: UniformValue,
    {
        self.send(name, layout.to_columns(matrix));
    }

    /// Sends a texture to the one the shader declares as `name`, the
    /// equivalent of `shader:send(name, image)`. Like [`Shader::send`],
    /// it applies to what is drawn afterwards. The texture is sampled with
    /// the filter it has now.
    ///
    /// Panics if the shader declares no such texture or one of another
    /// kind, which is always a bug in the game. Use
    /// [`Shader::try_send_texture`] to handle the failure instead.
    pub fn send_texture(&self, name: &str, texture: &impl Texture) {
        self.try_send_texture(name, texture)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    /// Sends a texture to the one the shader declares as `name`,
    /// returning an error if there is none or it is of another kind.
    pub fn try_send_texture(&self, name: &str, texture: &impl Texture) -> Result<(), ShaderError> {
        let (bound, kind, keep) = texture.binding();
        let index = {
            let textures = self.inner.textures.borrow();
            let (index, declared) = textures
                .iter()
                .enumerate()
                .find(|(_, declared)| declared.name == name)
                .ok_or_else(|| ShaderError::UnknownTexture(name.to_string()))?;
            if declared.kind != kind {
                return Err(ShaderError::WrongTextureKind {
                    name: name.to_string(),
                    sent: kind,
                    declared: declared.kind,
                });
            }
            if declared.comparison != texture.compares() {
                return Err(ShaderError::WrongDepthSampleMode {
                    name: name.to_string(),
                    declared_depth: declared.comparison,
                });
            }
            index
        };
        self.inner.sent.borrow_mut()[index] = Some(bound);
        self.inner.kept.borrow_mut()[index] = Some(keep);
        context::with(|ctx| ctx.gfx.refresh_shader(self));
        Ok(())
    }

    /// Returns the compiler's warnings about the shader, as LÖVE's
    /// `Shader:getWarnings`. The WGSL compiler reports problems only as
    /// errors, from [`Shader::new`], so this is always empty.
    pub fn warnings(&self) -> String {
        String::new()
    }

    /// Returns whether the shader declares a texture called `name`.
    pub fn has_texture(&self, name: &str) -> bool {
        self.inner
            .textures
            .borrow()
            .iter()
            .any(|texture| texture.name == name)
    }

    /// Returns whether `Params` has a member called `name`.
    pub fn has_value(&self, name: &str) -> bool {
        self.inner
            .fields
            .borrow()
            .iter()
            .any(|field| field.name == name)
    }

    pub(crate) fn id(&self) -> ShaderId {
        self.inner.id.get()
    }

    /// Calls `f` with the uniform data and the textures sent so far.
    pub(crate) fn with_data<R>(&self, f: impl FnOnce(&[u8], &[Option<BoundTexture>]) -> R) -> R {
        f(&self.inner.data.borrow(), &self.inner.sent.borrow())
    }

    pub(crate) fn is(&self, other: &Shader) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Replaces a shader with a newer version of its file. Values already
/// sent are kept for the members that still exist with the same type.
pub(crate) fn reload(
    inner: &Rc<Inner>,
    backend: &mut dyn Backend,
    gfx: &mut GraphicsState,
    bytes: &[u8],
) -> Result<(), String> {
    let source = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    // If this fails the old shader is untouched and stays in use.
    let info = backend.create_shader(source)?;

    let mut data = vec![0; info.uniform_size as usize];
    {
        let old_fields = inner.fields.borrow();
        let old_data = inner.data.borrow();
        for field in &info.fields {
            let old = old_fields.iter().find(|old| {
                old.name == field.name && old.kind == field.kind && old.array == field.array
            });
            if let Some(old) = old {
                let (from, to, span) = (old.offset as usize, field.offset as usize, field.span());
                data[to..to + span].copy_from_slice(&old_data[from..from + span]);
            }
        }
    }
    // Textures sent to a declaration that is still there are kept too.
    let mut sent = vec![None; info.textures.len()];
    let mut kept = vec![None; info.textures.len()];
    {
        let old_textures = inner.textures.borrow();
        let mut old_sent = inner.sent.borrow_mut();
        let mut old_kept = inner.kept.borrow_mut();
        for (index, texture) in info.textures.iter().enumerate() {
            if let Some(old) = old_textures.iter().position(|old| old == texture) {
                sent[index] = old_sent[old].take();
                kept[index] = old_kept[old].take();
            }
        }
    }
    backend.destroy_shader(inner.id.replace(info.id));
    *inner.fields.borrow_mut() = info.fields;
    *inner.data.borrow_mut() = data;
    *inner.textures.borrow_mut() = info.textures;
    *inner.sent.borrow_mut() = sent;
    *inner.kept.borrow_mut() = kept;
    // If the shader is in use, drawing must switch to the new one.
    gfx.refresh_shader(&Shader::from_inner(inner.clone()));
    Ok(())
}

impl fmt::Debug for Shader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fields = self.inner.fields.borrow();
        let names: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
        f.debug_struct("Shader").field("values", &names).finish()
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Something with pixels a shader can read: an [`Image`], a [`Canvas`],
/// an [`ArrayImage`], a [`CubeImage`] or a [`VolumeImage`]. See [`Shader::send_texture`].
pub trait Texture: sealed::Sealed {
    /// The texture to bind, its kind, and a handle that keeps it alive.
    #[doc(hidden)]
    fn binding(&self) -> (BoundTexture, TextureKind, Rc<dyn Any>);

    /// Whether it is a depth canvas with a depth sample mode, which only
    /// a `texture_depth_2d` can be sent.
    #[doc(hidden)]
    fn compares(&self) -> bool {
        false
    }
}

impl sealed::Sealed for Image {}
impl Texture for Image {
    fn binding(&self) -> (BoundTexture, TextureKind, Rc<dyn Any>) {
        let bound = BoundTexture {
            texture: self.texture(),
            filter: self.filter(),
        };
        (bound, TextureKind::Flat, self.keep_alive())
    }

    fn compares(&self) -> bool {
        self.depth_sample_mode().is_some()
    }
}

impl sealed::Sealed for Canvas {}
impl Texture for Canvas {
    fn binding(&self) -> (BoundTexture, TextureKind, Rc<dyn Any>) {
        let image: &Image = self;
        let (bound, _, keep) = image.binding();
        (bound, self.texture_type(), keep)
    }

    fn compares(&self) -> bool {
        self.depth_sample_mode().is_some()
    }
}

impl sealed::Sealed for ArrayImage {}
impl Texture for ArrayImage {
    fn binding(&self) -> (BoundTexture, TextureKind, Rc<dyn Any>) {
        let bound = BoundTexture {
            texture: self.texture(),
            filter: self.filter(),
        };
        (bound, TextureKind::Array, self.keep_alive())
    }
}

impl sealed::Sealed for CubeImage {}
impl Texture for CubeImage {
    fn binding(&self) -> (BoundTexture, TextureKind, Rc<dyn Any>) {
        let bound = BoundTexture {
            texture: self.texture(),
            filter: self.filter(),
        };
        (bound, TextureKind::Cube, self.keep_alive())
    }
}

impl sealed::Sealed for VolumeImage {}
impl Texture for VolumeImage {
    fn binding(&self) -> (BoundTexture, TextureKind, Rc<dyn Any>) {
        let bound = BoundTexture {
            texture: self.texture(),
            filter: self.filter(),
        };
        (bound, TextureKind::Volume, self.keep_alive())
    }
}

fn kind_name(kind: TextureKind) -> &'static str {
    match kind {
        TextureKind::Flat => "texture_2d<f32>",
        TextureKind::Array => "texture_2d_array<f32>",
        TextureKind::Cube => "texture_cube<f32>",
        TextureKind::Volume => "texture_3d<f32>",
    }
}

/// How the numbers of a matrix are listed, the equivalent of LÖVE's
/// `MatrixLayout`. See [`Shader::send_matrix`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MatrixLayout {
    /// Each inner array is a row, as a matrix is written on paper. LÖVE's
    /// default.
    #[default]
    Rows,
    /// Each inner array is a column, as WGSL stores a matrix.
    Columns,
}

impl MatrixLayout {
    fn to_columns<const N: usize>(self, matrix: [[f32; N]; N]) -> [[f32; N]; N] {
        match self {
            MatrixLayout::Columns => matrix,
            MatrixLayout::Rows => {
                std::array::from_fn(|column| std::array::from_fn(|row| matrix[row][column]))
            }
        }
    }
}

/// A value that can be passed to [`Shader::send`].
///
/// | Rust | WGSL |
/// | --- | --- |
/// | `f32` | `f32` |
/// | `i32` | `i32` |
/// | `u32` | `u32` |
/// | `bool` | `u32` or `i32`, as 1 or 0 |
/// | `[f32; 2]`, `(f32, f32)` | `vec2<f32>` |
/// | `[f32; 3]` | `vec3<f32>` |
/// | `[f32; 4]`, [`Color`] | `vec4<f32>` |
/// | `[i32; 2]` to `[i32; 4]` | `vec2<i32>` to `vec4<i32>` |
/// | `[u32; 2]` to `[u32; 4]` | `vec2<u32>` to `vec4<u32>` |
/// | `[[f32; 2]; 2]` to `[[f32; 4]; 4]` | `mat2x2<f32>` to `mat4x4<f32>`, as columns |
/// | [`Transform`] | `mat3x3<f32>` or `mat4x4<f32>` |
///
/// A [`Transform`] arrives as the matrix that maps `vec3(x, y, 1.0)`, or
/// `vec4(x, y, 0.0, 1.0)`, to where the transform puts the point.
pub trait UniformValue {
    /// The WGSL type this value most naturally has, for error messages.
    #[doc(hidden)]
    fn kind(&self) -> UniformKind;

    /// The bytes of this value as a member of type `declared`, laid out
    /// as the GPU reads it, or `None` if it cannot be one. Only
    /// `declared.size()` bytes are used.
    #[doc(hidden)]
    fn to_uniform(&self, declared: UniformKind) -> Option<[u8; UniformKind::MAX_SIZE]>;
}

/// Writes `values` as 4-byte numbers, one column of `rows` numbers every
/// `stride` bytes.
fn pack<T: Copy>(
    values: &[T],
    rows: usize,
    stride: usize,
    bytes_of: impl Fn(T) -> [u8; 4],
) -> [u8; UniformKind::MAX_SIZE] {
    let mut bytes = [0; UniformKind::MAX_SIZE];
    for (index, value) in values.iter().enumerate() {
        let start = index / rows * stride + index % rows * 4;
        bytes[start..start + 4].copy_from_slice(&bytes_of(*value));
    }
    bytes
}

/// A value of exactly one WGSL type.
macro_rules! uniform {
    ($rust:ty, $kind:ident, |$value:ident| $numbers:expr, $bytes_of:expr) => {
        impl UniformValue for $rust {
            fn kind(&self) -> UniformKind {
                UniformKind::$kind
            }

            fn to_uniform(&self, declared: UniformKind) -> Option<[u8; UniformKind::MAX_SIZE]> {
                let $value = self;
                (declared == UniformKind::$kind).then(|| {
                    let numbers = $numbers;
                    pack(&numbers, numbers.len(), 0, $bytes_of)
                })
            }
        }
    };
}

uniform!(f32, Float, |v| [*v], f32::to_ne_bytes);
uniform!(i32, Int, |v| [*v], i32::to_ne_bytes);
uniform!(u32, Uint, |v| [*v], u32::to_ne_bytes);
uniform!([f32; 2], Vec2, |v| *v, f32::to_ne_bytes);
uniform!((f32, f32), Vec2, |v| [v.0, v.1], f32::to_ne_bytes);
uniform!([f32; 3], Vec3, |v| *v, f32::to_ne_bytes);
uniform!([f32; 4], Vec4, |v| *v, f32::to_ne_bytes);
uniform!(Color, Vec4, |v| [v.r, v.g, v.b, v.a], f32::to_ne_bytes);
uniform!([i32; 2], IVec2, |v| *v, i32::to_ne_bytes);
uniform!([i32; 3], IVec3, |v| *v, i32::to_ne_bytes);
uniform!([i32; 4], IVec4, |v| *v, i32::to_ne_bytes);
uniform!([u32; 2], UVec2, |v| *v, u32::to_ne_bytes);
uniform!([u32; 3], UVec3, |v| *v, u32::to_ne_bytes);
uniform!([u32; 4], UVec4, |v| *v, u32::to_ne_bytes);

impl UniformValue for bool {
    fn kind(&self) -> UniformKind {
        UniformKind::Uint
    }

    fn to_uniform(&self, declared: UniformKind) -> Option<[u8; UniformKind::MAX_SIZE]> {
        match declared {
            UniformKind::Int => (i32::from(*self)).to_uniform(declared),
            UniformKind::Uint => (u32::from(*self)).to_uniform(declared),
            _ => None,
        }
    }
}

/// A matrix of `N` columns of `N` numbers.
macro_rules! matrix {
    ($n:literal, $kind:ident, $stride:literal) => {
        impl UniformValue for [[f32; $n]; $n] {
            fn kind(&self) -> UniformKind {
                UniformKind::$kind
            }

            fn to_uniform(&self, declared: UniformKind) -> Option<[u8; UniformKind::MAX_SIZE]> {
                (declared == UniformKind::$kind)
                    .then(|| pack(self.as_flattened(), $n, $stride, f32::to_ne_bytes))
            }
        }
    };
}

matrix!(2, Mat2, 8);
matrix!(3, Mat3, 16);
matrix!(4, Mat4, 16);

impl UniformValue for Transform {
    fn kind(&self) -> UniformKind {
        UniformKind::Mat4
    }

    fn to_uniform(&self, declared: UniformKind) -> Option<[u8; UniformKind::MAX_SIZE]> {
        let [[a, c, tx], [b, d, ty], _] = self.matrix();
        match declared {
            UniformKind::Mat3 => [[a, b, 0.0], [c, d, 0.0], [tx, ty, 1.0]].to_uniform(declared),
            UniformKind::Mat4 => [
                [a, b, 0.0, 0.0],
                [c, d, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [tx, ty, 0.0, 1.0],
            ]
            .to_uniform(declared),
            _ => None,
        }
    }
}

impl<T: UniformValue + ?Sized> UniformValue for &T {
    fn kind(&self) -> UniformKind {
        (**self).kind()
    }

    fn to_uniform(&self, declared: UniformKind) -> Option<[u8; UniformKind::MAX_SIZE]> {
        (**self).to_uniform(declared)
    }
}

fn wgsl_name(kind: UniformKind) -> &'static str {
    match kind {
        UniformKind::Float => "f32",
        UniformKind::Int => "i32",
        UniformKind::Uint => "u32",
        UniformKind::Vec2 => "vec2<f32>",
        UniformKind::Vec3 => "vec3<f32>",
        UniformKind::Vec4 => "vec4<f32>",
        UniformKind::IVec2 => "vec2<i32>",
        UniformKind::IVec3 => "vec3<i32>",
        UniformKind::IVec4 => "vec4<i32>",
        UniformKind::UVec2 => "vec2<u32>",
        UniformKind::UVec3 => "vec3<u32>",
        UniformKind::UVec4 => "vec4<u32>",
        UniformKind::Mat2 => "mat2x2<f32>",
        UniformKind::Mat3 => "mat3x3<f32>",
        UniformKind::Mat4 => "mat4x4<f32>",
    }
}

/// Why a shader could not be created or given a value.
#[derive(Debug)]
pub enum ShaderError {
    /// The file could not be read.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The source is not a valid shader. The message says where and why.
    Compile(String),
    /// The shader's `Params` has no member with this name.
    UnknownValue(String),
    /// The member exists but is declared with a different type.
    WrongType {
        name: String,
        sent: UniformKind,
        /// `None` if the member has a type that cannot be sent at all.
        declared: Option<UniformKind>,
    },
    /// More values were sent than the member has room for.
    TooManyValues {
        name: String,
        sent: usize,
        room: usize,
    },
    /// The shader declares no texture with this name.
    UnknownTexture(String),
    /// The texture is declared as a different kind from the one sent,
    /// such as a cube image sent to a `texture_2d<f32>`.
    WrongTextureKind {
        name: String,
        sent: TextureKind,
        declared: TextureKind,
    },
    /// A `texture_depth_2d` was sent something other than a depth canvas
    /// with a depth sample mode, or another texture was sent one.
    WrongDepthSampleMode {
        name: String,
        /// Whether the shader declares it as a `texture_depth_2d`.
        declared_depth: bool,
    },
}

impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShaderError::Read { path, source } => {
                write!(f, "could not read shader {}: {source}", path.display())
            }
            ShaderError::Compile(message) => write!(f, "could not compile shader:\n{message}"),
            ShaderError::UnknownValue(name) => {
                write!(f, "the shader's Params has no member named `{name}`")
            }
            ShaderError::WrongType {
                name,
                sent,
                declared,
            } => {
                write!(f, "shader value `{name}` was sent a {}", wgsl_name(*sent))?;
                match declared {
                    Some(declared) => write!(f, " but is declared as {}", wgsl_name(*declared)),
                    None => write!(f, " but has a type that cannot be sent"),
                }
            }
            ShaderError::TooManyValues { name, sent, room } => write!(
                f,
                "shader value `{name}` was sent {sent} values but has room for {room}"
            ),
            ShaderError::UnknownTexture(name) => {
                write!(f, "the shader declares no texture named `{name}`")
            }
            ShaderError::WrongTextureKind {
                name,
                sent,
                declared,
            } => write!(
                f,
                "shader texture `{name}` was sent a {} but is declared as {}",
                kind_name(*sent),
                kind_name(*declared)
            ),
            ShaderError::WrongDepthSampleMode {
                name,
                declared_depth: true,
            } => write!(
                f,
                "shader texture `{name}` is a `texture_depth_2d`, which takes a depth canvas \
                 with a depth sample mode set"
            ),
            ShaderError::WrongDepthSampleMode {
                name,
                declared_depth: false,
            } => write!(
                f,
                "shader texture `{name}` was sent a depth canvas with a depth sample mode, \
                 which only a `texture_depth_2d` takes; clear the mode to read plain depth"
            ),
        }
    }
}

impl std::error::Error for ShaderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ShaderError::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floats(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_ne_bytes(chunk.try_into().unwrap()))
            .collect()
    }

    #[test]
    fn values_are_packed_as_the_gpu_reads_them() {
        let bytes = Color::rgba(1.0, 0.5, 0.25, 0.0)
            .to_uniform(UniformKind::Vec4)
            .unwrap();
        assert_eq!(floats(&bytes[..16]), [1.0, 0.5, 0.25, 0.0]);

        let bytes = (-3i32).to_uniform(UniformKind::Int).unwrap();
        assert_eq!(bytes[..4], (-3i32).to_ne_bytes());

        assert_eq!(
            (2.0f32, 3.0f32).to_uniform(UniformKind::Vec2),
            [2.0f32, 3.0].to_uniform(UniformKind::Vec2)
        );
        assert_eq!(Color::WHITE.to_uniform(UniformKind::Vec3), None);
    }

    #[test]
    fn booleans_fit_either_integer() {
        assert_eq!(
            true.to_uniform(UniformKind::Uint),
            1u32.to_uniform(UniformKind::Uint)
        );
        assert_eq!(
            false.to_uniform(UniformKind::Int),
            0i32.to_uniform(UniformKind::Int)
        );
        assert_eq!(true.to_uniform(UniformKind::Float), None);
    }

    #[test]
    fn matrix_columns_are_padded_to_their_alignment() {
        let bytes = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]]
            .to_uniform(UniformKind::Mat3)
            .unwrap();
        assert_eq!(
            floats(&bytes[..48]),
            [1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 6.0, 0.0, 7.0, 8.0, 9.0, 0.0]
        );
        let bytes = [[1.0, 2.0], [3.0, 4.0]]
            .to_uniform(UniformKind::Mat2)
            .unwrap();
        assert_eq!(floats(&bytes[..16]), [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn rows_are_turned_into_columns() {
        let rows = [[1.0, 2.0], [3.0, 4.0]];
        assert_eq!(
            MatrixLayout::Rows.to_columns(rows),
            [[1.0, 3.0], [2.0, 4.0]]
        );
        assert_eq!(MatrixLayout::Columns.to_columns(rows), rows);
    }

    #[test]
    fn a_transform_becomes_the_matrix_that_moves_points_as_it_does() {
        let mut transform = Transform::IDENTITY;
        transform.translate(10.0, 20.0);
        transform.scale(2.0, 3.0);
        let bytes = transform.to_uniform(UniformKind::Mat3).unwrap();
        // Columns: x axis, y axis, translation.
        assert_eq!(
            floats(&bytes[..48]),
            [2.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 10.0, 20.0, 1.0, 0.0]
        );
        let bytes = transform.to_uniform(UniformKind::Mat4).unwrap();
        assert_eq!(floats(&bytes[48..64]), [10.0, 20.0, 0.0, 1.0]);
        assert_eq!(transform.to_uniform(UniformKind::Vec4), None);
    }

    #[test]
    fn a_type_mismatch_says_both_types() {
        let error = ShaderError::WrongType {
            name: "tint".to_string(),
            sent: UniformKind::Float,
            declared: Some(UniformKind::Vec4),
        };
        assert_eq!(
            error.to_string(),
            "shader value `tint` was sent a f32 but is declared as vec4<f32>"
        );
    }

    #[test]
    fn a_texture_kind_mismatch_says_both_kinds() {
        let error = ShaderError::WrongTextureKind {
            name: "sky".to_string(),
            sent: TextureKind::Flat,
            declared: TextureKind::Cube,
        };
        assert_eq!(
            error.to_string(),
            "shader texture `sky` was sent a texture_2d<f32> but is declared as texture_cube<f32>"
        );
    }
}
