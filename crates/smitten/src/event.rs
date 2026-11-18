//! Game-level events, the equivalent of `love.event`.

use std::any::Any;
use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;

use smitten_core::backend::BackendEvent;

use crate::context::{self, Context};
use crate::{
    DisplayOrientation, GamepadAxis, GamepadButton, Joystick, JoystickHat, Key, MouseButton,
    TouchId,
};

/// Asks the game to close. [`crate::Game::quit`] gets a chance to refuse.
pub fn quit() {
    context::with(|ctx| ctx.quit_requested = true);
}

/// A value sent with [`push`], handed to [`crate::Game::custom_event`].
///
/// It can hold a value of any type, so a game usually defines an enum of
/// its own events and asks for that back with [`CustomEvent::downcast`]
/// or [`CustomEvent::downcast_ref`].
pub struct CustomEvent(Box<dyn Any>);

impl CustomEvent {
    /// Wraps `value` as an event. [`push`] does this for you.
    pub fn new<T: Any>(value: T) -> CustomEvent {
        CustomEvent(Box::new(value))
    }

    /// Whether the event holds a `T`.
    pub fn is<T: Any>(&self) -> bool {
        self.0.is::<T>()
    }

    /// The value, if it is a `T`.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }

    /// Takes the value out if it is a `T`, or gives the event back if it
    /// is not.
    pub fn downcast<T: Any>(self) -> Result<T, CustomEvent> {
        self.0.downcast().map(|value| *value).map_err(CustomEvent)
    }
}

impl fmt::Debug for CustomEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CustomEvent { .. }")
    }
}

/// Something that happened, as [`poll`] and [`wait`] hand it over. Each
/// variant matches the [`crate::Game`] callback of the same name, with the
/// same values, already in units.
#[derive(Debug)]
pub enum Event {
    /// The player or the system asked the game to close. Taken by
    /// [`poll`], it no longer reaches [`crate::Game::quit`]; call [`quit`]
    /// to pass it on.
    Quit,
    Resized {
        width: u32,
        height: u32,
    },
    KeyPressed {
        key: Key,
        is_repeat: bool,
    },
    KeyReleased {
        key: Key,
    },
    TextInput(String),
    TextEdited {
        text: String,
        start: usize,
        length: usize,
    },
    MouseMoved {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
    },
    MousePressed {
        x: f32,
        y: f32,
        button: MouseButton,
        presses: u32,
    },
    MouseReleased {
        x: f32,
        y: f32,
        button: MouseButton,
        presses: u32,
    },
    WheelMoved {
        dx: f32,
        dy: f32,
    },
    TouchPressed {
        id: TouchId,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        pressure: f32,
    },
    TouchMoved {
        id: TouchId,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        pressure: f32,
    },
    TouchReleased {
        id: TouchId,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        pressure: f32,
    },
    JoystickAdded(Joystick),
    JoystickRemoved(Joystick),
    GamepadPressed {
        joystick: Joystick,
        button: GamepadButton,
    },
    GamepadReleased {
        joystick: Joystick,
        button: GamepadButton,
    },
    GamepadAxis {
        joystick: Joystick,
        axis: GamepadAxis,
        value: f32,
    },
    JoystickPressed {
        joystick: Joystick,
        button: usize,
    },
    JoystickReleased {
        joystick: Joystick,
        button: usize,
    },
    JoystickAxis {
        joystick: Joystick,
        axis: usize,
        value: f32,
    },
    JoystickHat {
        joystick: Joystick,
        hat: usize,
        direction: JoystickHat,
    },
    Focus(bool),
    MouseFocus(bool),
    Visible(bool),
    FileDropped {
        path: PathBuf,
        x: f32,
        y: f32,
    },
    DirectoryDropped {
        path: PathBuf,
        x: f32,
        y: f32,
    },
    LowMemory,
    DisplayRotated(DisplayOrientation),
    /// An event the game sent with [`push`].
    Custom(CustomEvent),
}

impl Event {
    /// Turns a backend event, already in units, into a game event.
    fn from_backend(event: BackendEvent) -> Event {
        match event {
            BackendEvent::Quit => Event::Quit,
            BackendEvent::Resized { width, height } => Event::Resized { width, height },
            BackendEvent::KeyDown { key, is_repeat } => Event::KeyPressed { key, is_repeat },
            BackendEvent::KeyUp { key } => Event::KeyReleased { key },
            BackendEvent::TextInput(text) => Event::TextInput(text),
            BackendEvent::TextEdited {
                text,
                start,
                length,
            } => Event::TextEdited {
                text,
                start,
                length,
            },
            BackendEvent::MouseMoved { x, y, dx, dy } => Event::MouseMoved { x, y, dx, dy },
            BackendEvent::MousePressed {
                x,
                y,
                button,
                presses,
            } => Event::MousePressed {
                x,
                y,
                button,
                presses,
            },
            BackendEvent::MouseReleased {
                x,
                y,
                button,
                presses,
            } => Event::MouseReleased {
                x,
                y,
                button,
                presses,
            },
            BackendEvent::WheelMoved { dx, dy } => Event::WheelMoved { dx, dy },
            BackendEvent::TouchPressed {
                id,
                x,
                y,
                dx,
                dy,
                pressure,
            } => Event::TouchPressed {
                id,
                x,
                y,
                dx,
                dy,
                pressure,
            },
            BackendEvent::TouchMoved {
                id,
                x,
                y,
                dx,
                dy,
                pressure,
            } => Event::TouchMoved {
                id,
                x,
                y,
                dx,
                dy,
                pressure,
            },
            BackendEvent::TouchReleased {
                id,
                x,
                y,
                dx,
                dy,
                pressure,
            } => Event::TouchReleased {
                id,
                x,
                y,
                dx,
                dy,
                pressure,
            },
            BackendEvent::JoystickAdded { id, .. } => Event::JoystickAdded(Joystick::new(id)),
            BackendEvent::JoystickRemoved { id } => Event::JoystickRemoved(Joystick::new(id)),
            BackendEvent::GamepadPressed { id, button } => Event::GamepadPressed {
                joystick: Joystick::new(id),
                button,
            },
            BackendEvent::GamepadReleased { id, button } => Event::GamepadReleased {
                joystick: Joystick::new(id),
                button,
            },
            BackendEvent::GamepadAxis { id, axis, value } => Event::GamepadAxis {
                joystick: Joystick::new(id),
                axis,
                value,
            },
            BackendEvent::JoystickPressed { id, button } => Event::JoystickPressed {
                joystick: Joystick::new(id),
                button,
            },
            BackendEvent::JoystickReleased { id, button } => Event::JoystickReleased {
                joystick: Joystick::new(id),
                button,
            },
            BackendEvent::JoystickAxis { id, axis, value } => Event::JoystickAxis {
                joystick: Joystick::new(id),
                axis,
                value,
            },
            BackendEvent::JoystickHat { id, hat, direction } => Event::JoystickHat {
                joystick: Joystick::new(id),
                hat,
                direction,
            },
            BackendEvent::Focus(focused) => Event::Focus(focused),
            BackendEvent::MouseFocus(focused) => Event::MouseFocus(focused),
            BackendEvent::Visible(visible) => Event::Visible(visible),
            BackendEvent::FileDropped { path, x, y } => Event::FileDropped { path, x, y },
            BackendEvent::DirectoryDropped { path, x, y } => Event::DirectoryDropped { path, x, y },
            BackendEvent::LowMemory => Event::LowMemory,
            BackendEvent::DisplayRotated(orientation) => Event::DisplayRotated(orientation),
        }
    }
}

/// Events waiting to be handed to the game.
#[derive(Default)]
pub(crate) struct EventQueue {
    /// Pushed by the game with [`push`].
    pending: VecDeque<CustomEvent>,
    /// Taken from the platform by [`pump`] and not yet handed over.
    platform: VecDeque<Event>,
}

impl EventQueue {
    /// Takes the events pushed so far. Events pushed while these are
    /// being handled wait for the next frame, so a handler that pushes
    /// an event cannot keep a frame from ending.
    pub fn take(&mut self) -> VecDeque<CustomEvent> {
        std::mem::take(&mut self.pending)
    }

    /// The next event taken from the platform and not yet handed over.
    pub fn next_platform(&mut self) -> Option<Event> {
        self.platform.pop_front()
    }
}

/// Records a platform event in the input state and turns it into a game
/// event. A key repeat is dropped here while repeats are off, as LÖVE
/// never reports them then.
fn accept(ctx: &mut Context, event: BackendEvent) -> Option<Event> {
    let event = crate::localize(ctx, event);
    ctx.input.apply(&event);
    if let BackendEvent::KeyDown {
        is_repeat: true, ..
    } = event
        && !ctx.input.key_repeat
    {
        return None;
    }
    Some(Event::from_backend(event))
}

/// Takes everything that has happened on the platform, such as key presses
/// and mouse movement, into the game's queue, the equivalent of
/// `love.event.pump`. The keyboard, mouse and joystick states are updated
/// straight away.
///
/// Smitten pumps at the start of every frame and hands what it finds to
/// the [`crate::Game`] callbacks, so most games never call this. It is for
/// a game that is busy for a long time inside one callback, such as while
/// loading, and wants to see input meanwhile with [`poll`].
pub fn pump() {
    context::with(|ctx| {
        while let Some(event) = ctx.backend.poll_event() {
            if let Some(event) = accept(ctx, event) {
                ctx.events.platform.push_back(event);
            }
        }
    });
}

/// Takes every event in the queue, the equivalent of `love.event.poll`:
/// those from the platform that [`pump`] collected, then those sent with
/// [`push`]. Call [`pump`] first to collect what is new.
///
/// Events taken this way are not handed to the [`crate::Game`] callbacks,
/// as in LÖVE, where `poll` is how `love.run` finds them. Taking
/// [`Event::Quit`] keeps the game from closing unless it calls [`quit`].
///
/// ```no_run
/// use smitten::event::{self, Event};
/// use smitten::Key;
///
/// # fn load_next_chunk() -> bool { false }
/// // A long load that stops early when Escape is pressed.
/// while load_next_chunk() {
///     event::pump();
///     for event in event::poll() {
///         match event {
///             Event::KeyPressed { key: Key::Escape, .. } => return,
///             Event::Quit => event::quit(),
///             _ => {}
///         }
///     }
/// }
/// ```
pub fn poll() -> impl Iterator<Item = Event> {
    let events: Vec<Event> = context::with(|ctx| {
        let platform = std::mem::take(&mut ctx.events.platform);
        let pushed = ctx.events.take();
        platform
            .into_iter()
            .chain(pushed.into_iter().map(Event::Custom))
            .collect()
    });
    events.into_iter()
}

/// Waits until something happens and takes it from the queue, the
/// equivalent of `love.event.wait`. `timeout` is in seconds, and `None`
/// waits as long as it takes. Returns `None` if the time ran out first.
///
/// The game stands still meanwhile: nothing is updated or drawn. As with
/// [`poll`], the event is not handed to the [`crate::Game`] callbacks.
pub fn wait(timeout: Option<f32>) -> Option<Event> {
    let queued = context::with(|ctx| {
        ctx.events
            .platform
            .pop_front()
            .or_else(|| ctx.events.pending.pop_front().map(Event::Custom))
    });
    if queued.is_some() {
        return queued;
    }
    let deadline = timeout.map(|seconds| {
        std::time::Instant::now() + std::time::Duration::from_secs_f32(seconds.max(0.0))
    });
    loop {
        let left = deadline.map(|deadline| {
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .as_millis()
                .min(u128::from(u32::MAX)) as u32
        });
        let event = context::with(|ctx| ctx.backend.wait_event(left))?;
        if let Some(event) = context::with(|ctx| accept(ctx, event)) {
            return Some(event);
        }
        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            return None;
        }
    }
}

/// Sends an event to the game, the equivalent of `love.event.push`.
///
/// [`crate::Game::custom_event`] receives it at the start of the next
/// frame, after the events from the platform, such as key presses, and
/// before `update`. Events arrive in the order they were pushed.
///
/// ```no_run
/// use smitten::event::{self, CustomEvent};
/// use smitten::Game;
///
/// enum Signal {
///     LevelComplete(u32),
///     PlayerDied,
/// }
///
/// struct MyGame {
///     level: u32,
/// }
///
/// impl Game for MyGame {
///     fn load() -> smitten::Result<Self> {
///         Ok(MyGame { level: 1 })
///     }
///
///     fn update(&mut self, _dt: f32) {
///         event::push(Signal::LevelComplete(self.level));
///     }
///
///     fn custom_event(&mut self, event: CustomEvent) {
///         match event.downcast::<Signal>() {
///             Ok(Signal::LevelComplete(level)) => self.level = level + 1,
///             Ok(Signal::PlayerDied) => self.level = 1,
///             Err(_) => {}
///         }
///     }
/// }
/// ```
pub fn push<T: Any>(event: T) {
    context::with(|ctx| ctx.events.pending.push_back(CustomEvent::new(event)));
}

/// Throws away the events pushed with [`push`] that have not been handed
/// to the game yet, the equivalent of `love.event.clear` for the game's
/// own events.
pub fn clear() {
    context::with(|ctx| ctx.events.pending.clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum Signal {
        Won(u32),
    }

    #[test]
    fn an_event_gives_back_the_value_it_holds() {
        let event = CustomEvent::new(Signal::Won(3));
        assert!(event.is::<Signal>());
        assert!(!event.is::<u32>());
        assert_eq!(event.downcast_ref::<Signal>(), Some(&Signal::Won(3)));
        assert_eq!(event.downcast_ref::<String>(), None);
        let event = event.downcast::<String>().unwrap_err();
        assert_eq!(event.downcast::<Signal>().unwrap(), Signal::Won(3));
    }

    #[test]
    fn the_queue_hands_over_events_in_order_and_empties() {
        let mut queue = EventQueue::default();
        queue.pending.push_back(CustomEvent::new(1_u32));
        queue.pending.push_back(CustomEvent::new(2_u32));
        let taken: Vec<u32> = queue
            .take()
            .into_iter()
            .map(|event| event.downcast().unwrap())
            .collect();
        assert_eq!(taken, [1, 2]);
        assert!(queue.take().is_empty());
    }
}
