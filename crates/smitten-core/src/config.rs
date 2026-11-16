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
    /// Names the game's save directory, like `t.identity` in LÖVE's
    /// `conf.lua`. When `None`, the executable's name is used.
    pub identity: Option<String>,
    /// Whether presentation waits for the display's vertical sync.
    pub vsync: bool,
    /// Whether images and shader files are loaded again when they change
    /// on disk while the game runs. On by default in debug builds, so
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
            identity: None,
            vsync: true,
            hot_reload: cfg!(debug_assertions),
        }
    }
}
