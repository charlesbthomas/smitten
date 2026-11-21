//! Named actions bound to keys, mouse buttons and gamepads.
//!
//! A game names what the player wants as an enum of its own (`Jump`,
//! `Fire`, `Pause`) and binds any number of inputs to each.
//! [`Actions`] then answers "was Jump pressed this frame?" whichever
//! device the player is using, merges analog sticks with keys, remembers
//! which device was used last for button prompts, and rebinds.
//!
//! `Actions` is fed from events, so it needs no window and works in
//! tests. Each frame:
//!
//! 1. pass every event to [`Actions::handle`],
//! 2. call [`Actions::update`] once,
//! 3. read [`is_pressed`](Actions::is_pressed), [`axis`](Actions::axis)
//!    and the rest.
//!
//! With [`scene::run`](crate::scene::run), steps 1 and 2 go in the
//! context's [`event`](crate::scene::Context::event) and
//! [`update`](crate::scene::Context::update) hooks, which run before the
//! scenes'.
//!
//! ```
//! use smitten::event::Event;
//! use smitten::{GamepadAxis, GamepadButton, Key};
//! use smitten_kit::input::{Actions, Binding};
//!
//! #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
//! enum Action {
//!     Up,
//!     Down,
//!     Jump,
//! }
//!
//! let mut actions = Actions::new();
//! actions
//!     .bind(Action::Up, [Binding::Key(Key::W), Binding::stick_negative(GamepadAxis::LeftY)])
//!     .bind(Action::Down, [Binding::Key(Key::S), Binding::stick_positive(GamepadAxis::LeftY)])
//!     .bind(Action::Jump, [Binding::Key(Key::Space), Binding::Button(GamepadButton::A)]);
//!
//! actions.handle(&Event::KeyPressed { key: Key::Space, is_repeat: false });
//! actions.update(1.0 / 60.0);
//! assert!(actions.is_pressed(Action::Jump));
//!
//! actions.set_gamepad_axis(GamepadAxis::LeftY, 1.0);
//! actions.update(1.0 / 60.0);
//! assert!(!actions.is_pressed(Action::Jump)); // only on the first frame
//! assert!(actions.is_down(Action::Jump));
//! assert_eq!(actions.axis(Action::Up, Action::Down), 1.0);
//! ```

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::str::FromStr;

use smitten::event::Event;
use smitten::{GamepadAxis, GamepadButton, Joystick, Key, MouseButton, ParseNameError, Vec2};

/// The threshold [`Binding::stick_positive`] and
/// [`Binding::stick_negative`] use: a stick pushed less than this far
/// counts as centered.
pub const STICK_DEAD_ZONE: f32 = 0.2;

/// The threshold [`Binding::trigger`] uses.
pub const TRIGGER_DEAD_ZONE: f32 = 0.1;

/// One input that can drive an action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Binding {
    /// A key, by what it types in the player's layout.
    Key(Key),
    /// A mouse button.
    Mouse(MouseButton),
    /// A gamepad button.
    Button(GamepadButton),
    /// A stick or trigger pushed past `threshold` in one direction. It
    /// acts as a button, and how far past the threshold it is pushed is
    /// the action's analog [`value`](Actions::value).
    Axis {
        /// Which stick axis or trigger.
        axis: GamepadAxis,
        /// Pushed toward positive values (right, down, pulled) or negative
        /// ones (left, up).
        positive: bool,
        /// From 0.0 to 1.0: how far it must be pushed to count.
        threshold: f32,
    },
}

impl Binding {
    /// A stick axis pushed toward positive values: right on an X axis,
    /// down on a Y axis.
    pub fn stick_positive(axis: GamepadAxis) -> Binding {
        Binding::Axis {
            axis,
            positive: true,
            threshold: STICK_DEAD_ZONE,
        }
    }

    /// A stick axis pushed toward negative values: left on an X axis, up
    /// on a Y axis.
    pub fn stick_negative(axis: GamepadAxis) -> Binding {
        Binding::Axis {
            axis,
            positive: false,
            threshold: STICK_DEAD_ZONE,
        }
    }

    /// A trigger pulled.
    pub fn trigger(axis: GamepadAxis) -> Binding {
        Binding::Axis {
            axis,
            positive: true,
            threshold: TRIGGER_DEAD_ZONE,
        }
    }

    /// Which kind of device the binding is on.
    pub fn device(&self) -> Device {
        match self {
            Binding::Key(_) => Device::Keyboard,
            Binding::Mouse(_) => Device::Mouse,
            Binding::Button(_) | Binding::Axis { .. } => Device::Gamepad,
        }
    }
}

impl From<Key> for Binding {
    fn from(key: Key) -> Binding {
        Binding::Key(key)
    }
}

impl From<MouseButton> for Binding {
    fn from(button: MouseButton) -> Binding {
        Binding::Mouse(button)
    }
}

impl From<GamepadButton> for Binding {
    fn from(button: GamepadButton) -> Binding {
        Binding::Button(button)
    }
}

const MOUSE_NAMES: [(MouseButton, &str); 5] = [
    (MouseButton::Left, "left"),
    (MouseButton::Right, "right"),
    (MouseButton::Middle, "middle"),
    (MouseButton::X1, "x1"),
    (MouseButton::X2, "x2"),
];

const BUTTON_NAMES: [(GamepadButton, &str); 15] = [
    (GamepadButton::A, "a"),
    (GamepadButton::B, "b"),
    (GamepadButton::X, "x"),
    (GamepadButton::Y, "y"),
    (GamepadButton::Back, "back"),
    (GamepadButton::Guide, "guide"),
    (GamepadButton::Start, "start"),
    (GamepadButton::LeftStick, "leftstick"),
    (GamepadButton::RightStick, "rightstick"),
    (GamepadButton::LeftShoulder, "leftshoulder"),
    (GamepadButton::RightShoulder, "rightshoulder"),
    (GamepadButton::DpadUp, "dpup"),
    (GamepadButton::DpadDown, "dpdown"),
    (GamepadButton::DpadLeft, "dpleft"),
    (GamepadButton::DpadRight, "dpright"),
];

const AXIS_NAMES: [(GamepadAxis, &str); 6] = [
    (GamepadAxis::LeftX, "leftx"),
    (GamepadAxis::LeftY, "lefty"),
    (GamepadAxis::RightX, "rightx"),
    (GamepadAxis::RightY, "righty"),
    (GamepadAxis::TriggerLeft, "triggerleft"),
    (GamepadAxis::TriggerRight, "triggerright"),
];

fn name_of<T: PartialEq + Copy>(table: &[(T, &'static str)], value: T) -> &'static str {
    table
        .iter()
        .find(|(item, _)| *item == value)
        .map_or("?", |(_, name)| name)
}

fn from_name<T: Copy>(table: &[(T, &'static str)], name: &str) -> Option<T> {
    table
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(item, _)| *item)
}

/// Writes a binding as text that [`FromStr`] reads back, for settings
/// files: `key:space`, `mouse:left`, `pad:a`, `axis:lefty-` or, with a
/// threshold other than the default, `axis:triggerright+0.5`.
impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Binding::Key(key) => write!(f, "key:{key}"),
            Binding::Mouse(button) => write!(f, "mouse:{}", name_of(&MOUSE_NAMES, button)),
            Binding::Button(button) => write!(f, "pad:{}", name_of(&BUTTON_NAMES, button)),
            Binding::Axis {
                axis,
                positive,
                threshold,
            } => {
                let sign = if positive { '+' } else { '-' };
                write!(f, "axis:{}{sign}", name_of(&AXIS_NAMES, axis))?;
                let default = match axis {
                    GamepadAxis::TriggerLeft | GamepadAxis::TriggerRight => TRIGGER_DEAD_ZONE,
                    _ => STICK_DEAD_ZONE,
                };
                if threshold != default {
                    write!(f, "{threshold}")?;
                }
                Ok(())
            }
        }
    }
}

impl FromStr for Binding {
    type Err = ParseNameError;

    fn from_str(text: &str) -> Result<Binding, ParseNameError> {
        let error = || ParseNameError::new("binding", text);
        let (kind, name) = text.split_once(':').ok_or_else(error)?;
        match kind {
            "key" => name.parse().map(Binding::Key).map_err(|_| error()),
            "mouse" => from_name(&MOUSE_NAMES, name)
                .map(Binding::Mouse)
                .ok_or_else(error),
            "pad" => from_name(&BUTTON_NAMES, name)
                .map(Binding::Button)
                .ok_or_else(error),
            "axis" => {
                let split = name.find(['+', '-']).ok_or_else(error)?;
                let axis = from_name(&AXIS_NAMES, &name[..split]).ok_or_else(error)?;
                let positive = name[split..].starts_with('+');
                let rest = &name[split + 1..];
                let mut binding = if positive {
                    match axis {
                        GamepadAxis::TriggerLeft | GamepadAxis::TriggerRight => {
                            Binding::trigger(axis)
                        }
                        _ => Binding::stick_positive(axis),
                    }
                } else {
                    Binding::stick_negative(axis)
                };
                if !rest.is_empty() {
                    let value: f32 = rest.parse().map_err(|_| error())?;
                    if let Binding::Axis { threshold, .. } = &mut binding {
                        *threshold = value.clamp(0.0, 0.99);
                    }
                }
                Ok(binding)
            }
            _ => Err(error()),
        }
    }
}

/// A kind of input device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Device {
    /// The keyboard.
    Keyboard,
    /// The mouse.
    Mouse,
    /// A gamepad.
    Gamepad,
}

#[derive(Clone, Copy, Debug, Default)]
struct ActionState {
    down: bool,
    pressed: bool,
    released: bool,
    held: f32,
    value: f32,
    pending_press: bool,
    pending_release: bool,
}

/// Which gamepad an [`Actions`] reads.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PadChoice {
    /// Whichever pad was last used.
    Any,
    /// Only this one.
    Only(Joystick),
    /// None at all, as for a keyboard-only second player.
    Nothing,
}

/// The state of every action a game defines. See the [module docs](self).
#[derive(Debug)]
pub struct Actions<A> {
    bindings: HashMap<A, Vec<Binding>>,
    states: HashMap<A, ActionState>,
    keys: Vec<Key>,
    mouse: Vec<MouseButton>,
    buttons: Vec<GamepadButton>,
    axes: [f32; 6],
    pad_choice: PadChoice,
    pad: Option<Joystick>,
    last_device: Device,
    rebind: Option<(A, Vec<Device>)>,
    rebound: Option<(A, Binding)>,
}

impl<A: Copy + Eq + Hash> Default for Actions<A> {
    fn default() -> Self {
        Actions {
            bindings: HashMap::new(),
            states: HashMap::new(),
            keys: Vec::new(),
            mouse: Vec::new(),
            buttons: Vec::new(),
            axes: [0.0; 6],
            pad_choice: PadChoice::Any,
            pad: None,
            last_device: Device::Keyboard,
            rebind: None,
            rebound: None,
        }
    }
}

impl<A: Copy + Eq + Hash> Actions<A> {
    /// No bindings, reading whichever gamepad is used.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `bindings` to `action`, keeping the ones it has.
    pub fn bind(
        &mut self,
        action: A,
        bindings: impl IntoIterator<Item = impl Into<Binding>>,
    ) -> &mut Self {
        let list = self.bindings.entry(action).or_default();
        for binding in bindings {
            let binding = binding.into();
            if !list.contains(&binding) {
                list.push(binding);
            }
        }
        self.states.entry(action).or_default();
        self
    }

    /// Removes every binding from `action`.
    pub fn unbind(&mut self, action: A) {
        self.bindings.remove(&action);
    }

    /// Replaces the bindings of `action` that are on `device` with
    /// `binding`, as a settings screen does.
    pub fn rebind(&mut self, action: A, binding: Binding) {
        let list = self.bindings.entry(action).or_default();
        list.retain(|b| b.device() != binding.device());
        list.push(binding);
        self.states.entry(action).or_default();
    }

    /// The inputs bound to `action`.
    pub fn bindings(&self, action: A) -> &[Binding] {
        self.bindings.get(&action).map_or(&[], Vec::as_slice)
    }

    /// Every action with its bindings, for saving them.
    pub fn all_bindings(&self) -> impl Iterator<Item = (A, &[Binding])> {
        self.bindings
            .iter()
            .map(|(action, list)| (*action, list.as_slice()))
    }

    /// Reads only `pad`, or no gamepad at all with `None`. Two players on
    /// one machine each get an `Actions` with their own pad.
    pub fn set_gamepad(&mut self, pad: Option<Joystick>) {
        self.pad_choice = match pad {
            Some(pad) => PadChoice::Only(pad),
            None => PadChoice::Nothing,
        };
        self.pad = pad;
        self.release_gamepad();
    }

    /// Goes back to reading whichever gamepad was used last, the default.
    pub fn use_any_gamepad(&mut self) {
        self.pad_choice = PadChoice::Any;
    }

    /// The gamepad being read: the one set with
    /// [`set_gamepad`](Self::set_gamepad), or the last one used.
    pub fn gamepad(&self) -> Option<&Joystick> {
        self.pad.as_ref()
    }

    /// The kind of device the player used last, for showing the right
    /// button prompts.
    pub fn last_device(&self) -> Device {
        self.last_device
    }

    fn accepts(&mut self, joystick: &Joystick) -> bool {
        match self.pad_choice {
            PadChoice::Only(pad) => pad == *joystick,
            PadChoice::Nothing => false,
            PadChoice::Any => {
                if self.pad != Some(*joystick) {
                    self.release_gamepad();
                    self.pad = Some(*joystick);
                }
                true
            }
        }
    }

    fn release_gamepad(&mut self) {
        for button in std::mem::take(&mut self.buttons) {
            self.mark(Binding::Button(button), false);
        }
        for axis in GamepadAxis::ALL {
            self.set_gamepad_axis_quietly(axis, 0.0);
        }
    }

    /// Takes in one event. Pass every event, before calling
    /// [`update`](Self::update) for the frame.
    pub fn handle(&mut self, event: &Event) {
        match event {
            Event::KeyPressed {
                key,
                is_repeat: false,
            } => self.press(Binding::Key(*key)),
            Event::KeyReleased { key } => self.release(Binding::Key(*key)),
            Event::MousePressed(click) => self.press(Binding::Mouse(click.button)),
            Event::MouseReleased(click) => self.release(Binding::Mouse(click.button)),
            Event::MouseMoved(_) => self.last_device = Device::Mouse,
            Event::GamepadPressed { joystick, button } => {
                if self.accepts(joystick) {
                    self.press(Binding::Button(*button));
                }
            }
            Event::GamepadReleased { joystick, button } => {
                if self.accepts(joystick) {
                    self.release(Binding::Button(*button));
                }
            }
            Event::GamepadAxis {
                joystick,
                axis,
                value,
            } => {
                if self.accepts(joystick) {
                    self.set_gamepad_axis(*axis, *value);
                }
            }
            Event::JoystickRemoved(joystick) => {
                if self.pad == Some(*joystick) {
                    self.release_gamepad();
                    if self.pad_choice == PadChoice::Any {
                        self.pad = None;
                    }
                }
            }
            // Keys held while the window lost focus never report their
            // release, so let go of everything.
            Event::Focus(false) => self.release_all(),
            _ => {}
        }
    }

    /// Presses `binding`, as an event would. For on-screen buttons,
    /// replays and tests.
    pub fn press(&mut self, binding: Binding) {
        self.last_device = binding.device();
        if self.capture(binding) {
            return;
        }
        let held = match binding {
            Binding::Key(key) => push_unique(&mut self.keys, key),
            Binding::Mouse(button) => push_unique(&mut self.mouse, button),
            Binding::Button(button) => push_unique(&mut self.buttons, button),
            Binding::Axis { .. } => true,
        };
        if held {
            self.mark(binding, true);
        }
    }

    /// Releases `binding`, as an event would.
    pub fn release(&mut self, binding: Binding) {
        let was_held = match binding {
            Binding::Key(key) => remove(&mut self.keys, key),
            Binding::Mouse(button) => remove(&mut self.mouse, button),
            Binding::Button(button) => remove(&mut self.buttons, button),
            Binding::Axis { .. } => true,
        };
        if was_held {
            self.mark(binding, false);
        }
    }

    /// Moves a gamepad axis, as an event would. `value` runs from -1.0 to
    /// 1.0 for sticks and 0.0 to 1.0 for triggers.
    pub fn set_gamepad_axis(&mut self, axis: GamepadAxis, value: f32) {
        if value.abs() > 0.5 {
            self.last_device = Device::Gamepad;
            if self.capture(Binding::Axis {
                axis,
                positive: value > 0.0,
                threshold: 0.5,
            }) {
                return;
            }
        }
        self.set_gamepad_axis_quietly(axis, value);
    }

    fn set_gamepad_axis_quietly(&mut self, axis: GamepadAxis, value: f32) {
        let old = std::mem::replace(&mut self.axes[axis.index()], value.clamp(-1.0, 1.0));
        let new = self.axes[axis.index()];
        // Crossing a binding's threshold counts as a press or a release,
        // even if it crosses back before the next update.
        for (action, list) in &self.bindings {
            for binding in list {
                if let Binding::Axis {
                    axis: bound,
                    positive,
                    threshold,
                } = *binding
                    && bound == axis
                {
                    let past = |v: f32| (if positive { v } else { -v }) > threshold;
                    let state = self.states.entry(*action).or_default();
                    if !past(old) && past(new) {
                        state.pending_press = true;
                    } else if past(old) && !past(new) {
                        state.pending_release = true;
                    }
                }
            }
        }
    }

    /// Lets go of every key, button and stick.
    pub fn release_all(&mut self) {
        for key in std::mem::take(&mut self.keys) {
            self.mark(Binding::Key(key), false);
        }
        for button in std::mem::take(&mut self.mouse) {
            self.mark(Binding::Mouse(button), false);
        }
        self.release_gamepad();
    }

    fn mark(&mut self, input: Binding, press: bool) {
        for (action, list) in &self.bindings {
            if list.contains(&input) {
                let state = self.states.entry(*action).or_default();
                if press {
                    state.pending_press = true;
                } else {
                    state.pending_release = true;
                }
            }
        }
    }

    fn value_of(&self, binding: &Binding) -> f32 {
        let on = |held: bool| if held { 1.0 } else { 0.0 };
        match *binding {
            Binding::Key(key) => on(self.keys.contains(&key)),
            Binding::Mouse(button) => on(self.mouse.contains(&button)),
            Binding::Button(button) => on(self.buttons.contains(&button)),
            Binding::Axis {
                axis,
                positive,
                threshold,
            } => {
                let v = self.axes[axis.index()];
                let pushed = if positive { v } else { -v };
                if pushed <= threshold {
                    0.0
                } else {
                    // Rescaled so the value ramps up from the threshold.
                    ((pushed - threshold) / (1.0 - threshold).max(f32::EPSILON)).clamp(0.0, 1.0)
                }
            }
        }
    }

    /// Rolls the frame over: works out what is down, pressed and released
    /// from the events handled since the last update. Call it once per
    /// frame, after handling that frame's events and before reading.
    pub fn update(&mut self, dt: f32) {
        let mut values = Vec::with_capacity(self.bindings.len());
        for (action, list) in &self.bindings {
            let value = list.iter().map(|b| self.value_of(b)).fold(0.0, f32::max);
            values.push((*action, value));
        }
        for (action, value) in values {
            let state = self.states.entry(action).or_default();
            let down = value > 0.0;
            state.pressed = state.pending_press || (down && !state.down);
            state.released = state.pending_release || (!down && state.down);
            state.held = match (state.down, down) {
                (true, true) => state.held + dt,
                _ => 0.0,
            };
            state.down = down;
            state.value = value;
            state.pending_press = false;
            state.pending_release = false;
        }
    }

    fn state(&self, action: A) -> ActionState {
        self.states.get(&action).copied().unwrap_or_default()
    }

    /// Whether any input bound to `action` is held.
    pub fn is_down(&self, action: A) -> bool {
        self.state(action).down
    }

    /// Whether `action` went down this frame. A tap that went down and up
    /// between two frames still counts.
    pub fn is_pressed(&self, action: A) -> bool {
        self.state(action).pressed
    }

    /// Whether `action` was let go this frame.
    pub fn is_released(&self, action: A) -> bool {
        self.state(action).released
    }

    /// Seconds `action` has been held, or 0.0 when it is up.
    pub fn held_for(&self, action: A) -> f32 {
        self.state(action).held
    }

    /// From 0.0 to 1.0: 1.0 for a held key or button, how far a stick or
    /// trigger is pushed, and the largest of them when several are held.
    pub fn value(&self, action: A) -> f32 {
        self.state(action).value
    }

    /// `value(positive) - value(negative)`, from -1.0 to 1.0.
    pub fn axis(&self, negative: A, positive: A) -> f32 {
        (self.value(positive) - self.value(negative)).clamp(-1.0, 1.0)
    }

    /// A movement vector no longer than 1. A keyboard diagonal is as fast
    /// as a straight line, and a stick keeps its analog speed.
    pub fn vector(&self, left: A, right: A, up: A, down: A) -> Vec2 {
        let v = Vec2::new(self.axis(left, right), self.axis(up, down));
        if v.length_squared() > 1.0 {
            v.normalize()
        } else {
            v
        }
    }

    /// Waits for the next key, button or stick push on one of `devices`
    /// and binds it to `action` in place of its bindings on that device.
    /// The input that finishes the rebind does not also press anything.
    pub fn start_rebind(&mut self, action: A, devices: &[Device]) {
        self.rebind = Some((action, devices.to_vec()));
        self.rebound = None;
    }

    /// Whether a rebind is waiting for input.
    pub fn is_rebinding(&self) -> bool {
        self.rebind.is_some()
    }

    /// The action and its new binding, on the one call after a rebind
    /// finished.
    pub fn rebind_finished(&mut self) -> Option<(A, Binding)> {
        self.rebound.take()
    }

    /// Stops waiting for a rebind.
    pub fn cancel_rebind(&mut self) {
        self.rebind = None;
    }

    /// Finishes a waiting rebind with `binding`. Returns true if it did.
    fn capture(&mut self, binding: Binding) -> bool {
        let Some((action, devices)) = &self.rebind else {
            return false;
        };
        if !devices.contains(&binding.device()) {
            return false;
        }
        let action = *action;
        let binding = match binding {
            Binding::Axis { axis, positive, .. } => match axis {
                GamepadAxis::TriggerLeft | GamepadAxis::TriggerRight => Binding::trigger(axis),
                _ if positive => Binding::stick_positive(axis),
                _ => Binding::stick_negative(axis),
            },
            other => other,
        };
        self.rebind = None;
        self.rebind(action, binding);
        self.rebound = Some((action, binding));
        // Hold the key so its release is not taken for a stray release.
        match binding {
            Binding::Key(key) => {
                push_unique(&mut self.keys, key);
            }
            Binding::Mouse(button) => {
                push_unique(&mut self.mouse, button);
            }
            Binding::Button(button) => {
                push_unique(&mut self.buttons, button);
            }
            Binding::Axis { .. } => {}
        }
        // The input is held but must not read as a fresh press.
        let state = self.states.entry(action).or_default();
        state.down = true;
        state.pending_press = false;
        true
    }
}

fn push_unique<T: PartialEq>(list: &mut Vec<T>, item: T) -> bool {
    if list.contains(&item) {
        false
    } else {
        list.push(item);
        true
    }
}

fn remove<T: PartialEq>(list: &mut Vec<T>, item: T) -> bool {
    let before = list.len();
    list.retain(|x| *x != item);
    list.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    enum Act {
        Left,
        Right,
        Up,
        Down,
        Fire,
    }

    fn actions() -> Actions<Act> {
        let mut a = Actions::new();
        a.bind(
            Act::Left,
            [
                Binding::Key(Key::A),
                Binding::stick_negative(GamepadAxis::LeftX),
            ],
        )
        .bind(
            Act::Right,
            [
                Binding::Key(Key::D),
                Binding::stick_positive(GamepadAxis::LeftX),
            ],
        )
        .bind(Act::Up, [Binding::Key(Key::W)])
        .bind(Act::Down, [Binding::Key(Key::S)])
        .bind(
            Act::Fire,
            [
                Binding::Key(Key::Space),
                Binding::Mouse(MouseButton::Left),
                Binding::trigger(GamepadAxis::TriggerRight),
            ],
        );
        a
    }

    #[test]
    fn pressed_held_released() {
        let mut a = actions();
        a.press(Key::Space.into());
        a.update(0.1);
        assert!(a.is_pressed(Act::Fire) && a.is_down(Act::Fire));
        a.update(0.1);
        assert!(!a.is_pressed(Act::Fire) && a.is_down(Act::Fire));
        assert!((a.held_for(Act::Fire) - 0.1).abs() < 1e-6);
        a.release(Key::Space.into());
        a.update(0.1);
        assert!(a.is_released(Act::Fire) && !a.is_down(Act::Fire));
        assert_eq!(a.held_for(Act::Fire), 0.0);
    }

    #[test]
    fn a_tap_inside_one_frame_counts() {
        let mut a = actions();
        a.press(Key::Space.into());
        a.release(Key::Space.into());
        a.update(0.1);
        assert!(a.is_pressed(Act::Fire));
        assert!(a.is_released(Act::Fire));
        assert!(!a.is_down(Act::Fire));
    }

    #[test]
    fn two_bindings_held_release_only_when_both_up() {
        let mut a = actions();
        a.press(Key::Space.into());
        a.press(MouseButton::Left.into());
        a.update(0.1);
        a.release(Key::Space.into());
        a.update(0.1);
        assert!(a.is_down(Act::Fire));
        assert!(!a.is_pressed(Act::Fire));
    }

    #[test]
    fn key_repeat_is_ignored() {
        let mut a = actions();
        a.handle(&Event::KeyPressed {
            key: Key::Space,
            is_repeat: false,
        });
        a.update(0.1);
        a.handle(&Event::KeyPressed {
            key: Key::Space,
            is_repeat: true,
        });
        a.update(0.1);
        assert!(!a.is_pressed(Act::Fire));
    }

    #[test]
    fn focus_loss_lets_go() {
        let mut a = actions();
        a.press(Key::Space.into());
        a.update(0.1);
        a.handle(&Event::Focus(false));
        a.update(0.1);
        assert!(a.is_released(Act::Fire) && !a.is_down(Act::Fire));
    }

    #[test]
    fn diagonals_are_not_faster() {
        let mut a = actions();
        a.press(Key::D.into());
        a.press(Key::S.into());
        a.update(0.1);
        let v = a.vector(Act::Left, Act::Right, Act::Up, Act::Down);
        assert!((v.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn sticks_are_analog_past_the_dead_zone() {
        let mut a = actions();
        a.set_gamepad_axis(GamepadAxis::LeftX, 0.1);
        a.update(0.1);
        assert_eq!(a.axis(Act::Left, Act::Right), 0.0);
        a.set_gamepad_axis(GamepadAxis::LeftX, -0.6);
        a.update(0.1);
        assert!((a.axis(Act::Left, Act::Right) + 0.5).abs() < 1e-5);
        assert!(a.is_pressed(Act::Left));
        assert_eq!(a.last_device(), Device::Gamepad);
    }

    #[test]
    fn rebinding_takes_the_next_input_and_does_not_fire() {
        let mut a = actions();
        a.start_rebind(Act::Fire, &[Device::Keyboard]);
        a.press(MouseButton::Left.into()); // wrong device: an ordinary press
        a.update(0.1);
        assert!(a.is_rebinding());
        a.release(MouseButton::Left.into());
        a.press(Key::F.into());
        a.update(0.1);
        assert_eq!(a.rebind_finished(), Some((Act::Fire, Binding::Key(Key::F))));
        assert!(!a.is_rebinding());
        assert!(!a.bindings(Act::Fire).contains(&Binding::Key(Key::Space)));
        assert!(
            a.bindings(Act::Fire)
                .contains(&Binding::Mouse(MouseButton::Left))
        );
        a.release(Key::F.into());
        a.update(0.1);
        a.press(Key::F.into());
        a.update(0.1);
        assert!(a.is_pressed(Act::Fire));
    }

    #[test]
    fn bindings_round_trip_as_text() {
        let all = [
            Binding::Key(Key::Space),
            Binding::Key(Key::Return),
            Binding::Mouse(MouseButton::X2),
            Binding::Button(GamepadButton::DpadLeft),
            Binding::stick_negative(GamepadAxis::LeftY),
            Binding::trigger(GamepadAxis::TriggerRight),
            Binding::Axis {
                axis: GamepadAxis::RightX,
                positive: true,
                threshold: 0.5,
            },
        ];
        for binding in all {
            let text = binding.to_string();
            assert_eq!(text.parse::<Binding>(), Ok(binding), "{text}");
        }
        assert_eq!(
            Binding::stick_negative(GamepadAxis::LeftY).to_string(),
            "axis:lefty-"
        );
        assert!("pad:z".parse::<Binding>().is_err());
        assert!("space".parse::<Binding>().is_err());
    }
}
