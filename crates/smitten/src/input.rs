//! Tracks what is currently held down or connected, from the stream of
//! backend events. The `keyboard`, `mouse` and `joystick` modules answer
//! their state queries from here.

use std::collections::HashSet;

use smitten_core::backend::BackendEvent;
use smitten_core::{GamepadAxis, GamepadButton, JoystickId, Key, MouseButton};

pub(crate) struct JoystickState {
    pub id: JoystickId,
    pub name: String,
    pub buttons: HashSet<GamepadButton>,
    pub axes: [f32; GamepadAxis::ALL.len()],
}

pub(crate) struct InputState {
    pub keys: HashSet<Key>,
    pub mouse_position: (f32, f32),
    pub mouse_buttons: HashSet<MouseButton>,
    pub mouse_visible: bool,
    pub mouse_relative: bool,
    pub mouse_grabbed: bool,
    pub cursor: smitten_core::Cursor,
    /// In the order they were connected.
    pub joysticks: Vec<JoystickState>,
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
            joysticks: Vec::new(),
        }
    }
}

impl InputState {
    pub fn joystick(&self, id: JoystickId) -> Option<&JoystickState> {
        self.joysticks.iter().find(|joystick| joystick.id == id)
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
            BackendEvent::JoystickAdded { id, name } => {
                if self.joystick(*id).is_none() {
                    self.joysticks.push(JoystickState {
                        id: *id,
                        name: name.clone(),
                        buttons: HashSet::new(),
                        axes: [0.0; GamepadAxis::ALL.len()],
                    });
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
            BackendEvent::Quit
            | BackendEvent::Resized { .. }
            | BackendEvent::TextInput(_)
            | BackendEvent::WheelMoved { .. }
            | BackendEvent::Focus(_)
            | BackendEvent::MouseFocus(_)
            | BackendEvent::Visible(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAD: JoystickId = JoystickId(3);

    fn connected() -> InputState {
        let mut input = InputState::default();
        input.apply(&BackendEvent::JoystickAdded {
            id: PAD,
            name: "Test Pad".to_string(),
        });
        input
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
        assert_eq!(pad.name, "Test Pad");
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
            name: "Test Pad".to_string(),
        });

        assert_eq!(input.joysticks.len(), 1);
    }
}
