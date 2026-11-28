//! [`Events`]: a queue that one part of a frame fills and another
//! drains.
//!
//! The kit's timers and sequences hand back events instead of running
//! callbacks; this is the same idea for the game's own code. Collision
//! code sends `Hit { .. }` while it holds the world mutably, and the code
//! that plays sounds, shakes the camera and adds score drains the queue
//! afterwards, with nothing borrowed.
//!
//! ```
//! use smitten_kit_core::events::Events;
//!
//! #[derive(Debug, PartialEq)]
//! enum Happened {
//!     Hit { damage: u32 },
//!     Died,
//! }
//!
//! let mut events = Events::new();
//! events.send(Happened::Hit { damage: 3 });
//! events.send(Happened::Died);
//!
//! let mut score = 0;
//! for event in events.drain() {
//!     if let Happened::Hit { damage } = event {
//!         score += damage;
//!     }
//! }
//! assert_eq!(score, 3);
//! assert!(events.is_empty());
//! ```

/// A first-in, first-out queue of `E`. See the [module docs](self).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Events<E> {
    queue: Vec<E>,
}

impl<E> Default for Events<E> {
    fn default() -> Self {
        Events { queue: Vec::new() }
    }
}

impl<E> Events<E> {
    /// An empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an event at the back.
    pub fn send(&mut self, event: E) {
        self.queue.push(event);
    }

    /// Takes every event out, oldest first. The queue keeps its memory
    /// for the next frame.
    pub fn drain(&mut self) -> std::vec::Drain<'_, E> {
        self.queue.drain(..)
    }

    /// Takes every event out as a `Vec`, leaving the queue empty, so the
    /// handler may send more while it works through them.
    pub fn take(&mut self) -> Vec<E> {
        std::mem::take(&mut self.queue)
    }

    /// The waiting events, oldest first, without taking them.
    pub fn iter(&self) -> std::slice::Iter<'_, E> {
        self.queue.iter()
    }

    /// How many are waiting.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether none are waiting.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Drops every waiting event.
    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

impl<E> Extend<E> for Events<E> {
    fn extend<I: IntoIterator<Item = E>>(&mut self, events: I) {
        self.queue.extend(events);
    }
}

impl<'a, E> IntoIterator for &'a Events<E> {
    type Item = &'a E;
    type IntoIter = std::slice::Iter<'a, E>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_lets_handlers_send_more() {
        let mut events: Events<u32> = Events::new();
        events.extend([1, 2]);
        let mut seen = Vec::new();
        while !events.is_empty() {
            for n in events.take() {
                seen.push(n);
                if n < 3 {
                    events.send(n + 2);
                }
            }
        }
        assert_eq!(seen, [1, 2, 3, 4]);
    }
}
