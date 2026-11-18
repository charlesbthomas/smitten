//! Joysticks and gamepads, the equivalent of `love.joystick`.
//!
//! Every connected joystick is reported. Those SDL recognizes as standard
//! gamepads, which covers Xbox, PlayStation and Switch controllers and most
//! others, can also be read by gamepad button and axis names. Any other
//! joystick can be read by its raw buttons, axes and hats, or given a
//! mapping with [`set_gamepad_mapping`] to make it a gamepad.

use glam::Vec2;
use std::io;
use std::path::Path;

use smitten_core::JoystickId;

use crate::context;
use crate::gamepad_mapping;
use crate::input::JoystickState;
use crate::{GamepadAxis, GamepadButton, GamepadInput, JoystickHat, JoystickInput};

/// A handle to a connected joystick or gamepad, the equivalent of LÖVE's
/// `Joystick`.
///
/// Handles are small and can be copied and compared freely. A handle stays
/// valid after its device is unplugged; it then reports nothing held.
///
/// Raw buttons, axes and hats count from 0, where LÖVE counts from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Joystick {
    id: JoystickId,
}

impl Joystick {
    pub(crate) fn new(id: JoystickId) -> Self {
        Self { id }
    }

    /// A number that identifies this gamepad until it is unplugged.
    pub fn id(&self) -> u32 {
        self.id.0
    }

    fn read<R>(&self, default: R, f: impl FnOnce(&JoystickState) -> R) -> R {
        context::with(|ctx| ctx.input.joystick(self.id).map_or(default, f))
    }

    /// Whether the device is recognized as a standard gamepad, so that the
    /// `gamepad_` methods report its buttons and axes.
    pub fn is_gamepad(&self) -> bool {
        context::with(|ctx| {
            ctx.input.joystick(self.id).is_some() && ctx.backend.is_gamepad(self.id)
        })
    }

    /// A string of 32 hex digits naming the kind of device, shared by
    /// every device of the same make and model. Gamepad mappings are
    /// stored under it. Empty once the device is unplugged.
    pub fn guid(&self) -> String {
        self.read(String::new(), |joystick| joystick.info.guid.clone())
    }

    /// The USB vendor id, product id and product version, as LÖVE's
    /// `Joystick:getDeviceInfo`. Each is 0 if unknown.
    #[doc(alias = "Joystick:getDeviceInfo")]
    pub fn device_info(&self) -> (u16, u16, u16) {
        self.read((0, 0, 0), |joystick| {
            let info = &joystick.info;
            (info.vendor_id, info.product_id, info.product_version)
        })
    }

    /// How many raw axes the device has.
    pub fn axis_count(&self) -> usize {
        self.read(0, |joystick| joystick.raw_axes.len())
    }

    /// How many raw buttons the device has.
    pub fn button_count(&self) -> usize {
        self.read(0, |joystick| joystick.raw_buttons.len())
    }

    /// How many hats the device has.
    pub fn hat_count(&self) -> usize {
        self.read(0, |joystick| joystick.hats.len())
    }

    /// The position of raw axis `axis`, from -1.0 to 1.0, or 0.0 for an
    /// axis the device does not have.
    pub fn axis(&self, axis: usize) -> f32 {
        self.read(0.0, |joystick| {
            joystick.raw_axes.get(axis).copied().unwrap_or(0.0)
        })
    }

    /// Every raw axis's position, in order.
    pub fn axes(&self) -> Vec<f32> {
        self.read(Vec::new(), |joystick| joystick.raw_axes.clone())
    }

    /// Returns whether raw button `button` is held.
    pub fn is_down(&self, button: usize) -> bool {
        self.read(false, |joystick| {
            joystick.raw_buttons.get(button).copied().unwrap_or(false)
        })
    }

    /// Returns whether any of the raw `buttons` is held, as LÖVE's
    /// `Joystick:isDown` does when given several.
    #[doc(alias = "Joystick:isDown")]
    pub fn is_any_down(&self, buttons: &[usize]) -> bool {
        buttons.iter().any(|&button| self.is_down(button))
    }

    /// The direction hat `hat` is pushed, or
    /// [`JoystickHat::Centered`] for a hat the device does not have.
    pub fn hat(&self, hat: usize) -> JoystickHat {
        self.read(JoystickHat::Centered, |joystick| {
            joystick.hats.get(hat).copied().unwrap_or_default()
        })
    }

    /// The raw input that gamepad `input` is read from, as LÖVE's
    /// `Joystick:getGamepadMapping`. `None` if the device has no mapping
    /// or the mapping leaves `input` out.
    #[doc(alias = "Joystick:getGamepadMapping")]
    pub fn gamepad_mapping(&self, input: GamepadInput) -> Option<JoystickInput> {
        let mapping = self.gamepad_mapping_string()?;
        gamepad_mapping::lookup(&mapping, input)
    }

    /// The whole mapping used for this device, in SDL's format, if it has
    /// one. Pass it to [`load_gamepad_mappings_from_str`] in a later run to
    /// restore it.
    pub fn gamepad_mapping_string(&self) -> Option<String> {
        let guid = self.guid();
        if guid.is_empty() {
            return None;
        }
        gamepad_mapping_string(&guid)
    }

    /// Returns whether the gamepad is still plugged in.
    pub fn is_connected(&self) -> bool {
        context::with(|ctx| ctx.input.joystick(self.id).is_some())
    }

    /// The device's name, or an empty string once it is unplugged.
    pub fn name(&self) -> String {
        self.read(String::new(), |joystick| joystick.info.name.clone())
    }

    /// Returns whether `button` is currently held.
    pub fn is_gamepad_down(&self, button: GamepadButton) -> bool {
        context::with(|ctx| {
            ctx.input
                .joystick(self.id)
                .is_some_and(|joystick| joystick.buttons.contains(&button))
        })
    }

    /// The current position of `axis`: -1.0 to 1.0 for sticks, with right
    /// and down positive, and 0.0 to 1.0 for triggers.
    ///
    /// Sticks rarely rest at exactly zero, so compare against a small
    /// threshold such as 0.2 before treating a value as movement.
    pub fn gamepad_axis(&self, axis: GamepadAxis) -> f32 {
        context::with(|ctx| {
            ctx.input
                .joystick(self.id)
                .map_or(0.0, |joystick| joystick.axes[axis.index()])
        })
    }

    /// The left stick as a direction, with right and down positive.
    ///
    /// Sticks rarely rest at exactly zero, so anything pushed less than
    /// `dead_zone` reads as `(0.0, 0.0)`; 0.2 suits most gamepads. Past
    /// that the push grows smoothly from zero and is never longer than 1.
    pub fn left_stick(&self, dead_zone: f32) -> Vec2 {
        crate::input::stick_with_dead_zone(
            self.gamepad_axis(GamepadAxis::LeftX),
            self.gamepad_axis(GamepadAxis::LeftY),
            dead_zone,
        )
        .into()
    }

    /// The right stick as a direction. See [`Joystick::left_stick`].
    pub fn right_stick(&self, dead_zone: f32) -> Vec2 {
        crate::input::stick_with_dead_zone(
            self.gamepad_axis(GamepadAxis::RightX),
            self.gamepad_axis(GamepadAxis::RightY),
            dead_zone,
        )
        .into()
    }

    /// Whether the gamepad can rumble.
    pub fn is_vibration_supported(&self) -> bool {
        context::with(|ctx| ctx.backend.gamepad_can_rumble(self.id))
    }

    /// Makes the gamepad rumble, as LÖVE's `Joystick:setVibration`.
    ///
    /// `left` and `right` are the strengths of the two motors, from 0.0 to
    /// 1.0. The left one is the heavy, low rumble and the right one the
    /// light buzz. `seconds` is how long to keep going. This replaces any
    /// rumble already running.
    ///
    /// Returns whether the gamepad took the request: `false` for one that
    /// cannot rumble or has been unplugged.
    #[doc(alias = "Joystick:setVibration")]
    pub fn set_vibration(&self, left: f32, right: f32, seconds: f32) -> bool {
        context::with(|ctx| {
            let taken = ctx.backend.rumble_gamepad(self.id, left, right, seconds);
            if let Some(joystick) = ctx.input.joystick_mut(self.id) {
                joystick.vibration = if taken {
                    let length = (f64::from(seconds.max(0.0)) * 1e9) as u64;
                    let end = ctx.backend.ticks_ns().saturating_add(length);
                    (left.clamp(0.0, 1.0), right.clamp(0.0, 1.0), end)
                } else {
                    (0.0, 0.0, 0)
                };
            }
            taken
        })
    }

    /// The strengths of the left and right motors, as LÖVE's
    /// `Joystick:getVibration`: what [`set_vibration`](Self::set_vibration)
    /// last asked for, or `(0.0, 0.0)` once its time is up or if the
    /// gamepad did not take it.
    #[doc(alias = "Joystick:getVibration")]
    pub fn vibration(&self) -> (f32, f32) {
        context::with(|ctx| {
            let now = ctx.backend.ticks_ns();
            ctx.input.joystick(self.id).map_or((0.0, 0.0), |joystick| {
                let (left, right, end) = joystick.vibration;
                if now < end { (left, right) } else { (0.0, 0.0) }
            })
        })
    }

    /// Stops any rumble. Returns whether the gamepad took the request.
    pub fn stop_vibration(&self) -> bool {
        self.set_vibration(0.0, 0.0, 0.0)
    }
}

/// The connected joysticks and gamepads, in the order they were plugged
/// in.
pub fn joysticks() -> Vec<Joystick> {
    context::with(|ctx| {
        ctx.input
            .joysticks
            .iter()
            .map(|joystick| Joystick::new(joystick.id))
            .collect()
    })
}

/// How many joysticks and gamepads are connected.
pub fn joystick_count() -> usize {
    context::with(|ctx| ctx.input.joysticks.len())
}

/// Binds a gamepad button or axis to one of the raw inputs of every
/// joystick with this GUID, the equivalent of
/// `love.joystick.setGamepadMapping`. Any binding `input` had is replaced.
/// A joystick that had no mapping becomes a gamepad once it has one.
///
/// ```no_run
/// # use smitten::{joystick, GamepadButton, GamepadInput, JoystickInput};
/// # let pad = joystick::joysticks()[0];
/// joystick::set_gamepad_mapping(
///     &pad.guid(),
///     GamepadInput::Button(GamepadButton::A),
///     JoystickInput::Button(2),
/// );
/// ```
///
/// Returns whether the mapping was taken.
#[doc(alias = "love.joystick.setGamepadMapping")]
pub fn set_gamepad_mapping(guid: &str, input: GamepadInput, from: JoystickInput) -> bool {
    let guid = guid.trim();
    if guid.is_empty() || guid.contains(',') {
        return false;
    }
    let base = gamepad_mapping_string(guid).unwrap_or_else(|| {
        // A new mapping needs a name; use the device's if one is plugged in.
        let name = context::with(|ctx| {
            ctx.input
                .joysticks
                .iter()
                .find(|joystick| joystick.info.guid == guid)
                .map(|joystick| joystick.info.name.replace(',', " "))
        });
        format!("{guid},{},", name.as_deref().unwrap_or("Gamepad"))
    });
    let mapping = gamepad_mapping::with_binding(&base, input, from);
    load_gamepad_mappings_from_str(&mapping).is_ok_and(|added| added > 0)
}

/// Adds gamepad mappings from text in SDL's format, one a line, as the
/// widely shared `gamecontrollerdb.txt` is written. Each replaces any
/// mapping for the same GUID. Blank lines, `#` comments and lines for
/// other platforms are skipped.
///
/// Returns how many mappings were added, or an
/// [`InvalidData`](io::ErrorKind::InvalidData) error if SDL cannot read
/// them.
pub fn load_gamepad_mappings_from_str(mappings: &str) -> io::Result<usize> {
    let added = context::with(|ctx| ctx.backend.add_gamepad_mappings(mappings))
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))?;
    context::with(|ctx| {
        for guid in mappings.lines().filter_map(gamepad_mapping::guid_of) {
            if !ctx.input.added_mappings.iter().any(|known| known == guid) {
                ctx.input.added_mappings.push(guid.to_string());
            }
        }
    });
    Ok(added)
}

/// Adds gamepad mappings from a file, the equivalent of
/// `love.joystick.loadGamepadMappings(filename)`. The path is looked up
/// as [`filesystem::read`](crate::filesystem::read) does. See
/// [`load_gamepad_mappings_from_str`].
#[doc(alias = "love.joystick.loadGamepadMappings")]
pub fn load_gamepad_mappings(path: impl AsRef<Path>) -> io::Result<usize> {
    let text = crate::filesystem::read_string(path)?;
    load_gamepad_mappings_from_str(&text)
}

/// The mapping used for joysticks with this GUID, in SDL's format, the
/// equivalent of `love.joystick.getGamepadMappingString`.
#[doc(alias = "love.joystick.getGamepadMappingString")]
pub fn gamepad_mapping_string(guid: &str) -> Option<String> {
    context::with(|ctx| ctx.backend.gamepad_mapping(guid))
}

/// The mappings of the connected gamepads and of every mapping the game
/// added, one a line, the equivalent of
/// `love.joystick.saveGamepadMappings`. Write the text to a file and load
/// it with [`load_gamepad_mappings`] in a later run to keep the player's
/// choices.
#[doc(alias = "love.joystick.saveGamepadMappings")]
pub fn save_gamepad_mappings() -> String {
    let guids = context::with(|ctx| {
        let mut guids = ctx.input.added_mappings.clone();
        for joystick in &ctx.input.joysticks {
            let guid = &joystick.info.guid;
            if ctx.backend.is_gamepad(joystick.id) && !guids.contains(guid) {
                guids.push(guid.clone());
            }
        }
        guids
    });
    let mut text = String::new();
    for mapping in guids.iter().filter_map(|guid| gamepad_mapping_string(guid)) {
        text.push_str(&mapping);
        text.push('\n');
    }
    text
}

/// The connected joysticks that are standard gamepads, in the order they
/// were plugged in. Most games want this rather than [`joysticks`], so a
/// flight stick or a wheel plugged in first does not take player one's
/// place.
///
/// LÖVE has no such call; games filter with `isGamepad` by hand.
pub fn gamepads() -> Vec<Joystick> {
    joysticks()
        .into_iter()
        .filter(Joystick::is_gamepad)
        .collect()
}
