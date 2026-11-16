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

use crate::{Color, Filter, GamepadAxis, GamepadButton, JoystickId, Key, MouseButton};

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

/// A run of triangles that share a texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawCall {
    /// The texture to sample, or `None` for plain vertex colors.
    pub texture: Option<TextureId>,
    pub filter: Filter,
    /// Index into [`Frame::vertices`] of the first vertex.
    pub first_vertex: u32,
    /// A multiple of three, since every three vertices form a triangle.
    pub vertex_count: u32,
}

/// Everything needed to draw one frame.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    pub clear: Color,
    pub vertices: &'a [Vertex],
    /// Drawn in order, so later calls appear on top.
    pub calls: &'a [DrawCall],
}

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

    /// Frees a texture. Its handle must not be used afterwards.
    fn destroy_texture(&mut self, texture: TextureId);

    /// Draws `frame` and shows it.
    fn present(&mut self, frame: &Frame);

    /// Monotonic time in nanoseconds.
    fn ticks_ns(&self) -> u64;

    /// Drawable size in pixels.
    fn size_in_pixels(&self) -> (u32, u32);

    /// Returns a directory the game may write to, creating it if needed.
    ///
    /// `identity` names the game. The same identity always gives the same
    /// directory on a given machine and user account.
    fn save_directory(&mut self, identity: &str) -> Result<PathBuf, String>;

    /// Shows or hides the mouse cursor while it is over the window.
    fn set_mouse_visible(&mut self, visible: bool);

    /// Sets the window title.
    fn set_title(&mut self, title: &str);
}
