//! The boundary between Smitten's public API and a platform.
//!
//! Everything a game sees is written once against [`Backend`]. Each platform
//! family provides one implementation: SDL3 covers desktop, mobile and
//! consoles, and a web backend can be added later without touching the
//! public modules. No type from a platform library may cross this boundary.

pub(crate) mod sdl;

use crate::graphics::Color;
use crate::keyboard::Key;

pub(crate) enum BackendEvent {
    Quit,
    Resized { width: u32, height: u32 },
    KeyDown { key: Key, is_repeat: bool },
    KeyUp { key: Key },
}

pub(crate) trait Backend {
    /// Returns the next pending event, if any, without blocking.
    fn poll_event(&mut self) -> Option<BackendEvent>;

    /// Clears the frame to `clear` and shows it.
    fn present(&mut self, clear: Color);

    /// Monotonic time in nanoseconds.
    fn ticks_ns(&self) -> u64;

    /// Drawable size in pixels.
    fn size_in_pixels(&self) -> (u32, u32);

    fn set_title(&mut self, title: &str);
}
