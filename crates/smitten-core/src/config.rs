use std::path::PathBuf;

/// Startup settings, the equivalent of LÖVE's `conf.lua`.
#[derive(Debug, Clone)]
pub struct Config {
    /// Window title.
    pub title: String,
    /// Window width in screen units.
    pub width: u32,
    /// Window height in screen units.
    pub height: u32,
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            title: "Untitled".to_string(),
            width: 800,
            height: 600,
            resizable: false,
            fullscreen: false,
            icon: None,
            identity: None,
            vsync: true,
        }
    }
}
