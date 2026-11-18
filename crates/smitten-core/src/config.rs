use std::path::PathBuf;

use crate::FullscreenType;

/// Startup settings, the equivalent of LÖVE's `conf.lua`.
///
/// LÖVE's `t.modules.*` switches are Cargo features of the `smitten`
/// crate here: turn off `physics` or `video` to leave those modules out
/// of the build. The other modules are small enough to always include.
#[derive(Debug, Clone)]
pub struct Config {
    /// Window title.
    pub title: String,
    /// Window width in units. See [`Config::dpi_scaling`].
    pub width: u32,
    /// Window height in units.
    pub height: u32,
    /// Whether sizes and positions are measured in units that look the
    /// same size on every display, as in LÖVE. On an ordinary display a
    /// unit is one pixel. On a high-DPI display it is more, so an 800 by
    /// 600 game still looks 800 by 600 and is simply drawn sharper.
    ///
    /// Turn it off to work in raw pixels instead.
    pub dpi_scaling: bool,
    /// A fixed size the game is designed for, as `(width, height)`. The
    /// game then always draws in a space of exactly that size, and Smitten
    /// scales the picture to the window and adds black bars where the
    /// shapes differ. The mouse position is reported in the same space.
    pub design_size: Option<(u32, u32)>,
    /// How the picture is scaled to the window when [`Config::design_size`]
    /// is set.
    pub design_scaling: Scaling,
    /// Whether the user can resize the window.
    pub resizable: bool,
    /// Whether the game starts filling the whole screen. It can be changed
    /// while the game runs.
    pub fullscreen: bool,
    /// How the game fills the screen when [`Config::fullscreen`] is on.
    pub fullscreen_type: FullscreenType,
    /// Whether the window has no title bar or frame, like `t.window.borderless`.
    pub borderless: bool,
    /// The smallest width in units the user can shrink a resizable window
    /// to, like `t.window.minwidth`. LÖVE's default is 1.
    pub min_width: u32,
    /// The smallest height in units, like `t.window.minheight`.
    pub min_height: u32,
    /// Which display the window opens on, counted from 0 in the order the
    /// system lists them (LÖVE counts from 1). A display that is not
    /// connected falls back to the first.
    pub display: u32,
    /// Where the window's top-left corner goes, in desktop coordinates
    /// from the top-left of [`Config::display`], like `t.window.x` and
    /// `t.window.y`. When `None`, [`Config::centered`] decides.
    pub position: Option<(i32, i32)>,
    /// Whether the window opens in the middle of its display when no
    /// [`Config::position`] is given, like `t.window.centered`.
    pub centered: bool,
    /// An image file to use as the window's icon, looked up like any other
    /// asset. When `None`, the platform's default icon is used.
    pub icon: Option<PathBuf>,
    /// Names the game's save directory, like `t.identity` in LÖVE's
    /// `conf.lua`. When `None`, the executable's name is used.
    pub identity: Option<String>,
    /// Whether the asset directory is searched before the save directory,
    /// like `t.appendidentity`. Off by default, so a file the game saved
    /// overrides a shipped file with the same name.
    pub append_identity: bool,
    /// Whether to open a console window for the game's printed output on
    /// Windows, like `t.console`. It matters for a game built with
    /// `#![windows_subsystem = "windows"]`, which otherwise has nowhere to
    /// print. When the game was started from a console, that console is
    /// used instead of a new one. Other platforms ignore it.
    pub console: bool,
    /// Whether presentation waits for the display's vertical sync.
    pub vsync: bool,
    /// How many samples each pixel of the screen holds, to smooth the
    /// edges of shapes and images, as `t.window.msaa`. 0 or 1 is none; 2,
    /// 4 and 8 are common. The GPU may give fewer than asked for.
    pub msaa: u32,
    /// Whether images, shaders, fonts and sounds are loaded again when
    /// their files change on disk while the game runs. On by default in debug builds, so
    /// `cargo run` picks up edits, and off in release builds.
    pub hot_reload: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            title: "Untitled".to_string(),
            width: 800,
            height: 600,
            resizable: false,
            dpi_scaling: true,
            design_size: None,
            design_scaling: Scaling::default(),
            fullscreen: false,
            fullscreen_type: FullscreenType::Desktop,
            borderless: false,
            min_width: 1,
            min_height: 1,
            display: 0,
            position: None,
            centered: true,
            icon: None,
            identity: None,
            append_identity: false,
            console: false,
            vsync: true,
            msaa: 0,
            hot_reload: cfg!(debug_assertions),
        }
    }
}

/// How a game with a [`Config::design_size`] is fitted to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scaling {
    /// Scale smoothly to the largest size that fits. Everything is drawn
    /// at the window's full resolution, so shapes, text and large images
    /// stay sharp. Good for games with high-resolution art.
    #[default]
    Fit,
    /// Draw at exactly the design size, one pixel per unit, then enlarge
    /// the result by a whole number with hard pixel edges. Good for pixel
    /// art, where every game pixel must stay a perfect square.
    PixelPerfect,
}
