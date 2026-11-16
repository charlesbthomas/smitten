//! Frame timing, the equivalent of `love.timer`.

use crate::context;

#[derive(Default)]
pub(crate) struct TimerState {
    start_ns: u64,
    last_ns: u64,
    delta: f32,
    fps: u32,
    fps_window_start_ns: u64,
    fps_frames: u32,
}

impl TimerState {
    pub fn reset(&mut self, now_ns: u64) {
        *self = Self {
            start_ns: now_ns,
            last_ns: now_ns,
            fps_window_start_ns: now_ns,
            ..Self::default()
        };
    }

    /// Advances to `now_ns` and returns the seconds since the previous step.
    pub fn step(&mut self, now_ns: u64) -> f32 {
        self.delta = now_ns.saturating_sub(self.last_ns) as f32 / 1e9;
        self.last_ns = now_ns;

        self.fps_frames += 1;
        let window_ns = now_ns.saturating_sub(self.fps_window_start_ns);
        if window_ns >= 1_000_000_000 {
            self.fps = (self.fps_frames as f64 * 1e9 / window_ns as f64).round() as u32;
            self.fps_frames = 0;
            self.fps_window_start_ns = now_ns;
        }
        self.delta
    }
}

/// Seconds between the last two frames, the same value passed to `update`.
pub fn delta() -> f32 {
    context::with(|ctx| ctx.timer.delta)
}

/// Frames per second, averaged over the last second.
pub fn fps() -> u32 {
    context::with(|ctx| ctx.timer.fps)
}

/// Seconds since the game finished loading.
pub fn time() -> f64 {
    context::with(|ctx| ctx.backend.ticks_ns().saturating_sub(ctx.timer.start_ns) as f64 / 1e9)
}
