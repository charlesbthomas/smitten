//! The game window, the equivalent of `love.window`.

use std::path::Path;

use crate::context;
use crate::graphics::{ImageData, ImageError};
use crate::{DisplayOrientation, FullscreenType, MessageBoxKind};

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

/// The size in units of `display`'s desktop, the equivalent of
/// `love.window.getDesktopDimensions(displayindex)`. Displays are counted
/// from 0, where LÖVE counts from 1. `None` if there is no such display.
pub fn desktop_dimensions_of(display: u32) -> Option<(u32, u32)> {
    context::with(|ctx| ctx.backend.display_desktop_size(display))
}

/// How many displays are connected.
pub fn display_count() -> u32 {
    context::with(|ctx| ctx.backend.display_count())
}

/// Which display the window is on, counted from 0 in the order the system
/// lists them (LÖVE counts from 1). A window across two displays is on the
/// one holding most of it.
pub fn display() -> u32 {
    context::with(|ctx| ctx.backend.window_display())
}

/// The name the system gives `display`, such as the monitor's model, the
/// equivalent of `love.window.getDisplayName`. Displays are counted from
/// 0. `None` if there is no such display.
pub fn display_name(display: u32) -> Option<String> {
    context::with(|ctx| ctx.backend.display_name(display))
}

/// What [`dpi_scale`] would be for a window on `display`, counted from 0.
/// `None` if there is no such display. It is always 1.0 when
/// [`crate::Config::dpi_scaling`] is off.
pub fn display_dpi_scale(display: u32) -> Option<f32> {
    context::with(|ctx| {
        if ctx.gfx.screen.dpi_scaling() {
            ctx.backend.display_dpi_scale(display)
        } else {
            ctx.backend.display_bounds(display).map(|_| 1.0)
        }
    })
}

/// Where the window's top-left corner is, as `(x, y, display)`: the
/// display it is on, and the position from that display's top-left. This
/// is the three values of `love.window.getPosition`, with the display
/// counted from 0.
pub fn position_on_display() -> (i32, i32, u32) {
    context::with(|ctx| {
        let (x, y) = ctx.backend.window_position();
        let display = ctx.backend.window_display();
        let (left, top, _, _) = ctx.backend.display_bounds(display).unwrap_or_default();
        (x - left, y - top, display)
    })
}

/// Moves the window's top-left corner to `(x, y)` from the top-left of
/// `display`, counted from 0, the three-argument form of
/// `love.window.setPosition`. A display that is not connected is taken as
/// the first.
pub fn set_position_on_display(x: i32, y: i32, display: u32) {
    context::with(|ctx| place(ctx, x, y, display));
}

fn place(ctx: &mut context::Context, x: i32, y: i32, display: u32) {
    let bounds = ctx
        .backend
        .display_bounds(display)
        .or_else(|| ctx.backend.display_bounds(0));
    let (left, top, _, _) = bounds.unwrap_or_default();
    ctx.backend.set_window_position(left + x, top + y);
}

/// Hides the window, the equivalent of `love.window.close`. The game keeps
/// running: callbacks are still called and canvases are still drawn, but
/// nothing reaches the screen. [`set_mode`] or [`open`] shows it again.
///
/// Closing the window does not end the game. Use [`crate::event::quit`]
/// for that.
pub fn close() {
    context::with(|ctx| {
        ctx.backend.set_window_shown(false);
        ctx.window_closed = true;
    });
}

/// Shows the window again after [`close`].
pub fn open() {
    context::with(|ctx| {
        if ctx.window_closed {
            ctx.backend.set_window_shown(true);
            ctx.window_closed = false;
            measure(ctx);
        }
    });
}

/// Whether the window is open, the equivalent of `love.window.isOpen`. It
/// is only closed after [`close`].
pub fn is_open() -> bool {
    context::with(|ctx| !ctx.window_closed)
}

/// Lets the display turn off or start its screen saver while the game
/// runs, the equivalent of `love.window.setDisplaySleepEnabled`. Games keep
/// it awake by default, as in LÖVE.
pub fn set_display_sleep_enabled(enabled: bool) {
    context::with(|ctx| ctx.backend.set_display_sleep(enabled));
}

/// Whether the display may sleep while the game runs.
pub fn is_display_sleep_enabled() -> bool {
    context::with(|ctx| ctx.backend.display_sleep_enabled())
}

/// The window's settings, the flags table of `love.window.setMode` and
/// `love.window.getMode`. Start from [`WindowMode::default`], which matches
/// [`crate::Config`]'s defaults, or from [`mode`] to keep what the window
/// has now.
///
/// Sizes and positions are in units, as everywhere else. Displays are
/// counted from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowMode {
    /// Whether the game fills the whole screen.
    pub fullscreen: bool,
    /// How it fills the screen when [`WindowMode::fullscreen`] is on.
    pub fullscreen_type: FullscreenType,
    /// Whether presenting waits for the display's refresh.
    pub vsync: bool,
    /// Whether the user can resize the window.
    pub resizable: bool,
    /// Whether the window has no title bar or frame.
    pub borderless: bool,
    /// Whether to move the window to the middle of its display when no
    /// [`WindowMode::position`] is given.
    pub centered: bool,
    /// The display the window goes on.
    pub display: u32,
    /// The smallest width the user can shrink the window to.
    pub min_width: u32,
    /// The smallest height the user can shrink the window to.
    pub min_height: u32,
    /// Where the window's top-left corner goes, from the top-left of
    /// [`WindowMode::display`]. When `None`, [`WindowMode::centered`]
    /// decides.
    pub position: Option<(i32, i32)>,
    /// How many samples each pixel of the screen holds, 0 or 1 for none.
    /// The GPU may give fewer; [`mode`] reports how many it gave. See
    /// [`set_msaa`].
    pub msaa: u32,
}

impl Default for WindowMode {
    fn default() -> Self {
        Self {
            fullscreen: false,
            fullscreen_type: FullscreenType::Desktop,
            vsync: true,
            resizable: false,
            borderless: false,
            centered: true,
            display: 0,
            min_width: 1,
            min_height: 1,
            position: None,
            msaa: 0,
        }
    }
}

/// The window's size in units and its settings, the equivalent of
/// `love.window.getMode`.
///
/// The settings hold the window's current position, so passing them back
/// to [`set_mode`] leaves the window where it is. They report
/// [`WindowMode::centered`] as on, so clearing [`WindowMode::position`]
/// centers the window instead.
pub fn mode() -> (u32, u32, WindowMode) {
    let (width, height) = dimensions();
    let (x, y, display) = position_on_display();
    context::with(|ctx| {
        let (min_width, min_height) = ctx.backend.minimum_size();
        let mode = WindowMode {
            fullscreen: ctx.backend.is_fullscreen(),
            fullscreen_type: ctx.backend.fullscreen_type(),
            vsync: ctx.backend.vsync(),
            resizable: ctx.backend.is_resizable(),
            borderless: ctx.backend.is_borderless(),
            centered: true,
            display,
            min_width,
            min_height,
            position: Some((x, y)),
            msaa: ctx.backend.screen_msaa(),
        };
        (width, height, mode)
    })
}

/// Changes the window's size and every setting at once, the equivalent of
/// `love.window.setMode`. A size of 0 takes the size of the display's
/// desktop, as in LÖVE. Opens the window if [`close`] hid it.
///
/// This calls [`crate::Game::resized`] if the size changed. Settings the
/// platform refuses print a message and are left as they were; the rest
/// still apply. Returns whether everything applied.
///
/// ```no_run
/// use smitten::window::{self, WindowMode};
///
/// window::set_mode(1280, 720, WindowMode {
///     resizable: true,
///     min_width: 640,
///     min_height: 360,
///     ..WindowMode::default()
/// });
/// ```
pub fn set_mode(width: u32, height: u32, mode: WindowMode) -> bool {
    context::with(|ctx| apply_mode(ctx, width, height, &mode))
}

/// Changes some of the window's settings and leaves the rest, the
/// equivalent of `love.window.updateMode`. `change` gets the current
/// settings from [`mode`] to edit; `width` and `height` replace the size.
///
/// ```no_run
/// use smitten::window;
///
/// let (width, height) = window::dimensions();
/// window::update_mode(width, height, |mode| mode.borderless = true);
/// ```
pub fn update_mode(width: u32, height: u32, change: impl FnOnce(&mut WindowMode)) -> bool {
    let (_, _, mut mode) = mode();
    change(&mut mode);
    set_mode(width, height, mode)
}

fn apply_mode(ctx: &mut context::Context, width: u32, height: u32, mode: &WindowMode) -> bool {
    let mut applied = true;
    if ctx.window_closed {
        ctx.backend.set_window_shown(true);
        ctx.window_closed = false;
    }
    let display = if ctx.backend.display_bounds(mode.display).is_some() {
        mode.display
    } else {
        0
    };
    let (width, height) = if width == 0 || height == 0 {
        let desktop = ctx
            .backend
            .display_desktop_size(display)
            .unwrap_or_else(|| ctx.backend.desktop_size());
        (
            if width == 0 { desktop.0 } else { width },
            if height == 0 { desktop.1 } else { height },
        )
    } else {
        (width, height)
    };

    // Leave fullscreen first, so the size and position below apply to the
    // window rather than to the fullscreen mode.
    let kind = ctx.backend.fullscreen_type();
    if ctx.backend.is_fullscreen()
        && let Err(message) = ctx.backend.set_fullscreen(false, kind)
    {
        eprintln!("smitten: could not leave fullscreen: {message}");
        applied = false;
    }
    ctx.backend.set_borderless(mode.borderless);
    ctx.backend.set_resizable(mode.resizable);
    ctx.backend
        .set_minimum_size(mode.min_width.max(1), mode.min_height.max(1));
    if let Err(message) = ctx.backend.set_window_size(width, height) {
        eprintln!("smitten: could not resize the window: {message}");
        applied = false;
    }
    match mode.position {
        Some((x, y)) => place(ctx, x, y, display),
        None if mode.centered || display != ctx.backend.window_display() => {
            ctx.backend.center_window(display);
        }
        None => {}
    }
    if let Err(message) = ctx
        .backend
        .set_fullscreen(mode.fullscreen, mode.fullscreen_type)
    {
        eprintln!("smitten: could not change fullscreen: {message}");
        applied = false;
    }
    if mode.vsync != ctx.backend.vsync()
        && let Err(message) = ctx.backend.set_vsync(mode.vsync)
    {
        eprintln!("smitten: could not change vsync: {message}");
        applied = false;
    }
    if mode.msaa.max(1) != ctx.backend.screen_msaa() {
        ctx.backend.set_screen_msaa(mode.msaa);
    }
    measure(ctx);
    applied
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
/// Fullscreen keeps the [`FullscreenType`] last used, which is
/// [`FullscreenType::Desktop`] unless the game chose otherwise: the
/// desktop's own resolution, so switching is quick and other windows are
/// left alone. The drawable size changes, which calls
/// [`crate::Game::resized`]. To start in fullscreen, set
/// [`crate::Config::fullscreen`] instead.
///
/// If the platform refuses, a message is printed and nothing changes.
pub fn set_fullscreen(fullscreen: bool) {
    context::with(|ctx| {
        let kind = ctx.backend.fullscreen_type();
        apply_fullscreen(ctx, fullscreen, kind);
        measure(ctx);
    });
}

/// Makes the game fill the whole screen in the way `kind` says, or puts it
/// back in a window, the two-argument form of `love.window.setFullscreen`.
///
/// [`FullscreenType::Exclusive`] changes the display's resolution to the
/// one closest to the window's size, so set the size first with
/// [`set_size`] or pick one from [`fullscreen_modes`].
pub fn set_fullscreen_with(fullscreen: bool, kind: FullscreenType) {
    context::with(|ctx| {
        apply_fullscreen(ctx, fullscreen, kind);
        measure(ctx);
    });
}

fn apply_fullscreen(ctx: &mut context::Context, fullscreen: bool, kind: FullscreenType) {
    if let Err(message) = ctx.backend.set_fullscreen(fullscreen, kind) {
        eprintln!("smitten: could not change fullscreen: {message}");
    }
}

/// How the game fills the screen when it is fullscreen, the second value
/// of `love.window.getFullscreen`.
pub fn fullscreen_type() -> FullscreenType {
    context::with(|ctx| ctx.backend.fullscreen_type())
}

/// The sizes in units that [`FullscreenType::Exclusive`] can switch
/// `display` to, largest first, the equivalent of
/// `love.window.getFullscreenModes`. Displays are counted from 0. Empty if
/// there is no such display.
pub fn fullscreen_modes(display: u32) -> Vec<(u32, u32)> {
    context::with(|ctx| ctx.backend.fullscreen_modes(display))
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

/// Sets the window's icon from an image file (PNG, JPEG or TGA), looked up like
/// any other asset. To set it from the start, use [`crate::Config::icon`].
///
/// Some platforms take a game's icon from somewhere else: macOS shows the
/// application bundle's icon in the Dock, and Windows shows the icon built
/// into the executable in Explorer.
pub fn set_icon(path: impl AsRef<Path>) -> Result<(), ImageError> {
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

/// How many samples each pixel of the screen holds, 1 for none: the
/// `msaa` of `love.window.getMode`. It may be fewer than
/// [`Config::msaa`](crate::Config::msaa) asked for, if the GPU cannot do
/// that many.
pub fn msaa() -> u32 {
    context::with(|ctx| ctx.backend.screen_msaa())
}

/// Smooths the edges of what is drawn to the screen with `samples`
/// samples per pixel, or stops for 0 or 1, the `msaa` flag of LÖVE's
/// `setMode`. Returns how many samples the GPU gave, which may be fewer.
///
/// Canvases are not affected; see
/// [`CanvasSettings::msaa`](crate::graphics::CanvasSettings::msaa). With a
/// [`Scaling::PixelPerfect`](crate::Scaling::PixelPerfect) design size the
/// game draws to a canvas first, so this does not smooth it.
pub fn set_msaa(samples: u32) -> u32 {
    context::with(|ctx| ctx.backend.set_screen_msaa(samples))
}
