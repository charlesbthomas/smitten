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
    pub state: PowerState,
    /// How full the battery is, from 0 to 100, if that is known.
    pub percent: Option<u8>,
    /// How many seconds of battery life are left, if that is known.
    pub seconds: Option<u32>,
}
