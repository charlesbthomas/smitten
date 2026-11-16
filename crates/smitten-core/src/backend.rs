//! The boundary between Smitten's public API and a platform.
//!
//! Everything a game sees is written once against [`Backend`]. Each platform
//! family provides one implementation in its own crate: `smitten-sdl` covers
//! desktop, mobile and consoles, and a web backend can be added later
//! without touching the public modules. No type from a platform library may
//! cross this boundary.
//!
//! Games never use this module. It is public only so backend crates can
//! implement it.

use std::path::PathBuf;

use crate::{
    BlendMode, Color, Cursor, Filter, GamepadAxis, GamepadButton, JoystickId, Key, MouseButton,
    PowerInfo, StencilMode, Wrap,
};

/// Something that happened on the platform since the last poll.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendEvent {
    /// The user or the system asked the game to close.
    Quit,
    /// The drawable size changed, in pixels.
    Resized {
        width: u32,
        height: u32,
    },
    KeyDown {
        key: Key,
        is_repeat: bool,
    },
    KeyUp {
        key: Key,
    },
    /// Text the user typed, after keyboard layout and input methods.
    TextInput(String),
    /// The pointer moved to `(x, y)` in pixels, by `(dx, dy)`.
    MouseMoved {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
    },
    /// `presses` is 1 for a single click, 2 for a double click, and so on.
    MousePressed {
        x: f32,
        y: f32,
        button: MouseButton,
        presses: u32,
    },
    MouseReleased {
        x: f32,
        y: f32,
        button: MouseButton,
        presses: u32,
    },
    /// The wheel moved. Positive `dy` is away from the user.
    WheelMoved {
        dx: f32,
        dy: f32,
    },
    JoystickAdded {
        id: JoystickId,
        name: String,
    },
    JoystickRemoved {
        id: JoystickId,
    },
    GamepadPressed {
        id: JoystickId,
        button: GamepadButton,
    },
    GamepadReleased {
        id: JoystickId,
        button: GamepadButton,
    },
    GamepadAxis {
        id: JoystickId,
        axis: GamepadAxis,
        value: f32,
    },
}

/// A corner of a triangle, in pixels from the top-left of the drawable.
///
/// The layout is part of the contract: backends upload slices of this type
/// to the GPU unchanged.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: [f32; 2],
    /// Texture coordinate from 0.0 to 1.0.
    pub uv: [f32; 2],
    /// Multiplied with the texture color. Alpha is not premultiplied.
    pub color: [f32; 4],
}

/// A handle to a texture owned by a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureId(pub u32);

/// A handle to a custom shader owned by a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShaderId(pub u32);

/// The type of a value a game can send to a shader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UniformKind {
    /// `f32`
    Float,
    /// `i32`
    Int,
    /// `u32`
    Uint,
    /// `vec2<f32>`
    Vec2,
    /// `vec3<f32>`
    Vec3,
    /// `vec4<f32>`
    Vec4,
}

impl UniformKind {
    /// How many bytes a value of this type occupies.
    pub fn size(self) -> usize {
        match self {
            UniformKind::Float | UniformKind::Int | UniformKind::Uint => 4,
            UniformKind::Vec2 => 8,
            UniformKind::Vec3 => 12,
            UniformKind::Vec4 => 16,
        }
    }
}

/// One named value in a shader's `Params` struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniformField {
    pub name: String,
    /// Where the value starts in the shader's uniform bytes.
    pub offset: u32,
    /// `None` for a type that cannot be sent yet, such as a matrix.
    pub kind: Option<UniformKind>,
}

/// What a backend reports about a shader it compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderInfo {
    pub id: ShaderId,
    /// The size in bytes of the shader's uniform data, or 0 if it has none.
    /// Every [`DrawCall`] using the shader must supply exactly this many.
    pub uniform_size: u32,
    pub fields: Vec<UniformField>,
}

/// A rectangle of pixels on a render target, measured from its top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scissor {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A run of triangles that share a texture and drawing state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawCall {
    /// The texture to sample, or `None` for plain vertex colors.
    pub texture: Option<TextureId>,
    pub filter: Filter,
    pub blend: BlendMode,
    /// Pixels outside this rectangle are left untouched. `None` draws
    /// anywhere on the target.
    pub scissor: Option<Scissor>,
    pub stencil: StencilMode,
    /// The value written by [`StencilMode::Write`] with a replace action,
    /// or compared against by [`StencilMode::Test`].
    pub stencil_value: u8,
    /// The custom shader to draw with, or `None` for the built-in one.
    pub shader: Option<ShaderId>,
    /// Where the shader's uniform data starts in [`Frame::uniforms`]. Its
    /// length is the shader's [`ShaderInfo::uniform_size`].
    pub uniform_offset: u32,
    /// Index into [`Frame::vertices`] of the first vertex.
    pub first_vertex: u32,
    /// A multiple of three, since every three vertices form a triangle.
    pub vertex_count: u32,
}

/// A run of draw calls that share a render target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pass {
    /// A texture from [`Backend::create_render_target`], or `None` for
    /// the screen.
    pub target: Option<TextureId>,
    /// The color to fill the target with first. `None` keeps what the
    /// target already holds and draws on top of it.
    pub clear: Option<Color>,
    /// Whether to reset the target's stencil values to zero first. Each
    /// target has its own stencil values, kept until cleared.
    pub clear_stencil: bool,
    /// Index into [`Frame::calls`] of the first call.
    pub first_call: u32,
    pub call_count: u32,
}

/// Everything needed to draw one frame.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    /// In pixels of the target they are drawn to.
    pub vertices: &'a [Vertex],
    pub calls: &'a [DrawCall],
    /// Uniform data for the calls that use a custom shader.
    pub uniforms: &'a [u8],
    /// Run in order. A later pass may sample a target drawn by an
    /// earlier one.
    pub passes: &'a [Pass],
}

/// The sample rate Smitten mixes audio at. Backends convert from this to
/// whatever the output device wants.
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;

/// Fills a buffer with the next audio to play: interleaved stereo samples
/// (left, right, left, right...) from -1.0 to 1.0 at [`AUDIO_SAMPLE_RATE`].
///
/// Backends call this from an audio thread, so it must not block for long.
pub type AudioFill = Box<dyn FnMut(&mut [f32]) + Send>;

/// What a platform must provide for Smitten to run on it.
pub trait Backend {
    /// Returns the next pending event, if any, without blocking.
    fn poll_event(&mut self) -> Option<BackendEvent>;

    /// Uploads an image and returns a handle to it.
    ///
    /// `rgba` holds `width * height` pixels, four bytes each, in rows from
    /// the top, with alpha not premultiplied.
    fn create_texture(&mut self, width: u32, height: u32, rgba: &[u8])
    -> Result<TextureId, String>;

    /// Replaces the `width` by `height` region of a texture whose top-left
    /// corner is at `(x, y)`. `rgba` is laid out as in `create_texture`.
    fn update_texture(
        &mut self,
        texture: TextureId,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), String>;

    /// Creates a texture that can be drawn to by naming it as a
    /// [`Pass::target`], and sampled like any other. It starts out
    /// transparent, and keeps its contents between frames.
    fn create_render_target(&mut self, width: u32, height: u32) -> Result<TextureId, String>;

    /// Sets what a texture shows beyond its edges, along each axis. It
    /// applies to everything drawn with the texture from this frame on.
    fn set_texture_wrap(&mut self, texture: TextureId, horizontal: Wrap, vertical: Wrap);

    /// Frees a texture. Its handle must not be used afterwards.
    fn destroy_texture(&mut self, texture: TextureId);

    /// Compiles a custom fragment shader written in WGSL.
    ///
    /// `source` must define
    /// `fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32>`,
    /// which returns the color of one pixel. It may read the texture being
    /// drawn through `image` and `image_sampler`, and values sent by the
    /// game through `params` if it declares a `struct Params`.
    ///
    /// The error is a message for the game's author, with line numbers
    /// that refer to `source`.
    fn create_shader(&mut self, source: &str) -> Result<ShaderInfo, String>;

    /// Frees a shader. Its handle must not be used afterwards.
    fn destroy_shader(&mut self, shader: ShaderId);

    /// Draws `frame` and shows the screen.
    fn present(&mut self, frame: &Frame);

    /// Draws the passes of `frame` that go to render targets, without
    /// showing anything. Passes that go to the screen are left out.
    fn render_offscreen(&mut self, frame: &Frame);

    /// Copies a texture's pixels back from the GPU as
    /// `(width, height, rgba)`: four bytes a pixel, in rows from the top.
    /// This waits for the GPU, so it is slow.
    fn read_texture(&mut self, texture: TextureId) -> Result<(u32, u32, Vec<u8>), String>;

    /// Monotonic time in nanoseconds.
    fn ticks_ns(&self) -> u64;

    /// Drawable size in pixels.
    fn size_in_pixels(&self) -> (u32, u32);

    /// Opens the default audio output and starts calling `fill` whenever
    /// it needs more audio. Called at most once.
    ///
    /// An error means the game runs without sound, which is not fatal.
    fn start_audio(&mut self, fill: AudioFill) -> Result<(), String>;

    /// Returns a directory the game may write to, creating it if needed.
    ///
    /// `identity` names the game. The same identity always gives the same
    /// directory on a given machine and user account.
    fn save_directory(&mut self, identity: &str) -> Result<PathBuf, String>;

    /// Shows or hides the mouse cursor while it is over the window.
    fn set_mouse_visible(&mut self, visible: bool);

    /// Chooses the cursor shown while the pointer is over the window.
    fn set_cursor(&mut self, cursor: Cursor) -> Result<(), String>;

    /// Hides the cursor and reports motion without limit, for a
    /// first-person camera or similar. The pointer stays in the window.
    fn set_relative_mouse(&mut self, relative: bool) -> Result<(), String>;

    /// Keeps the pointer inside the window while the window has focus.
    fn set_mouse_grabbed(&mut self, grabbed: bool) -> Result<(), String>;

    /// Moves the pointer to `(x, y)` in pixels from the window's top-left
    /// corner.
    fn warp_mouse(&mut self, x: f32, y: f32);

    /// Puts `text` on the system clipboard.
    fn set_clipboard_text(&mut self, text: &str) -> Result<(), String>;

    /// The text on the system clipboard, or an empty string.
    fn clipboard_text(&mut self) -> String;

    /// Opens a web address or file with the program the system uses for
    /// it, usually the web browser.
    fn open_url(&mut self, url: &str) -> Result<(), String>;

    /// The state of the battery, if there is one.
    fn power_info(&mut self) -> PowerInfo;

    /// Sets the window title.
    fn set_title(&mut self, title: &str);

    /// Makes the window fill the whole screen, or puts it back. A
    /// [`BackendEvent::Resized`] follows if the drawable size changed.
    fn set_fullscreen(&mut self, fullscreen: bool) -> Result<(), String>;

    /// Whether the window currently fills the whole screen.
    fn is_fullscreen(&self) -> bool;

    /// Sets the window's icon. `rgba` is laid out as in `create_texture`.
    fn set_icon(&mut self, width: u32, height: u32, rgba: &[u8]) -> Result<(), String>;

    /// How much larger than normal the user's display settings ask things
    /// to be drawn: 1.0 on an ordinary display, 2.0 on one with twice the
    /// pixels in each direction.
    fn dpi_scale(&self) -> f32;
}
