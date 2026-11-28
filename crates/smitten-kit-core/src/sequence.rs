//! Scripted runs of waits and events, for wave banners, tutorials and
//! cutscenes.
//!
//! A [`Sequence`] is built once and then stepped each frame. Like
//! [`Timers`](crate::timer::Timers) it hands back events rather than
//! running callbacks, so the game acts on them with full access to its
//! own state.
//!
//! ```
//! use smitten_kit_core::sequence::Sequence;
//!
//! #[derive(Debug, PartialEq)]
//! enum Beat {
//!     BannerIn,
//!     BannerOut,
//!     Spawn(u32),
//! }
//!
//! let mut wave = Sequence::new()
//!     .emit(Beat::BannerIn)
//!     .wait(1.5)
//!     .emit(Beat::BannerOut)
//!     .wait(0.5)
//!     .emit(Beat::Spawn(1))
//!     .wait(0.25)
//!     .emit(Beat::Spawn(2));
//!
//! assert_eq!(wave.update(0.0).collect::<Vec<_>>(), [Beat::BannerIn]);
//! assert_eq!(wave.update(2.0).collect::<Vec<_>>(), [Beat::BannerOut, Beat::Spawn(1)]);
//! // Skipping at triple speed plays the rest quickly instead of cutting it.
//! wave.set_speed(3.0);
//! assert_eq!(wave.update(0.1).collect::<Vec<_>>(), [Beat::Spawn(2)]);
//! assert!(wave.is_finished());
//! ```

use crate::Float;

#[derive(Debug)]
enum Step<E, F: Float> {
    Wait(F),
    Emit(Option<E>),
    Together(Vec<Sequence<E, F>>),
}

/// Waits and events played in order. See the [module docs](self).
#[derive(Debug)]
pub struct Sequence<E, F: Float = f32> {
    steps: Vec<Step<E, F>>,
    cursor: usize,
    speed: F,
    out: Vec<E>,
}

impl<E, F: Float> Default for Sequence<E, F> {
    fn default() -> Self {
        Sequence {
            steps: Vec::new(),
            cursor: 0,
            speed: F::ONE,
            out: Vec::new(),
        }
    }
}

impl<E, F: Float> Sequence<E, F> {
    /// An empty sequence, already finished.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a pause of `seconds`.
    pub fn wait(mut self, seconds: F) -> Self {
        self.steps.push(Step::Wait(seconds.max(F::ZERO)));
        self
    }

    /// Adds an event.
    pub fn emit(mut self, event: E) -> Self {
        self.steps.push(Step::Emit(Some(event)));
        self
    }

    /// Adds sequences that play side by side. The sequence carries on
    /// once the longest of them ends.
    pub fn together(mut self, sequences: impl IntoIterator<Item = Sequence<E, F>>) -> Self {
        self.steps
            .push(Step::Together(sequences.into_iter().collect()));
        self
    }

    /// Adds every step of `other` at the end.
    pub fn then(mut self, other: Sequence<E, F>) -> Self {
        self.steps
            .extend(other.steps.into_iter().skip(other.cursor));
        self
    }

    /// Whether every step has played.
    pub fn is_finished(&self) -> bool {
        self.cursor >= self.steps.len()
    }

    /// How fast the sequence plays: 1.0 is normal, 3.0 triple speed.
    pub fn speed(&self) -> F {
        self.speed
    }

    /// Plays faster or slower, as a "hold to fast-forward" does.
    pub fn set_speed(&mut self, speed: F) {
        self.speed = speed.max(F::ZERO);
    }

    /// Plays on by `dt` seconds (times the speed) and yields the events
    /// that came due, in order.
    pub fn update(&mut self, dt: F) -> impl Iterator<Item = E> + '_ {
        let mut out = std::mem::take(&mut self.out);
        out.clear();
        self.advance(dt.max(F::ZERO) * self.speed, &mut out);
        self.out = out;
        self.out.drain(..)
    }

    /// Plays every remaining step at once and yields their events.
    pub fn skip(&mut self) -> impl Iterator<Item = E> + '_ {
        self.update(F::INFINITY)
    }

    /// Plays on by `dt`, pushing events to `out`. Returns the time left
    /// over once it finishes, or 0.0 while it is still playing.
    fn advance(&mut self, mut dt: F, out: &mut Vec<E>) -> F {
        while let Some(step) = self.steps.get_mut(self.cursor) {
            match step {
                Step::Wait(left) => {
                    if *left > dt {
                        *left -= dt;
                        return F::ZERO;
                    }
                    dt -= *left;
                    *left = F::ZERO;
                }
                Step::Emit(event) => {
                    if let Some(event) = event.take() {
                        out.push(event);
                    }
                }
                Step::Together(sequences) => {
                    let mut leftover = F::INFINITY;
                    let mut all_done = true;
                    for sequence in sequences.iter_mut() {
                        let left = sequence.advance(dt, out);
                        if sequence.is_finished() {
                            leftover = leftover.min(left);
                        } else {
                            all_done = false;
                        }
                    }
                    if !all_done {
                        return F::ZERO;
                    }
                    dt = if leftover.is_finite() { leftover } else { dt };
                }
            }
            self.cursor += 1;
        }
        dt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_finished() {
        let mut s = Sequence::<()>::new();
        assert!(s.is_finished());
        assert_eq!(s.update(1.0).count(), 0);
    }

    #[test]
    fn together_waits_for_the_longest() {
        let mut s = Sequence::new()
            .together([
                Sequence::new().wait(1.0).emit("short"),
                Sequence::new().wait(2.0).emit("long"),
            ])
            .wait(0.5)
            .emit("after");
        assert_eq!(s.update(1.0).collect::<Vec<_>>(), ["short"]);
        assert_eq!(s.update(1.0).collect::<Vec<_>>(), ["long"]);
        assert_eq!(s.update(0.4).count(), 0);
        assert_eq!(s.update(0.1).collect::<Vec<_>>(), ["after"]);
    }

    #[test]
    fn leftover_time_carries_through_together() {
        let mut s = Sequence::new()
            .together([Sequence::new().wait(1.0)])
            .wait(1.0)
            .emit("done");
        assert_eq!(s.update(2.0).collect::<Vec<_>>(), ["done"]);
    }

    #[test]
    fn skip_fires_everything_left() {
        let mut s = Sequence::new().wait(10.0).emit(1).wait(10.0).emit(2);
        assert_eq!(s.skip().collect::<Vec<_>>(), [1, 2]);
        assert!(s.is_finished());
    }

    #[test]
    fn then_appends() {
        let mut s = Sequence::new()
            .emit(1)
            .then(Sequence::new().wait(1.0).emit(2));
        assert_eq!(s.update(1.0).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn sequence_in_f64() {
        let mut s: Sequence<u8, f64> = Sequence::new().wait(0.5).emit(1);
        assert_eq!(s.update(0.25).count(), 0);
        assert_eq!(s.update(0.25).collect::<Vec<_>>(), [1]);
    }
}
