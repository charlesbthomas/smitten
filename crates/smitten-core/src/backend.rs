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
    BlendMode, Color, Cursor, Filter, GamepadAxis, GamepadButton, JoystickId, Key, MessageBoxKind,
    MouseButton, PowerInfo, StencilMode, TouchId, WindowState, Wrap,
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
    /// A finger touched the screen at `(x, y)` in pixels. `pressure` runs
    /// from 0.0 to 1.0, and is 1.0 on screens that cannot measure it.
    TouchPressed {
        id: TouchId,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        pressure: f32,
    },
    /// A finger on the screen moved to `(x, y)` in pixels, by `(dx, dy)`.
    TouchMoved {
        id: TouchId,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        pressure: f32,
    },
    /// A finger left the screen, or the system took the touch away.
    TouchReleased {
        id: TouchId,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        pressure: f32,
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
    /// The window started or stopped receiving the keyboard.
    Focus(bool),
    /// The pointer entered or left the window.
    MouseFocus(bool),
    /// The window was shown, or was hidden or minimized.
    Visible(bool),
    /// A file was dropped on the window at `(x, y)` in pixels. `path` is
    /// absolute.
    FileDropped {
        path: PathBuf,
        x: f32,
        y: f32,
    },
    /// A directory was dropped on the window at `(x, y)` in pixels.
    /// `path` is absolute.
    DirectoryDropped {
        path: PathBuf,
        x: f32,
        y: f32,
    },
    /// The system is running out of memory and the game should free what
    /// it can, or it may be closed.
    LowMemory,
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

/// A handle to a cursor made from an image, owned by a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CursorId(pub u32);

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

/// The shape of a texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextureKind {
    /// An ordinary picture.
    #[default]
    Flat,
    /// Several pictures of one size, picked by index.
    Array,
    /// Six square pictures that make the inside of a cube, picked by
    /// direction.
    Cube,
}

/// A way of storing pixels compressed in blocks of 4 by 4, which the GPU
/// reads without unpacking first. These are the BC formats, also called
/// DXT or S3TC, that desktop GPUs read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompressedFormat {
    /// Color with on-or-off alpha, 8 bytes a block. Also called DXT1.
    Bc1,
    /// Color with sharp alpha, 16 bytes a block. Also called DXT3.
    Bc2,
    /// Color with smooth alpha, 16 bytes a block. Also called DXT5.
    Bc3,
    /// One channel, red, 8 bytes a block.
    Bc4,
    /// Two channels, red and green, 16 bytes a block.
    Bc5,
    /// High quality color and alpha, 16 bytes a block.
    Bc7,
}

impl CompressedFormat {
    /// Every format, in declaration order.
    pub const ALL: [CompressedFormat; 6] = [
        CompressedFormat::Bc1,
        CompressedFormat::Bc2,
        CompressedFormat::Bc3,
        CompressedFormat::Bc4,
        CompressedFormat::Bc5,
        CompressedFormat::Bc7,
    ];

    /// How many bytes one 4 by 4 block takes.
    pub fn block_size(self) -> usize {
        match self {
            CompressedFormat::Bc1 | CompressedFormat::Bc4 => 8,
            _ => 16,
        }
    }

    /// How many bytes a `width` by `height` picture takes. A partial block
    /// at an edge takes a whole one.
    pub fn data_size(self, width: u32, height: u32) -> usize {
        let blocks = |pixels: u32| pixels.div_ceil(4).max(1) as usize;
        blocks(width) * blocks(height) * self.block_size()
    }
}

/// A texture a custom shader declares besides `image`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderTexture {
    pub name: String,
    pub kind: TextureKind,
}

/// What a backend reports about a shader it compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderInfo {
    pub id: ShaderId,
    /// The size in bytes of the shader's uniform data, or 0 if it has none.
    /// Every [`DrawCall`] using the shader must supply exactly this many.
    pub uniform_size: u32,
    pub fields: Vec<UniformField>,
    /// The textures the shader declares, in order. Every [`DrawCall`]
    /// using the shader binds this many from [`Frame::textures`].
    pub textures: Vec<ShaderTexture>,
}

/// A rectangle of pixels on a render target, measured from its top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scissor {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Which color channels drawing may change. The rest keep what the
/// target already holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorMask {
    pub r: bool,
    pub g: bool,
    pub b: bool,
    pub a: bool,
}

impl ColorMask {
    /// Every channel may change, as drawing starts out.
    pub const ALL: ColorMask = ColorMask {
        r: true,
        g: true,
        b: true,
        a: true,
    };
}

impl Default for ColorMask {
    fn default() -> Self {
        Self::ALL
    }
}

/// A texture sent to a custom shader, and how to sample it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BoundTexture {
    pub texture: TextureId,
    pub filter: Filter,
}

/// A run of triangles that share a texture and drawing state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawCall {
    /// The texture to sample, or `None` for plain vertex colors. Only an
    /// array texture with the built-in shader, or a flat one, can be drawn
    /// this way.
    pub texture: Option<TextureId>,
    /// Which picture of an array texture to draw. Ignored for others.
    pub layer: u32,
    pub filter: Filter,
    pub blend: BlendMode,
    /// Pixels outside this rectangle are left untouched. `None` draws
    /// anywhere on the target.
    pub scissor: Option<Scissor>,
    pub stencil: StencilMode,
    /// The value written by [`StencilMode::Write`] with a replace action,
    /// or compared against by [`StencilMode::Test`].
    pub stencil_value: u8,
    /// The color channels the draw may change.
    pub color_mask: ColorMask,
    /// Whether to draw only the edges of the triangles, one pixel wide,
    /// for debugging.
    pub wireframe: bool,
    /// The custom shader to draw with, or `None` for the built-in one.
    pub shader: Option<ShaderId>,
    /// Where the shader's uniform data starts in [`Frame::uniforms`]. Its
    /// length is the shader's [`ShaderInfo::uniform_size`].
    pub uniform_offset: u32,
    /// Where the textures for the custom shader's own declarations start
    /// in [`Frame::textures`]. There are as many as
    /// [`ShaderInfo::textures`] lists.
    pub texture_offset: u32,
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
    /// Textures for the calls whose custom shader declares its own. `None`
    /// is one the game has not sent, which reads as white.
    pub textures: &'a [Option<BoundTexture>],
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

    /// As `create_texture`, with a full chain of mipmaps made from the
    /// image: smaller copies the GPU uses when the image is drawn small.
    /// They start out blended with [`Filter::Linear`] and a sharpness of 0,
    /// and `update_texture` makes them again.
    fn create_mipmapped_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<TextureId, String>;

    /// Sets how a texture with mipmaps blends between them, or `None` to
    /// draw only the full-size image. Positive `sharpness` chooses larger
    /// mipmaps than the GPU would, and negative smaller ones. Does nothing
    /// to a texture without mipmaps.
    fn set_texture_mipmap_filter(
        &mut self,
        texture: TextureId,
        filter: Option<Filter>,
        sharpness: f32,
    );

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

    /// Whether the GPU can read textures stored in `format`.
    fn supports_compressed_format(&self, format: CompressedFormat) -> bool;

    /// Creates a texture from compressed data, with one entry of `levels`
    /// for the full-size picture and one for each mipmap after it, each
    /// half the size of the one before and exactly
    /// [`CompressedFormat::data_size`] long. It cannot be updated. With
    /// more than one level, its mipmaps start out blended as those of
    /// `create_mipmapped_texture` do.
    fn create_compressed_texture(
        &mut self,
        format: CompressedFormat,
        width: u32,
        height: u32,
        levels: &[&[u8]],
    ) -> Result<TextureId, String>;

    /// Creates an array or cube texture from `layers`, each laid out as in
    /// `create_texture` and all `width` by `height` pixels. A cube has six
    /// square layers, in the order +x, -x, +y, -y, +z, -z. A
    /// [`TextureKind::Flat`] texture has one layer.
    fn create_layered_texture(
        &mut self,
        kind: TextureKind,
        width: u32,
        height: u32,
        layers: &[&[u8]],
    ) -> Result<TextureId, String>;

    /// Creates a texture that can be drawn to by naming it as a
    /// [`Pass::target`], and sampled like any other. It starts out
    /// transparent, and keeps its contents between frames.
    fn create_render_target(&mut self, width: u32, height: u32) -> Result<TextureId, String>;

    /// As `create_render_target`, with a full chain of mipmaps that start
    /// out blended as those of `create_mipmapped_texture` do. With
    /// `automatic`, the mipmaps are made again whenever a frame has drawn
    /// to the target; otherwise only by `generate_mipmaps`.
    fn create_mipmapped_render_target(
        &mut self,
        width: u32,
        height: u32,
        automatic: bool,
    ) -> Result<TextureId, String>;

    /// Makes a render target's mipmaps again from what it holds. Draws
    /// still waiting in a frame are not included, so the caller renders
    /// them first.
    fn generate_mipmaps(&mut self, texture: TextureId) -> Result<(), String>;

    /// Sets what a texture shows beyond its edges, along each axis. It
    /// applies to everything drawn with the texture from this frame on.
    fn set_texture_wrap(&mut self, texture: TextureId, horizontal: Wrap, vertical: Wrap);

    /// Frees a texture. Its handle must not be used afterwards.
    fn destroy_texture(&mut self, texture: TextureId);

    /// Roughly how much GPU memory the textures that have not been
    /// destroyed take, in bytes.
    fn texture_memory(&self) -> u64;

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

    /// Makes a cursor from an image. `rgba` is laid out as in
    /// `create_texture`, and `(hot_x, hot_y)` is the pixel that points,
    /// which must lie inside the image.
    fn create_cursor(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        hot_x: u32,
        hot_y: u32,
    ) -> Result<CursorId, String>;

    /// Shows a cursor from `create_cursor` while the pointer is over the
    /// window. A later `set_cursor` replaces it.
    fn set_image_cursor(&mut self, cursor: CursorId) -> Result<(), String>;

    /// Frees a cursor from `create_cursor`. It is not in use: the caller
    /// has switched to another cursor first.
    fn destroy_cursor(&mut self, cursor: CursorId);

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

    /// Whether the gamepad `id` has rumble motors.
    fn gamepad_can_rumble(&self, id: JoystickId) -> bool;

    /// Runs the gamepad's low and high frequency motors at strengths from
    /// 0 to 1 for `seconds`, replacing any rumble in progress. Zero
    /// strengths stop it. Returns whether the gamepad took the request.
    fn rumble_gamepad(&mut self, id: JoystickId, low: f32, high: f32, seconds: f32) -> bool;

    /// Sets the window title.
    /// The window's title.
    fn title(&self) -> String;

    /// Resizes the window to `width` by `height` units. The backend
    /// applies the display's scale, as it does for the size in
    /// [`Config`](crate::Config).
    fn set_window_size(&mut self, width: u32, height: u32) -> Result<(), String>;

    /// Where the window's top-left corner is on the desktop.
    fn window_position(&self) -> (i32, i32);

    fn set_window_position(&mut self, x: i32, y: i32);

    fn minimize(&mut self);

    fn maximize(&mut self);

    /// Undoes minimizing or maximizing.
    fn restore(&mut self);

    fn window_state(&self) -> WindowState;

    /// Turns waiting for the display's refresh on or off.
    fn set_vsync(&mut self, vsync: bool) -> Result<(), String>;

    fn vsync(&self) -> bool;

    /// The size in units of the desktop the window is on.
    fn desktop_size(&self) -> (u32, u32);

    /// How many displays are connected.
    fn display_count(&self) -> u32;

    /// Shows a message with an OK button and waits for it to be closed.
    fn show_message_box(
        &mut self,
        kind: MessageBoxKind,
        title: &str,
        message: &str,
    ) -> Result<(), String>;

    /// Shows a message with a button for each of `buttons`, in order
    /// from left to right, and waits for one to be chosen. Enter chooses
    /// the first button and Escape the last.
    ///
    /// Returns the index of the chosen button, or `None` if the box was
    /// closed without choosing one. `buttons` is never empty.
    fn show_message_box_with_buttons(
        &mut self,
        kind: MessageBoxKind,
        title: &str,
        message: &str,
        buttons: &[&str],
    ) -> Result<Option<usize>, String>;

    /// Asks for the user's attention, for instance by flashing the
    /// taskbar. `continuous` keeps asking until the window gets focus.
    fn request_attention(&mut self, continuous: bool);

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
