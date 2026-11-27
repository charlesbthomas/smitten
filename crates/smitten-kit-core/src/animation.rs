//! Frame animation without a window: [`Animation`] clips (which frames,
//! for how long, looping or not) and an [`Animator`] that plays one clip
//! per state of a character.
//!
//! Frames are numbers: indices into a sprite sheet. `smitten-kit`'s
//! `SpriteSheet` draws them; tests and rules code only need the numbers.
//!
//! ```
//! use smitten_kit_core::animation::{Animation, Animator};
//!
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
//! enum Pose {
//!     Idle,
//!     Run,
//!     Land,
//! }
//!
//! let mut hero = Animator::new(Pose::Idle, Animation::new([0, 1], 2.0));
//! hero.add(Pose::Run, Animation::range(2..=7, 12.0));
//! hero.add(Pose::Land, Animation::new([8, 9], 20.0).once());
//!
//! hero.set(Pose::Run);
//! hero.update(0.25); // three frames in at 12 fps
//! assert_eq!(hero.frame(), 5);
//! hero.set(Pose::Run); // already running: carries on, no restart
//! assert_eq!(hero.frame(), 5);
//!
//! hero.set(Pose::Land);
//! assert!(!hero.update(0.05));
//! assert!(hero.update(0.05)); // true on the frame a one-shot ends
//! assert_eq!(hero.frame(), 9); // and it holds its last frame
//! ```

use std::collections::HashMap;
use std::hash::Hash;

/// What a clip does at its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Playback {
    /// Starts over.
    #[default]
    Loop,
    /// Stops on the last frame.
    Once,
    /// Plays backward to the start, then forward again.
    PingPong,
}

/// A run of frames with how long each shows. Build one with
/// [`new`](Animation::new) or [`range`](Animation::range).
#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    frames: Vec<usize>,
    /// When each frame ends, from the clip's start, in seconds.
    ends: Vec<f32>,
    playback: Playback,
}

impl Animation {
    /// `frames` shown in order at `fps` frames a second, looping.
    ///
    /// # Panics
    ///
    /// With no frames.
    pub fn new(frames: impl IntoIterator<Item = usize>, fps: f32) -> Animation {
        let frames: Vec<usize> = frames.into_iter().collect();
        let seconds = if fps > 0.0 { 1.0 / fps } else { f32::INFINITY };
        let durations = vec![seconds; frames.len()];
        Animation::with_durations(frames.into_iter().zip(durations))
    }

    /// Frames `first..=last` of a sheet (or any range) at `fps`, looping.
    pub fn range(frames: impl IntoIterator<Item = usize>, fps: f32) -> Animation {
        Animation::new(frames, fps)
    }

    /// Frames that each show for their own time in seconds, as an
    /// Aseprite export lists them.
    ///
    /// # Panics
    ///
    /// With no frames.
    pub fn with_durations(frames: impl IntoIterator<Item = (usize, f32)>) -> Animation {
        let mut clip = Animation {
            frames: Vec::new(),
            ends: Vec::new(),
            playback: Playback::Loop,
        };
        let mut end = 0.0;
        for (frame, seconds) in frames {
            end += seconds.max(0.0);
            clip.frames.push(frame);
            clip.ends.push(end);
        }
        assert!(!clip.frames.is_empty(), "an animation needs a frame");
        clip
    }

    /// The same clip, stopping on its last frame.
    #[must_use]
    pub fn once(mut self) -> Animation {
        self.playback = Playback::Once;
        self
    }

    /// The same clip, playing back and forth.
    #[must_use]
    pub fn ping_pong(mut self) -> Animation {
        self.playback = Playback::PingPong;
        self
    }

    /// What it does at its end.
    pub fn playback(&self) -> Playback {
        self.playback
    }

    /// The frames, in order.
    pub fn frames(&self) -> &[usize] {
        &self.frames
    }

    /// Seconds for one pass through the frames.
    pub fn duration(&self) -> f32 {
        *self.ends.last().expect("an animation has a frame")
    }

    fn position(&self, time: f32) -> usize {
        self.ends
            .partition_point(|&end| end <= time)
            .min(self.frames.len() - 1)
    }

    /// The frame showing `time` seconds after the clip started.
    pub fn frame_at(&self, time: f32) -> usize {
        let duration = self.duration();
        if !(duration > 0.0 && duration.is_finite()) {
            return self.frames[self.position(time.max(0.0))];
        }
        let time = time.max(0.0);
        let index = match self.playback {
            Playback::Once => self.position(time),
            Playback::Loop => self.position(time % duration),
            Playback::PingPong => {
                let count = self.frames.len();
                if count == 1 {
                    0
                } else {
                    // Forward through every frame, then back through the
                    // middle ones, so the end frames do not show twice.
                    let back = duration - self.ends[0] - (duration - self.ends[count - 2]);
                    let cycle = duration + back;
                    let t = time % cycle;
                    if t < duration {
                        self.position(t)
                    } else {
                        let into_back = t - duration;
                        // Mirror into the frames 1..count-1, played backward.
                        let mirrored = self.ends[count - 2] - into_back;
                        self.position(mirrored.max(self.ends[0]))
                    }
                }
            }
        };
        self.frames[index]
    }

    /// Whether a clip that plays once has ended `time` seconds in.
    pub fn is_finished_at(&self, time: f32) -> bool {
        self.playback == Playback::Once && time >= self.duration()
    }
}

/// Plays one [`Animation`] per state, switching on
/// [`set`](Animator::set). See the [module docs](self).
#[derive(Clone, Debug)]
pub struct Animator<S> {
    clips: HashMap<S, Animation>,
    state: S,
    time: f32,
    speed: f32,
}

impl<S: Copy + Eq + Hash> Animator<S> {
    /// An animator starting in `state`, playing `clip`.
    pub fn new(state: S, clip: Animation) -> Self {
        let mut clips = HashMap::new();
        clips.insert(state, clip);
        Animator {
            clips,
            state,
            time: 0.0,
            speed: 1.0,
        }
    }

    /// Adds (or replaces) the clip for `state`.
    pub fn add(&mut self, state: S, clip: Animation) -> &mut Self {
        self.clips.insert(state, clip);
        self
    }

    /// Switches to `state`'s clip from its start. Setting the state it is
    /// already in does nothing, so this can be called every frame.
    /// A state with no clip keeps the current one playing.
    pub fn set(&mut self, state: S) {
        if state != self.state && self.clips.contains_key(&state) {
            self.state = state;
            self.time = 0.0;
        }
    }

    /// Plays the current clip again from its start.
    pub fn restart(&mut self) {
        self.time = 0.0;
    }

    /// The state playing.
    pub fn state(&self) -> S {
        self.state
    }

    /// The clip playing.
    pub fn clip(&self) -> &Animation {
        &self.clips[&self.state]
    }

    /// Plays faster or slower: 2.0 runs at double speed.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.0);
    }

    /// Moves on by `dt` seconds. Returns true on the frame a clip that
    /// plays once reaches its end, to move on to the next state.
    pub fn update(&mut self, dt: f32) -> bool {
        let clip = &self.clips[&self.state];
        let was_finished = clip.is_finished_at(self.time);
        self.time += dt.max(0.0) * self.speed;
        !was_finished && clip.is_finished_at(self.time)
    }

    /// Whether a clip that plays once has ended.
    pub fn is_finished(&self) -> bool {
        self.clip().is_finished_at(self.time)
    }

    /// Seconds since the current clip started.
    pub fn time(&self) -> f32 {
        self.time
    }

    /// The sheet frame to draw now.
    pub fn frame(&self) -> usize {
        self.clip().frame_at(self.time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loops_and_holds() {
        let walk = Animation::new([4, 5, 6], 10.0);
        assert_eq!(walk.frame_at(0.0), 4);
        assert_eq!(walk.frame_at(0.15), 5);
        assert_eq!(walk.frame_at(0.35), 4);
        let once = walk.clone().once();
        assert_eq!(once.frame_at(10.0), 6);
        assert!(once.is_finished_at(0.3));
        assert!(!walk.is_finished_at(10.0));
    }

    #[test]
    fn ping_pong_does_not_repeat_the_ends() {
        let bob = Animation::new([0, 1, 2, 3], 1.0).ping_pong();
        let frames: Vec<usize> = (0..10).map(|s| bob.frame_at(s as f32 + 0.5)).collect();
        assert_eq!(frames, [0, 1, 2, 3, 2, 1, 0, 1, 2, 3]);
        let two = Animation::new([7, 8], 1.0).ping_pong();
        let frames: Vec<usize> = (0..4).map(|s| two.frame_at(s as f32 + 0.5)).collect();
        assert_eq!(frames, [7, 8, 7, 8]);
    }

    #[test]
    fn durations_per_frame() {
        let blink = Animation::with_durations([(0, 2.0), (1, 0.1)]);
        assert_eq!(blink.frame_at(1.9), 0);
        assert_eq!(blink.frame_at(2.05), 1);
        assert_eq!(blink.frame_at(2.15), 0);
    }

    #[test]
    fn unknown_state_keeps_playing() {
        let mut a = Animator::new('i', Animation::new([0, 1], 1.0));
        a.update(1.5);
        a.set('x');
        assert_eq!(a.state(), 'i');
        assert_eq!(a.frame(), 1);
        a.set_speed(2.0);
        a.update(0.25);
        assert_eq!(a.frame(), 0);
    }
}
