/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)] // The variants are named for what they are.
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
#[allow(missing_docs)] // The variants are named for what they are.
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

impl GamepadButton {
    /// Every button.
    pub const ALL: [GamepadButton; 15] = [
        GamepadButton::A,
        GamepadButton::B,
        GamepadButton::X,
        GamepadButton::Y,
        GamepadButton::Back,
        GamepadButton::Guide,
        GamepadButton::Start,
        GamepadButton::LeftStick,
        GamepadButton::RightStick,
        GamepadButton::LeftShoulder,
        GamepadButton::RightShoulder,
        GamepadButton::DpadUp,
        GamepadButton::DpadDown,
        GamepadButton::DpadLeft,
        GamepadButton::DpadRight,
    ];
}

/// An analog input on a standard gamepad.
///
/// Stick axes run from -1.0 to 1.0, with right and down positive. Triggers
/// run from 0.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)] // The variants are named for what they are.
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

/// The direction a joystick's hat (a small digital pad) is pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[allow(missing_docs)] // The variants are named for what they are.
pub enum JoystickHat {
    #[default]
    Centered,
    Up,
    Right,
    Down,
    Left,
    RightUp,
    RightDown,
    LeftUp,
    LeftDown,
}

impl JoystickHat {
    /// Reads a hat from the usual bit flags: 1 for up, 2 for right, 4 for
    /// down and 8 for left. Opposite directions held together cancel out.
    pub fn from_bits(bits: u8) -> JoystickHat {
        let up = bits & 1 != 0 && bits & 4 == 0;
        let down = bits & 4 != 0 && bits & 1 == 0;
        let right = bits & 2 != 0 && bits & 8 == 0;
        let left = bits & 8 != 0 && bits & 2 == 0;
        match (up, right, down, left) {
            (true, true, _, _) => JoystickHat::RightUp,
            (true, _, _, true) => JoystickHat::LeftUp,
            (_, true, true, _) => JoystickHat::RightDown,
            (_, _, true, true) => JoystickHat::LeftDown,
            (true, _, _, _) => JoystickHat::Up,
            (_, true, _, _) => JoystickHat::Right,
            (_, _, true, _) => JoystickHat::Down,
            (_, _, _, true) => JoystickHat::Left,
            _ => JoystickHat::Centered,
        }
    }

    /// The bit flags for this direction, as read by
    /// [`JoystickHat::from_bits`].
    pub fn bits(self) -> u8 {
        match self {
            JoystickHat::Centered => 0,
            JoystickHat::Up => 1,
            JoystickHat::Right => 2,
            JoystickHat::Down => 4,
            JoystickHat::Left => 8,
            JoystickHat::RightUp => 2 | 1,
            JoystickHat::RightDown => 2 | 4,
            JoystickHat::LeftUp => 8 | 1,
            JoystickHat::LeftDown => 8 | 4,
        }
    }
}

/// What a joystick reports about itself when it is connected.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JoystickInfo {
    /// The device's name, as the system reports it.
    pub name: String,
    /// A string of 32 hex digits naming the kind of device, the key that
    /// gamepad mappings are stored under.
    pub guid: String,
    /// The USB vendor id, or 0 if unknown.
    pub vendor_id: u16,
    /// The USB product id, or 0 if unknown.
    pub product_id: u16,
    /// The product version, or 0 if unknown.
    pub product_version: u16,
    /// How many axes it has.
    pub axis_count: usize,
    /// How many buttons it has.
    pub button_count: usize,
    /// How many hats it has.
    pub hat_count: usize,
}

/// A button or axis of a standard gamepad, as named in a gamepad mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)] // The variants are named for what they are.
pub enum GamepadInput {
    Button(GamepadButton),
    Axis(GamepadAxis),
}

/// A raw input of a joystick, numbered from 0 in the order the device
/// reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)] // The variants are named for what they are.
pub enum JoystickInput {
    Button(usize),
    Axis(usize),
    /// A hat pushed in one direction.
    Hat(usize, JoystickHat),
}
