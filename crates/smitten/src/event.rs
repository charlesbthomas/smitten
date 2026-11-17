//! Game-level events, the equivalent of `love.event`.

use std::any::Any;
use std::collections::VecDeque;
use std::fmt;

use crate::context;

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

/// Events pushed by the game and not yet handed back to it.
#[derive(Default)]
pub(crate) struct EventQueue {
    pending: VecDeque<CustomEvent>,
}

impl EventQueue {
    /// Takes the events pushed so far. Events pushed while these are
    /// being handled wait for the next frame, so a handler that pushes
    /// an event cannot keep a frame from ending.
    pub fn take(&mut self) -> VecDeque<CustomEvent> {
        std::mem::take(&mut self.pending)
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
///     fn load() -> Self {
///         MyGame { level: 1 }
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
