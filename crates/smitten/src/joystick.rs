//! Gamepads, the equivalent of `love.joystick`.
//!
//! Only controllers that SDL recognizes as standard gamepads are reported,
//! which covers Xbox, PlayStation and Switch controllers and most others.

use smitten_core::JoystickId;

use crate::context;
use crate::{GamepadAxis, GamepadButton};

/// A handle to a connected gamepad, the equivalent of LÖVE's `Joystick`.
///
/// Handles are small and can be copied and compared freely. A handle stays
/// valid after its gamepad is unplugged; it then reports nothing held.
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

    /// Returns whether the gamepad is still plugged in.
    pub fn is_connected(&self) -> bool {
        context::with(|ctx| ctx.input.joystick(self.id).is_some())
    }

    /// The gamepad's name, or an empty string once it is unplugged.
    pub fn name(&self) -> String {
        context::with(|ctx| {
            ctx.input
                .joystick(self.id)
                .map(|joystick| joystick.name.clone())
                .unwrap_or_default()
        })
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
    pub fn left_stick(&self, dead_zone: f32) -> (f32, f32) {
        crate::input::stick_with_dead_zone(
            self.gamepad_axis(GamepadAxis::LeftX),
            self.gamepad_axis(GamepadAxis::LeftY),
            dead_zone,
        )
    }

    /// The right stick as a direction. See [`Joystick::left_stick`].
    pub fn right_stick(&self, dead_zone: f32) -> (f32, f32) {
        crate::input::stick_with_dead_zone(
            self.gamepad_axis(GamepadAxis::RightX),
            self.gamepad_axis(GamepadAxis::RightY),
            dead_zone,
        )
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
    pub fn set_vibration(&self, left: f32, right: f32, seconds: f32) -> bool {
        context::with(|ctx| ctx.backend.rumble_gamepad(self.id, left, right, seconds))
    }

    /// Stops any rumble. Returns whether the gamepad took the request.
    pub fn stop_vibration(&self) -> bool {
        self.set_vibration(0.0, 0.0, 0.0)
    }
}

/// The connected gamepads, in the order they were plugged in.
pub fn joysticks() -> Vec<Joystick> {
    context::with(|ctx| {
        ctx.input
            .joysticks
            .iter()
            .map(|joystick| Joystick::new(joystick.id))
            .collect()
    })
}

/// How many gamepads are connected.
pub fn joystick_count() -> usize {
    context::with(|ctx| ctx.input.joysticks.len())
}
