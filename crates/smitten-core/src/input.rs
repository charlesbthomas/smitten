/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// The first extra button, often "back".
    X1,
    /// The second extra button, often "forward".
    X2,
}

/// Identifies one finger on a touch screen, from the moment it touches
/// until it lifts.
///
/// A finger that touches again later may get a different id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TouchId(pub u64);

/// Identifies one connected joystick for as long as it stays connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JoystickId(pub u32);

/// A button on a standard gamepad, named after the Xbox layout as in LÖVE.
///
/// `A` is always the bottom face button, whatever is printed on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GamepadButton {
    A,
    B,
    X,
    Y,
    Back,
    Guide,
    Start,
    LeftStick,
    RightStick,
    LeftShoulder,
    RightShoulder,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

/// An analog input on a standard gamepad.
///
/// Stick axes run from -1.0 to 1.0, with right and down positive. Triggers
/// run from 0.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GamepadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    TriggerLeft,
    TriggerRight,
}

impl GamepadAxis {
    /// Every axis, in a fixed order usable for indexing.
    pub const ALL: [GamepadAxis; 6] = [
        GamepadAxis::LeftX,
        GamepadAxis::LeftY,
        GamepadAxis::RightX,
        GamepadAxis::RightY,
        GamepadAxis::TriggerLeft,
        GamepadAxis::TriggerRight,
    ];

    /// This axis's position in [`GamepadAxis::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }
}
