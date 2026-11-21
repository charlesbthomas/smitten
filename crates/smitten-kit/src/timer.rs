//! Cooldowns, countdowns and a scheduler that hands back events.
//!
//! [`Cooldown`] and [`Countdown`] replace the `f32` fields games keep for
//! "seconds until the gun can fire again" and "seconds of invulnerability
//! left". [`Timers`] schedules events for later; its
//! [`update`](Timers::update) returns the events that are due instead of
//! running callbacks, so acting on them can borrow the game freely.
//!
//! ```
//! use smitten_kit::timer::Timers;
//!
//! #[derive(Clone, Debug, PartialEq)]
//! enum Due {
//!     Spawn,
//!     Blink,
//! }
//!
//! let mut timers = Timers::new();
//! timers.after(0.9, Due::Spawn);
//! timers.every(0.25, Due::Blink);
//!
//! let due: Vec<Due> = timers.update(0.5).collect();
//! assert_eq!(due, [Due::Blink, Due::Blink]);
//! let due: Vec<Due> = timers.update(0.5).collect();
//! assert_eq!(due, [Due::Blink, Due::Spawn, Due::Blink]); // at 0.75, 0.9, 1.0
//! ```

/// Something that can be used again after a delay: a gun, a dash, a
/// spell.
///
/// ```
/// use smitten_kit::timer::Cooldown;
///
/// let mut gun = Cooldown::new(0.1);
/// assert!(gun.try_use());
/// assert!(!gun.try_use());
/// gun.tick(0.1);
/// assert!(gun.try_use());
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cooldown {
    duration: f32,
    left: f32,
}

impl Cooldown {
    /// A cooldown of `seconds` that starts ready.
    pub fn new(seconds: f32) -> Self {
        Cooldown {
            duration: seconds.max(0.0),
            left: 0.0,
        }
    }

    /// Counts the cooldown down by `dt` seconds.
    ///
    /// Time keeps counting below zero until the next use (to at most one
    /// full cooldown), so a held fire button keeps an exact rate even
    /// when the frame time does not divide it.
    pub fn tick(&mut self, dt: f32) {
        self.left = (self.left - dt).max(-self.duration);
    }

    /// Whether it can be used now.
    pub fn is_ready(&self) -> bool {
        self.left <= 0.0
    }

    /// If ready, starts the cooldown and returns true.
    pub fn try_use(&mut self) -> bool {
        if !self.is_ready() {
            return false;
        }
        self.left += self.duration;
        // A use after a long idle starts a full cooldown, not a short one.
        if self.left <= 0.0 {
            self.left = self.duration;
        }
        true
    }

    /// Starts the cooldown whether or not it was ready.
    pub fn trigger(&mut self) {
        self.left = self.duration;
    }

    /// Makes it ready at once.
    pub fn reset(&mut self) {
        self.left = 0.0;
    }

    /// Seconds until it is ready, or 0.0 when it is.
    pub fn left(&self) -> f32 {
        self.left.max(0.0)
    }

    /// The full cooldown in seconds.
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Changes the full cooldown, as an upgrade that fires faster would.
    pub fn set_duration(&mut self, seconds: f32) {
        self.duration = seconds.max(0.0);
        self.left = self.left.min(self.duration);
    }

    /// 0.0 just used, 1.0 ready, for drawing a recharge meter.
    pub fn progress(&self) -> f32 {
        if self.duration <= 0.0 {
            1.0
        } else {
            1.0 - self.left() / self.duration
        }
    }
}

/// A one-shot timer kept in a field: invulnerability, a hit flash, a
/// banner that stays up for a moment.
///
/// ```
/// use smitten_kit::timer::Countdown;
///
/// let mut flash = Countdown::default();
/// flash.start(0.2);
/// assert!(!flash.tick(0.1));
/// assert_eq!(flash.fraction_left(), 0.5); // the flash's strength
/// assert!(flash.tick(0.1)); // true on the frame it runs out
/// assert!(!flash.is_running());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Countdown {
    duration: f32,
    left: f32,
}

impl Countdown {
    /// A countdown already running for `seconds`.
    pub fn running(seconds: f32) -> Self {
        let mut countdown = Countdown::default();
        countdown.start(seconds);
        countdown
    }

    /// Starts (or restarts) the countdown at `seconds`.
    pub fn start(&mut self, seconds: f32) {
        self.duration = seconds.max(0.0);
        self.left = self.duration;
    }

    /// Restarts it at `seconds` unless more than that is already left.
    pub fn extend_to(&mut self, seconds: f32) {
        if seconds > self.left {
            self.start(seconds);
        }
    }

    /// Stops it without finishing.
    pub fn stop(&mut self) {
        self.left = 0.0;
    }

    /// Counts down by `dt` seconds. Returns true on the one frame it
    /// reaches zero.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.left <= 0.0 {
            return false;
        }
        self.left -= dt;
        if self.left <= 0.0 {
            self.left = 0.0;
            return true;
        }
        false
    }

    /// Whether there is time left.
    pub fn is_running(&self) -> bool {
        self.left > 0.0
    }

    /// Seconds left.
    pub fn left(&self) -> f32 {
        self.left
    }

    /// Seconds since it started, up to its full length.
    pub fn elapsed(&self) -> f32 {
        self.duration - self.left
    }

    /// 1.0 just started, 0.0 done. Handy as an alpha or a flash strength.
    pub fn fraction_left(&self) -> f32 {
        if self.duration <= 0.0 {
            0.0
        } else {
            self.left / self.duration
        }
    }
}

/// Names one scheduled event, to cancel it or ask how long it has left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimerId(u64);

#[derive(Debug)]
struct Entry<E> {
    id: TimerId,
    left: f32,
    every: Option<f32>,
    /// Firings to go, for a repeating timer. `None` repeats forever.
    times: Option<u32>,
    event: E,
}

/// Events scheduled for later, returned from [`update`](Timers::update)
/// when they are due.
///
/// `E` is usually an enum of the game's own. Repeating timers clone their
/// event each time they fire.
#[derive(Debug)]
pub struct Timers<E> {
    entries: Vec<Entry<E>>,
    next_id: u64,
    due: Vec<(f32, u64, E)>,
}

impl<E> Default for Timers<E> {
    fn default() -> Self {
        Timers {
            entries: Vec::new(),
            next_id: 0,
            due: Vec::new(),
        }
    }
}

impl<E> Timers<E> {
    /// No timers.
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, left: f32, every: Option<f32>, times: Option<u32>, event: E) -> TimerId {
        let id = TimerId(self.next_id);
        self.next_id += 1;
        self.entries.push(Entry {
            id,
            left: left.max(0.0),
            every,
            times,
            event,
        });
        id
    }

    /// Fires `event` once, `seconds` from now.
    pub fn after(&mut self, seconds: f32, event: E) -> TimerId {
        self.add(seconds, None, None, event)
    }

    /// Cancels the timer. Returns false if it had already fired or been
    /// cancelled.
    pub fn cancel(&mut self, id: TimerId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != before
    }

    /// Seconds until the timer next fires, or `None` once it is gone.
    pub fn left(&self, id: TimerId) -> Option<f32> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.left)
    }

    /// Whether the timer is still waiting to fire.
    pub fn contains(&self, id: TimerId) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }

    /// How many timers are waiting.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no timers are waiting.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Cancels every timer.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl<E: Clone> Timers<E> {
    /// Fires `event` every `seconds`, starting `seconds` from now.
    pub fn every(&mut self, seconds: f32, event: E) -> TimerId {
        let seconds = seconds.max(f32::EPSILON);
        self.add(seconds, Some(seconds), None, event)
    }

    /// Fires `event` every `seconds`, `times` times.
    pub fn every_times(&mut self, seconds: f32, times: u32, event: E) -> TimerId {
        let seconds = seconds.max(f32::EPSILON);
        if times == 0 {
            // Nothing to fire; hand back an id that is already gone.
            let id = TimerId(self.next_id);
            self.next_id += 1;
            return id;
        }
        self.add(seconds, Some(seconds), Some(times), event)
    }

    /// Moves time on by `dt` seconds and yields the events that came due,
    /// in the order they were due. A long frame that crosses two ticks of
    /// an `every` timer yields both.
    pub fn update(&mut self, dt: f32) -> impl Iterator<Item = E> + '_ {
        let dt = dt.max(0.0);
        self.due.clear();
        let mut index = 0;
        while index < self.entries.len() {
            let entry = &mut self.entries[index];
            // `when` is how far into this frame each firing happens.
            let mut when = entry.left;
            let mut finished = false;
            while when <= dt {
                self.due.push((when, entry.id.0, entry.event.clone()));
                match entry.every {
                    Some(every) => {
                        if let Some(times) = &mut entry.times {
                            *times -= 1;
                            if *times == 0 {
                                finished = true;
                                break;
                            }
                        }
                        when += every;
                    }
                    None => {
                        finished = true;
                        break;
                    }
                }
            }
            if finished {
                self.entries.remove(index);
            } else {
                entry.left = when - dt;
                index += 1;
            }
        }
        // Ties go to the timer scheduled first.
        self.due
            .sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        self.due.drain(..).map(|(_, _, event)| event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_keeps_an_exact_rate() {
        // A 0.375 s gun on 0.25 s frames: carrying the leftover time over
        // fires 8 times in 3 s, as the rate says. Restarting from zero
        // each shot would round every gap up to two frames and fire 6.
        let mut gun = Cooldown::new(0.375);
        let mut shots = 0;
        for _ in 0..12 {
            if gun.try_use() {
                shots += 1;
            }
            gun.tick(0.25);
        }
        assert_eq!(shots, 8);
    }

    #[test]
    fn cooldown_after_idle_is_full() {
        let mut dash = Cooldown::new(1.0);
        dash.tick(10.0);
        assert!(dash.try_use());
        assert_eq!(dash.left(), 1.0);
        assert_eq!(dash.progress(), 0.0);
    }

    #[test]
    fn countdown_extend_keeps_the_longer() {
        let mut c = Countdown::running(1.0);
        c.extend_to(0.5);
        assert_eq!(c.left(), 1.0);
        c.extend_to(2.0);
        assert_eq!(c.left(), 2.0);
    }

    #[test]
    fn cancel_and_left() {
        let mut timers = Timers::new();
        let a = timers.after(1.0, 'a');
        let b = timers.after(2.0, 'b');
        assert!(timers.cancel(a));
        assert!(!timers.cancel(a));
        let fired: Vec<char> = timers.update(1.5).collect();
        assert!(fired.is_empty());
        assert_eq!(timers.left(b), Some(0.5));
        let fired: Vec<char> = timers.update(0.5).collect();
        assert_eq!(fired, ['b']);
        assert!(timers.is_empty());
    }

    #[test]
    fn every_times_stops() {
        let mut timers = Timers::new();
        let id = timers.every_times(0.1, 3, ());
        assert_eq!(timers.update(1.0).count(), 3);
        assert!(!timers.contains(id));
    }

    #[test]
    fn events_come_in_due_order() {
        let mut timers = Timers::new();
        timers.after(0.3, "late");
        timers.after(0.1, "early");
        timers.every(0.2, "tick");
        let fired: Vec<&str> = timers.update(0.45).collect();
        assert_eq!(fired, ["early", "tick", "late", "tick"]);
    }
}
