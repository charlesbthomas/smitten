//! The game window, the equivalent of `love.window`.

use std::path::Path;

use crate::context;
use crate::graphics::ImageError;

/// Sets the window title.
pub fn set_title(title: &str) {
    context::with(|ctx| ctx.backend.set_title(title));
}

/// Makes the game fill the whole screen, or puts it back in a window.
///
/// Fullscreen uses the desktop's own resolution, so switching is quick and
/// other windows are left alone. The drawable size changes, which calls
/// [`crate::Game::resized`]. To start in fullscreen, set
/// [`crate::Config::fullscreen`] instead.
///
/// If the platform refuses, a message is printed and nothing changes.
pub fn set_fullscreen(fullscreen: bool) {
    context::with(|ctx| {
        if let Err(message) = ctx.backend.set_fullscreen(fullscreen) {
            eprintln!("smitten: could not change fullscreen: {message}");
        }
    });
}

/// Whether the game currently fills the whole screen.
pub fn fullscreen() -> bool {
    context::with(|ctx| ctx.backend.is_fullscreen())
}

/// How many pixels make up one unit, the equivalent of
/// `love.window.getDPIScale`.
///
/// It is 1.0 on an ordinary display and 2.0 on one with twice the pixels
/// in each direction, such as a Retina display, or a Windows display set
/// to 200%. Smitten applies it for you: sizes and positions are in units,
/// and drawing is simply sharper where this is higher. It is always 1.0
/// when [`crate::Config::dpi_scaling`] is off.
pub fn dpi_scale() -> f32 {
    context::with(|ctx| ctx.gfx.screen.dpi)
}

/// The window's size in units as `(width, height)`.
///
/// This is the same as [`crate::graphics::dimensions`] for the screen,
/// except in a game with a design size, where that is always the design
/// size and this is the real window.
pub fn dimensions() -> (u32, u32) {
    context::with(|ctx| ctx.gfx.screen.window_size())
}

/// Sets the window's icon from an image file (PNG or JPEG), looked up like
/// any other asset. To set it from the start, use [`crate::Config::icon`].
///
/// Panics with the file name if it cannot be loaded. Use [`try_set_icon`]
/// to handle the failure instead.
///
/// Some platforms take a game's icon from somewhere else: macOS shows the
/// application bundle's icon in the Dock, and Windows shows the icon built
/// into the executable in Explorer.
pub fn set_icon(path: impl AsRef<Path>) {
    try_set_icon(path).unwrap_or_else(|error| panic!("{error}"));
}

/// Sets the window's icon from an image file, returning an error if it
/// fails.
pub fn try_set_icon(path: impl AsRef<Path>) -> Result<(), ImageError> {
    let (width, height, rgba) = crate::graphics::image::load_rgba(path.as_ref())?;
    set_icon_rgba(width, height, &rgba)
}

/// Sets the window's icon from raw pixels: `width * height` of them, four
/// bytes each (red, green, blue, alpha), in rows from the top.
pub fn set_icon_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<(), ImageError> {
    context::with(|ctx| ctx.backend.set_icon(width, height, rgba)).map_err(ImageError::Upload)
}
