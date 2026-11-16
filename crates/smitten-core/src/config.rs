use std::path::PathBuf;

/// Startup settings, the equivalent of LÖVE's `conf.lua`.
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
    /// An image file to use as the window's icon, looked up like any other
    /// asset. When `None`, the platform's default icon is used.
    pub icon: Option<PathBuf>,
    /// Names the game's save directory, like `t.identity` in LÖVE's
    /// `conf.lua`. When `None`, the executable's name is used.
    pub identity: Option<String>,
    /// Whether presentation waits for the display's vertical sync.
    pub vsync: bool,
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
            icon: None,
            identity: None,
            vsync: true,
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
