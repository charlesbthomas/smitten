//! The game window, the equivalent of `love.window`.

use std::path::Path;

use crate::context;
use crate::graphics::{ImageData, ImageError};
use crate::{DisplayOrientation, MessageBoxKind};

/// Sets the window title.
pub fn set_title(title: &str) {
    context::with(|ctx| ctx.backend.set_title(title));
}

/// Returns the window title.
pub fn title() -> String {
    context::with(|ctx| ctx.backend.title())
}

/// Resizes the window to `width` by `height` units, the part of
/// `love.window.setMode` that games use most. This calls
/// [`crate::Game::resized`] like any other change of size.
///
/// If the platform refuses, a message is printed and nothing changes. A
/// fullscreen or maximized window keeps its size.
pub fn set_size(width: u32, height: u32) {
    context::with(|ctx| {
        if let Err(message) = ctx.backend.set_window_size(width, height) {
            eprintln!("smitten: could not resize the window: {message}");
        }
        measure(ctx);
    });
}

/// Takes the window's size afresh after changing it, so that sizes read
/// before the next frame are already the new ones.
fn measure(ctx: &mut context::Context) {
    ctx.gfx
        .screen
        .update(ctx.backend.size_in_pixels(), ctx.backend.dpi_scale());
}

/// Where the window's top-left corner is on the desktop, as `(x, y)` in
/// the desktop's own coordinates.
pub fn position() -> (i32, i32) {
    context::with(|ctx| ctx.backend.window_position())
}

/// Moves the window's top-left corner to `(x, y)` on the desktop. Some
/// window systems, Wayland among them, do not let programs place their
/// windows and ignore this.
pub fn set_position(x: i32, y: i32) {
    context::with(|ctx| ctx.backend.set_window_position(x, y));
}

/// Shrinks the window to the taskbar or Dock.
pub fn minimize() {
    context::with(|ctx| ctx.backend.minimize());
}

/// Enlarges the window to fill the desktop. Only a resizable window
/// (see [`crate::Config::resizable`]) can be maximized.
pub fn maximize() {
    context::with(|ctx| {
        ctx.backend.maximize();
        measure(ctx);
    });
}

/// Puts a minimized or maximized window back as it was.
pub fn restore() {
    context::with(|ctx| {
        ctx.backend.restore();
        measure(ctx);
    });
}

/// Whether the window is minimized.
pub fn is_minimized() -> bool {
    context::with(|ctx| ctx.backend.window_state().minimized)
}

/// Whether the window is maximized.
pub fn is_maximized() -> bool {
    context::with(|ctx| ctx.backend.window_state().maximized)
}

/// Whether the window is receiving the keyboard. See
/// [`crate::Game::focus`] to be told when this changes.
pub fn has_focus() -> bool {
    context::with(|ctx| ctx.backend.window_state().focused)
}

/// Whether the pointer is over the window.
pub fn has_mouse_focus() -> bool {
    context::with(|ctx| ctx.backend.window_state().mouse_focused)
}

/// Whether the window is on screen: neither hidden nor minimized.
pub fn is_visible() -> bool {
    context::with(|ctx| ctx.backend.window_state().visible)
}

/// Turns waiting for the display's refresh on or off. On, the game runs
/// at the display's rate without tearing. To choose from the start, set
/// [`crate::Config::vsync`].
///
/// If the display cannot do what was asked, a message is printed and
/// nothing changes.
pub fn set_vsync(vsync: bool) {
    context::with(|ctx| {
        if let Err(message) = ctx.backend.set_vsync(vsync) {
            eprintln!("smitten: could not change vsync: {message}");
        }
    });
}

/// Whether the game waits for the display's refresh.
pub fn vsync() -> bool {
    context::with(|ctx| ctx.backend.vsync())
}

/// The size in units of the desktop the window is on, as
/// `(width, height)`.
pub fn desktop_dimensions() -> (u32, u32) {
    context::with(|ctx| ctx.backend.desktop_size())
}

/// How many displays are connected.
pub fn display_count() -> u32 {
    context::with(|ctx| ctx.backend.display_count())
}

/// Shows a message with an OK button and waits until the player closes
/// it. The game stands still meanwhile.
///
/// Returns whether the message could be shown.
pub fn show_message_box(title: &str, message: &str, kind: MessageBoxKind) -> bool {
    context::with(
        |ctx| match ctx.backend.show_message_box(kind, title, message) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("smitten: could not show a message box: {error}");
                false
            }
        },
    )
}

/// Shows a message with a button for each label in `buttons`, from left
/// to right, and waits until the player picks one. The game stands still
/// meanwhile. Enter picks the first button and Escape the last, so put
/// the safe choice last.
///
/// Returns the index of the button picked, or `None` if the box was closed
/// some other way, could not be shown, or `buttons` is empty.
///
/// ```no_run
/// use smitten::{MessageBoxKind, event, window};
///
/// let choice = window::show_message_box_with_buttons(
///     "Quit?",
///     "Your progress since the last save will be lost.",
///     MessageBoxKind::Warning,
///     &["Quit", "Keep playing"],
/// );
/// if choice == Some(0) {
///     event::quit();
/// }
/// ```
pub fn show_message_box_with_buttons(
    title: &str,
    message: &str,
    kind: MessageBoxKind,
    buttons: &[&str],
) -> Option<usize> {
    if buttons.is_empty() {
        eprintln!("smitten: a message box needs at least one button");
        return None;
    }
    context::with(|ctx| {
        match ctx
            .backend
            .show_message_box_with_buttons(kind, title, message, buttons)
        {
            Ok(choice) => choice,
            Err(error) => {
                eprintln!("smitten: could not show a message box: {error}");
                None
            }
        }
    })
}

/// Asks for the player's attention when the window is in the background,
/// usually by flashing its taskbar button. `continuous` keeps asking
/// until the window gets focus.
pub fn request_attention(continuous: bool) {
    context::with(|ctx| ctx.backend.request_attention(continuous));
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
        measure(ctx);
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
    context::with(|ctx| {
        ctx.backend.set_icon(width, height, rgba)?;
        ctx.icon = Some(ImageData::from_rgba(width, height, rgba.to_vec()));
        Ok(())
    })
    .map_err(ImageError::Upload)
}

/// The window's icon as last set, by [`set_icon`], its siblings or
/// [`crate::Config::icon`], the equivalent of `love.window.getIcon`.
/// `None` if the game has not set one.
pub fn icon() -> Option<ImageData> {
    context::with(|ctx| ctx.icon.clone())
}

/// The part of the window that nothing covers, as `(x, y, width,
/// height)` in units from its top-left, the equivalent of
/// `love.window.getSafeArea`.
///
/// On a phone with a notch, rounded corners or an on-screen home bar,
/// keep buttons and text inside this. Elsewhere it is the whole window,
/// the same size as [`dimensions`]. It can change when the window is
/// resized or the display rotates.
pub fn safe_area() -> (f32, f32, f32, f32) {
    context::with(|ctx| {
        let (x, y, width, height) = ctx.backend.safe_area();
        let dpi = ctx.gfx.screen.dpi;
        (
            x as f32 / dpi,
            y as f32 / dpi,
            width as f32 / dpi,
            height as f32 / dpi,
        )
    })
}

/// Which way up the display the window is on is, the equivalent of
/// `love.window.getDisplayOrientation`. A desktop display reports the
/// orientation its shape suggests, or [`DisplayOrientation::Unknown`]
/// where the system cannot tell. See
/// [`crate::Game::display_rotated`] to be told when it changes.
pub fn display_orientation() -> DisplayOrientation {
    context::with(|ctx| ctx.backend.display_orientation())
}

/// Converts a length in units to pixels, the equivalent of
/// `love.window.toPixels`: it multiplies by [`dpi_scale`].
pub fn to_pixels(units: f32) -> f32 {
    units * dpi_scale()
}

/// Converts a length in pixels to units, the equivalent of
/// `love.window.fromPixels`: it divides by [`dpi_scale`].
pub fn from_pixels(pixels: f32) -> f32 {
    pixels / dpi_scale()
}
