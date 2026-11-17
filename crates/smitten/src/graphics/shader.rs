use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use smitten_core::backend::{
    Backend, BoundTexture, ShaderId, ShaderTexture, TextureKind, UniformField, UniformKind,
};

use super::{ArrayImage, Canvas, CubeImage, GraphicsState, Image};
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
/// Members may be `f32`, `i32`, `u32`, `vec2<f32>`, `vec3<f32>` or
/// `vec4<f32>`. They start out as zero.
///
/// A shader can read more textures than the one being drawn. Declare each
/// with no binding, use it with the sampler named after it, and send it
/// an [`Image`], [`Canvas`], [`ArrayImage`] or [`CubeImage`] with
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
/// The types it can be are `texture_2d<f32>`, `texture_2d_array<f32>` and
/// `texture_cube<f32>`.
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

    /// Sets the member of `Params` called `name`, returning an error if
    /// there is no such member or it has a different type.
    pub fn try_send(&self, name: &str, value: impl UniformValue) -> Result<(), ShaderError> {
        let (kind, components) = value.to_uniform();
        let fields = self.inner.fields.borrow();
        let field = fields
            .iter()
            .find(|field| field.name == name)
            .ok_or_else(|| ShaderError::UnknownValue(name.to_string()))?;
        if field.kind != Some(kind) {
            return Err(ShaderError::WrongType {
                name: name.to_string(),
                sent: kind,
                declared: field.kind,
            });
        }
        {
            let mut data = self.inner.data.borrow_mut();
            let start = field.offset as usize;
            let bytes = &components[..kind.size()];
            // The backend reported a layout that fits inside the struct.
            data[start..start + bytes.len()].copy_from_slice(bytes);
        }
        drop(fields);
        // If this shader is the active one, later draws must see the change.
        context::with(|ctx| ctx.gfx.refresh_shader(self));
        Ok(())
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
            index
        };
        self.inner.sent.borrow_mut()[index] = Some(bound);
        self.inner.kept.borrow_mut()[index] = Some(keep);
        context::with(|ctx| ctx.gfx.refresh_shader(self));
        Ok(())
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
            let old = old_fields
                .iter()
                .find(|old| old.name == field.name && old.kind == field.kind);
            if let (Some(old), Some(kind)) = (old, field.kind) {
                let (from, to) = (old.offset as usize, field.offset as usize);
                data[to..to + kind.size()].copy_from_slice(&old_data[from..from + kind.size()]);
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
/// an [`ArrayImage`] or a [`CubeImage`]. See [`Shader::send_texture`].
pub trait Texture: sealed::Sealed {
    /// The texture to bind, its kind, and a handle that keeps it alive.
    #[doc(hidden)]
    fn binding(&self) -> (BoundTexture, TextureKind, Rc<dyn Any>);
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
}

impl sealed::Sealed for Canvas {}
impl Texture for Canvas {
    fn binding(&self) -> (BoundTexture, TextureKind, Rc<dyn Any>) {
        let image: &Image = self;
        image.binding()
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

fn kind_name(kind: TextureKind) -> &'static str {
    match kind {
        TextureKind::Flat => "texture_2d<f32>",
        TextureKind::Array => "texture_2d_array<f32>",
        TextureKind::Cube => "texture_cube<f32>",
    }
}

/// A value that can be passed to [`Shader::send`].
///
/// | Rust | WGSL |
/// | --- | --- |
/// | `f32` | `f32` |
/// | `i32` | `i32` |
/// | `u32` | `u32` |
/// | `[f32; 2]`, `(f32, f32)` | `vec2<f32>` |
/// | `[f32; 3]` | `vec3<f32>` |
/// | `[f32; 4]`, [`Color`] | `vec4<f32>` |
pub trait UniformValue {
    /// The WGSL type this value has, and its bytes. Only as many bytes as
    /// the type needs are used.
    #[doc(hidden)]
    fn to_uniform(&self) -> (UniformKind, [u8; 16]);
}

fn pack(kind: UniformKind, floats: &[f32]) -> (UniformKind, [u8; 16]) {
    let mut bytes = [0; 16];
    for (chunk, value) in bytes.chunks_exact_mut(4).zip(floats) {
        chunk.copy_from_slice(&value.to_ne_bytes());
    }
    (kind, bytes)
}

impl UniformValue for f32 {
    fn to_uniform(&self) -> (UniformKind, [u8; 16]) {
        pack(UniformKind::Float, &[*self])
    }
}

impl UniformValue for i32 {
    fn to_uniform(&self) -> (UniformKind, [u8; 16]) {
        let mut bytes = [0; 16];
        bytes[..4].copy_from_slice(&self.to_ne_bytes());
        (UniformKind::Int, bytes)
    }
}

impl UniformValue for u32 {
    fn to_uniform(&self) -> (UniformKind, [u8; 16]) {
        let mut bytes = [0; 16];
        bytes[..4].copy_from_slice(&self.to_ne_bytes());
        (UniformKind::Uint, bytes)
    }
}

impl UniformValue for [f32; 2] {
    fn to_uniform(&self) -> (UniformKind, [u8; 16]) {
        pack(UniformKind::Vec2, self)
    }
}

impl UniformValue for (f32, f32) {
    fn to_uniform(&self) -> (UniformKind, [u8; 16]) {
        pack(UniformKind::Vec2, &[self.0, self.1])
    }
}

impl UniformValue for [f32; 3] {
    fn to_uniform(&self) -> (UniformKind, [u8; 16]) {
        pack(UniformKind::Vec3, self)
    }
}

impl UniformValue for [f32; 4] {
    fn to_uniform(&self) -> (UniformKind, [u8; 16]) {
        pack(UniformKind::Vec4, self)
    }
}

impl UniformValue for Color {
    fn to_uniform(&self) -> (UniformKind, [u8; 16]) {
        pack(UniformKind::Vec4, &[self.r, self.g, self.b, self.a])
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
    /// The shader declares no texture with this name.
    UnknownTexture(String),
    /// The texture is declared as a different kind from the one sent,
    /// such as a cube image sent to a `texture_2d<f32>`.
    WrongTextureKind {
        name: String,
        sent: TextureKind,
        declared: TextureKind,
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

    #[test]
    fn values_are_packed_as_the_gpu_reads_them() {
        let (kind, bytes) = Color::rgba(1.0, 0.5, 0.25, 0.0).to_uniform();
        assert_eq!(kind, UniformKind::Vec4);
        assert_eq!(bytes[..4], 1.0f32.to_ne_bytes());
        assert_eq!(bytes[4..8], 0.5f32.to_ne_bytes());
        assert_eq!(bytes[8..12], 0.25f32.to_ne_bytes());

        let (kind, bytes) = (-3i32).to_uniform();
        assert_eq!(kind, UniformKind::Int);
        assert_eq!(bytes[..4], (-3i32).to_ne_bytes());

        assert_eq!((2.0f32, 3.0f32).to_uniform(), [2.0f32, 3.0].to_uniform());
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
