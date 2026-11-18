/// A mouse cursor provided by the operating system, as LÖVE's
/// `CursorType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Cursor {
    /// The ordinary arrow.
    #[default]
    Arrow,
    /// The bar shown over text.
    IBeam,
    /// Busy: the game cannot be used for a moment.
    Wait,
    /// Busy in the background: the game can still be used.
    WaitArrow,
    /// A thin cross, for aiming.
    Crosshair,
    /// The pointing hand shown over links and buttons.
    Hand,
    /// A slashed circle: this cannot be done here.
    No,
    /// Four arrows, for moving something.
    SizeAll,
    /// Up and down arrows.
    SizeNs,
    /// Left and right arrows.
    SizeWe,
    /// Arrows towards the top-left and bottom-right.
    SizeNwse,
    /// Arrows towards the top-right and bottom-left.
    SizeNesw,
}

/// Where the computer's power is coming from, as LÖVE's `PowerState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PowerState {
    /// It could not be found out.
    #[default]
    Unknown,
    /// Running on its battery.
    Battery,
    /// Plugged in, with no battery.
    NoBattery,
    /// Plugged in and charging its battery.
    Charging,
    /// Plugged in with a full battery.
    Charged,
}

/// The state of the computer's power supply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PowerInfo {
    /// Whether the computer runs on battery or is plugged in.
    pub state: PowerState,
    /// How full the battery is, from 0 to 100, if that is known.
    pub percent: Option<u8>,
    /// How many seconds of battery life are left, if that is known.
    pub seconds: Option<u32>,
}

/// What the window is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowState {
    /// Shrunk to the taskbar or Dock.
    pub minimized: bool,
    /// Enlarged to fill the desktop, short of fullscreen.
    pub maximized: bool,
    /// Receiving the keyboard.
    pub focused: bool,
    /// Under the pointer.
    pub mouse_focused: bool,
    /// Shown on screen: neither hidden nor minimized.
    pub visible: bool,
}

/// The icon a message box shows, as LÖVE's `MessageBoxType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MessageBoxKind {
    #[default]
    /// An information icon.
    Info,
    /// A warning icon.
    Warning,
    /// An error icon.
    Error,
}

/// Which way up a display is, as LÖVE's `DisplayOrientation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisplayOrientation {
    /// It could not be found out.
    #[default]
    Unknown,
    /// Wider than tall, the usual way up.
    Landscape,
    /// Wider than tall, upside down.
    LandscapeFlipped,
    /// Taller than wide, the usual way up.
    Portrait,
    /// Taller than wide, upside down.
    PortraitFlipped,
}

/// How a window fills the screen, as LÖVE's `FullscreenType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FullscreenType {
    /// Covers the display at the desktop's own resolution. Switching is
    /// quick, and other windows and displays are left alone.
    #[default]
    Desktop,
    /// Changes the display's resolution to the one closest to the
    /// window's size, as older games do. Switching can take a moment and
    /// may rearrange other windows.
    Exclusive,
}
