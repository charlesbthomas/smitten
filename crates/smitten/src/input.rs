//! Tracks what is currently held down or connected, from the stream of
//! backend events. The `keyboard`, `mouse`, `touch` and `joystick` modules answer
//! their state queries from here.

use std::collections::HashSet;

use smitten_core::backend::BackendEvent;
use smitten_core::{
    GamepadAxis, GamepadButton, JoystickHat, JoystickId, JoystickInfo, Key, MouseButton, TouchId,
};

pub(crate) struct JoystickState {
    pub id: JoystickId,
    pub info: JoystickInfo,
    /// Gamepad buttons held, for a joystick that is a gamepad.
    pub buttons: HashSet<GamepadButton>,
    pub axes: [f32; GamepadAxis::ALL.len()],
    /// Raw buttons held, by number.
    pub raw_buttons: Vec<bool>,
    pub raw_axes: Vec<f32>,
    pub hats: Vec<JoystickHat>,
}

impl JoystickState {
    fn new(id: JoystickId, info: JoystickInfo) -> Self {
        Self {
            id,
            raw_buttons: vec![false; info.button_count],
            raw_axes: vec![0.0; info.axis_count],
            hats: vec![JoystickHat::Centered; info.hat_count],
            info,
            buttons: HashSet::new(),
            axes: [0.0; GamepadAxis::ALL.len()],
        }
    }
}

/// Stores `value` at `index`, growing `list` if a device reports more
/// inputs than it said it had.
fn set_at<T: Clone + Default>(list: &mut Vec<T>, index: usize, value: T) {
    if index >= list.len() {
        list.resize(index + 1, T::default());
    }
    list[index] = value;
}

/// A finger that is on the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TouchState {
    pub id: TouchId,
    pub position: (f32, f32),
    pub pressure: f32,
}

pub(crate) struct InputState {
    pub keys: HashSet<Key>,
    pub mouse_position: (f32, f32),
    pub mouse_buttons: HashSet<MouseButton>,
    pub mouse_visible: bool,
    pub mouse_relative: bool,
    pub mouse_grabbed: bool,
    pub cursor: smitten_core::Cursor,
    /// The image cursor being shown, kept alive while it is.
    pub image_cursor: Option<crate::mouse::ImageCursor>,
    /// In the order they were connected.
    pub joysticks: Vec<JoystickState>,
    /// In the order they touched the screen.
    pub touches: Vec<TouchState>,
    /// Whether held keys send repeated `key_pressed` callbacks.
    pub key_repeat: bool,
    /// GUIDs of the gamepad mappings the game added, in order.
    pub added_mappings: Vec<String>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            keys: HashSet::new(),
            mouse_position: (0.0, 0.0),
            mouse_buttons: HashSet::new(),
            mouse_visible: true,
            mouse_relative: false,
            mouse_grabbed: false,
            cursor: smitten_core::Cursor::Arrow,
            image_cursor: None,
            joysticks: Vec::new(),
            touches: Vec::new(),
            key_repeat: false,
            added_mappings: Vec::new(),
        }
    }
}

impl InputState {
    pub fn joystick(&self, id: JoystickId) -> Option<&JoystickState> {
        self.joysticks.iter().find(|joystick| joystick.id == id)
    }

    pub fn touch(&self, id: TouchId) -> Option<&TouchState> {
        self.touches.iter().find(|touch| touch.id == id)
    }

    fn joystick_mut(&mut self, id: JoystickId) -> Option<&mut JoystickState> {
        self.joysticks.iter_mut().find(|joystick| joystick.id == id)
    }

    /// Updates the tracked state for one event.
    ///
    /// This runs before the game's callback for the event, so a state query
    /// made inside the callback already reflects it.
    pub fn apply(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::KeyDown { key, .. } => {
                self.keys.insert(*key);
            }
            BackendEvent::KeyUp { key } => {
                self.keys.remove(key);
            }
            BackendEvent::MouseMoved { x, y, .. } => self.mouse_position = (*x, *y),
            BackendEvent::MousePressed { x, y, button, .. } => {
                self.mouse_position = (*x, *y);
                self.mouse_buttons.insert(*button);
            }
            BackendEvent::MouseReleased { x, y, button, .. } => {
                self.mouse_position = (*x, *y);
                self.mouse_buttons.remove(button);
            }
            BackendEvent::TouchPressed {
                id, x, y, pressure, ..
            }
            | BackendEvent::TouchMoved {
                id, x, y, pressure, ..
            } => {
                let touch = TouchState {
                    id: *id,
                    position: (*x, *y),
                    pressure: *pressure,
                };
                // A move for a finger that was never pressed, which can
                // happen if it touched down before the window opened,
                // starts tracking it.
                match self.touches.iter_mut().find(|known| known.id == *id) {
                    Some(known) => *known = touch,
                    None => self.touches.push(touch),
                }
            }
            BackendEvent::TouchReleased { id, .. } => {
                self.touches.retain(|touch| touch.id != *id);
            }
            BackendEvent::JoystickAdded { id, info } => {
                if self.joystick(*id).is_none() {
                    self.joysticks.push(JoystickState::new(*id, info.clone()));
                }
            }
            BackendEvent::JoystickRemoved { id } => {
                self.joysticks.retain(|joystick| joystick.id != *id);
            }
            BackendEvent::GamepadPressed { id, button } => {
                if let Some(joystick) = self.joystick_mut(*id) {
                    joystick.buttons.insert(*button);
                }
            }
            BackendEvent::GamepadReleased { id, button } => {
                if let Some(joystick) = self.joystick_mut(*id) {
                    joystick.buttons.remove(button);
                }
            }
            BackendEvent::GamepadAxis { id, axis, value } => {
                if let Some(joystick) = self.joystick_mut(*id) {
                    joystick.axes[axis.index()] = *value;
                }
            }
            BackendEvent::JoystickPressed { id, button }
            | BackendEvent::JoystickReleased { id, button } => {
                let down = matches!(event, BackendEvent::JoystickPressed { .. });
                if let Some(joystick) = self.joystick_mut(*id) {
                    set_at(&mut joystick.raw_buttons, *button, down);
                }
            }
            BackendEvent::JoystickAxis { id, axis, value } => {
                if let Some(joystick) = self.joystick_mut(*id) {
                    set_at(&mut joystick.raw_axes, *axis, *value);
                }
            }
            BackendEvent::JoystickHat { id, hat, direction } => {
                if let Some(joystick) = self.joystick_mut(*id) {
                    set_at(&mut joystick.hats, *hat, *direction);
                }
            }
            BackendEvent::Quit
            | BackendEvent::Resized { .. }
            | BackendEvent::TextInput(_)
            | BackendEvent::TextEdited { .. }
            | BackendEvent::WheelMoved { .. }
            | BackendEvent::Focus(_)
            | BackendEvent::MouseFocus(_)
            | BackendEvent::Visible(_) => {}
        }
    }
}

/// Shortens `(x, y)` to a length of 1 if it is longer.
pub fn limit_length(x: f32, y: f32) -> (f32, f32) {
    let length = x.hypot(y);
    if length > 1.0 {
        (x / length, y / length)
    } else {
        (x, y)
    }
}

/// Reads a stick with a dead zone: nothing while it is pushed less than
/// `dead_zone`, then growing smoothly from zero to a full push, never
/// longer than 1.
pub fn stick_with_dead_zone(x: f32, y: f32, dead_zone: f32) -> (f32, f32) {
    let length = x.hypot(y);
    let dead_zone = dead_zone.clamp(0.0, 0.99);
    if length <= dead_zone || length == 0.0 {
        return (0.0, 0.0);
    }
    let pushed = ((length - dead_zone) / (1.0 - dead_zone)).min(1.0);
    (x / length * pushed, y / length * pushed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stick_inside_its_dead_zone_reads_as_nothing() {
        assert_eq!(stick_with_dead_zone(0.1, -0.1, 0.2), (0.0, 0.0));
        assert_eq!(stick_with_dead_zone(0.0, 0.0, 0.0), (0.0, 0.0));
        // Just past the edge is a small push, not a jump to 0.2.
        let (x, y) = stick_with_dead_zone(0.21, 0.0, 0.2);
        assert!(x > 0.0 && x < 0.02 && y == 0.0);
        // Halfway between the edge and a full push is half.
        let (x, _) = stick_with_dead_zone(-0.6, 0.0, 0.2);
        assert!((x + 0.5).abs() < 1e-6);
        // A corner of a square stick is still no longer than 1.
        let (x, y) = stick_with_dead_zone(1.0, 1.0, 0.2);
        assert!((x.hypot(y) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_direction_is_never_longer_than_one() {
        assert_eq!(limit_length(0.5, 0.0), (0.5, 0.0));
        let (x, y) = limit_length(1.0, -1.0);
        assert!((x.hypot(y) - 1.0).abs() < 1e-6 && x > 0.0 && y < 0.0);
    }

    const PAD: JoystickId = JoystickId(3);

    fn connected() -> InputState {
        let mut input = InputState::default();
        input.apply(&BackendEvent::JoystickAdded {
            id: PAD,
            info: pad_info(),
        });
        input
    }

    fn pad_info() -> JoystickInfo {
        JoystickInfo {
            name: "Test Pad".to_string(),
            axis_count: 2,
            button_count: 4,
            hat_count: 1,
            ..JoystickInfo::default()
        }
    }

    #[test]
    fn keys_are_held_between_down_and_up() {
        let mut input = InputState::default();
        input.apply(&BackendEvent::KeyDown {
            key: Key::Space,
            is_repeat: false,
        });
        assert!(input.keys.contains(&Key::Space));

        input.apply(&BackendEvent::KeyUp { key: Key::Space });
        assert!(!input.keys.contains(&Key::Space));
    }

    #[test]
    fn mouse_clicks_update_position_and_buttons() {
        let mut input = InputState::default();
        input.apply(&BackendEvent::MousePressed {
            x: 40.0,
            y: 25.0,
            button: MouseButton::Left,
            presses: 1,
        });
        assert_eq!(input.mouse_position, (40.0, 25.0));
        assert!(input.mouse_buttons.contains(&MouseButton::Left));

        input.apply(&BackendEvent::MouseReleased {
            x: 41.0,
            y: 25.0,
            button: MouseButton::Left,
            presses: 1,
        });
        assert_eq!(input.mouse_position, (41.0, 25.0));
        assert!(input.mouse_buttons.is_empty());
    }

    fn touch_at(id: u64, x: f32, y: f32) -> BackendEvent {
        BackendEvent::TouchPressed {
            id: TouchId(id),
            x,
            y,
            dx: 0.0,
            dy: 0.0,
            pressure: 0.5,
        }
    }

    #[test]
    fn touches_are_tracked_from_press_to_release_in_order() {
        let mut input = InputState::default();
        input.apply(&touch_at(7, 10.0, 20.0));
        input.apply(&touch_at(2, 30.0, 40.0));
        input.apply(&BackendEvent::TouchMoved {
            id: TouchId(7),
            x: 12.0,
            y: 21.0,
            dx: 2.0,
            dy: 1.0,
            pressure: 0.75,
        });

        let ids: Vec<_> = input.touches.iter().map(|touch| touch.id).collect();
        assert_eq!(ids, [TouchId(7), TouchId(2)]);
        let moved = input.touch(TouchId(7)).expect("the finger is down");
        assert_eq!(moved.position, (12.0, 21.0));
        assert_eq!(moved.pressure, 0.75);

        input.apply(&BackendEvent::TouchReleased {
            id: TouchId(7),
            x: 12.0,
            y: 21.0,
            dx: 0.0,
            dy: 0.0,
            pressure: 0.0,
        });
        assert!(input.touch(TouchId(7)).is_none());
        assert_eq!(input.touches.len(), 1);
    }

    #[test]
    fn a_move_without_a_press_starts_tracking_the_finger() {
        let mut input = InputState::default();
        input.apply(&BackendEvent::TouchMoved {
            id: TouchId(1),
            x: 5.0,
            y: 6.0,
            dx: 1.0,
            dy: 1.0,
            pressure: 1.0,
        });
        assert_eq!(input.touch(TouchId(1)).unwrap().position, (5.0, 6.0));
    }

    #[test]
    fn gamepad_buttons_and_axes_are_tracked_per_joystick() {
        let mut input = connected();
        input.apply(&BackendEvent::GamepadPressed {
            id: PAD,
            button: GamepadButton::A,
        });
        input.apply(&BackendEvent::GamepadAxis {
            id: PAD,
            axis: GamepadAxis::LeftX,
            value: -0.5,
        });

        let pad = input.joystick(PAD).expect("the pad is connected");
        assert_eq!(pad.info.name, "Test Pad");
        assert!(pad.buttons.contains(&GamepadButton::A));
        assert_eq!(pad.axes[GamepadAxis::LeftX.index()], -0.5);
        assert_eq!(pad.axes[GamepadAxis::LeftY.index()], 0.0);
    }

    #[test]
    fn removing_a_joystick_forgets_its_state() {
        let mut input = connected();
        input.apply(&BackendEvent::GamepadPressed {
            id: PAD,
            button: GamepadButton::A,
        });
        input.apply(&BackendEvent::JoystickRemoved { id: PAD });

        assert!(input.joystick(PAD).is_none());
    }

    #[test]
    fn input_from_an_unknown_joystick_is_ignored() {
        let mut input = InputState::default();
        input.apply(&BackendEvent::GamepadPressed {
            id: PAD,
            button: GamepadButton::A,
        });

        assert!(input.joysticks.is_empty());
    }

    #[test]
    fn a_joystick_reported_twice_is_listed_once() {
        let mut input = connected();
        input.apply(&BackendEvent::JoystickAdded {
            id: PAD,
            info: pad_info(),
        });

        assert_eq!(input.joysticks.len(), 1);
    }

    #[test]
    fn raw_buttons_axes_and_hats_are_tracked_per_joystick() {
        let mut input = connected();
        let pad = input.joystick(PAD).unwrap();
        assert_eq!(pad.raw_buttons, [false; 4]);
        assert_eq!(pad.raw_axes, [0.0; 2]);
        assert_eq!(pad.hats, [JoystickHat::Centered]);

        input.apply(&BackendEvent::JoystickPressed { id: PAD, button: 2 });
        input.apply(&BackendEvent::JoystickAxis {
            id: PAD,
            axis: 1,
            value: 0.25,
        });
        input.apply(&BackendEvent::JoystickHat {
            id: PAD,
            hat: 0,
            direction: JoystickHat::LeftUp,
        });
        let pad = input.joystick(PAD).unwrap();
        assert_eq!(pad.raw_buttons, [false, false, true, false]);
        assert_eq!(pad.raw_axes, [0.0, 0.25]);
        assert_eq!(pad.hats, [JoystickHat::LeftUp]);

        input.apply(&BackendEvent::JoystickReleased { id: PAD, button: 2 });
        assert!(!input.joystick(PAD).unwrap().raw_buttons[2]);
    }

    #[test]
    fn inputs_beyond_the_reported_counts_are_still_tracked() {
        let mut input = connected();
        input.apply(&BackendEvent::JoystickPressed { id: PAD, button: 6 });
        let pad = input.joystick(PAD).unwrap();
        assert_eq!(pad.raw_buttons.len(), 7);
        assert!(pad.raw_buttons[6]);
    }

    #[test]
    fn hat_bits_round_trip_and_opposites_cancel() {
        for hat in [
            JoystickHat::Centered,
            JoystickHat::Up,
            JoystickHat::Right,
            JoystickHat::Down,
            JoystickHat::Left,
            JoystickHat::RightUp,
            JoystickHat::RightDown,
            JoystickHat::LeftUp,
            JoystickHat::LeftDown,
        ] {
            assert_eq!(JoystickHat::from_bits(hat.bits()), hat);
        }
        assert_eq!(JoystickHat::from_bits(1 | 4), JoystickHat::Centered);
        assert_eq!(JoystickHat::from_bits(1 | 4 | 2), JoystickHat::Right);
    }
}
