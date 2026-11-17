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
    /// Seconds of the frames counted since `fps_window_start_ns`.
    fps_seconds: f64,
    average_delta: f32,
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
        self.fps_seconds += f64::from(self.delta);
        let window_ns = now_ns.saturating_sub(self.fps_window_start_ns);
        if window_ns >= 1_000_000_000 {
            self.fps = (self.fps_frames as f64 * 1e9 / window_ns as f64).round() as u32;
            self.average_delta = (self.fps_seconds / f64::from(self.fps_frames)) as f32;
            self.fps_frames = 0;
            self.fps_seconds = 0.0;
            self.fps_window_start_ns = now_ns;
        }
        self.delta
    }
}

/// Seconds between the last two frames, the same value passed to `update`.
pub fn delta() -> f32 {
    context::with(|ctx| ctx.timer.delta)
}

/// The average of the seconds between frames over the last second, the
/// equivalent of `love.timer.getAverageDelta`. Steadier than [`delta`]
/// for showing frame times. It is 0 for the first second.
pub fn average_delta() -> f32 {
    context::with(|ctx| ctx.timer.average_delta)
}

/// Frames per second, averaged over the last second.
pub fn fps() -> u32 {
    context::with(|ctx| ctx.timer.fps)
}

/// Seconds since the game finished loading.
pub fn time() -> f64 {
    context::with(|ctx| ctx.backend.ticks_ns().saturating_sub(ctx.timer.start_ns) as f64 / 1e9)
}

/// Pauses the game for `seconds`, the equivalent of `love.timer.sleep`.
/// Nothing is drawn and no events are handled meanwhile, so keep it
/// short. Negative or non-finite values do nothing.
pub fn sleep(seconds: f64) {
    if seconds.is_finite() && seconds > 0.0 {
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    #[test]
    fn the_average_delta_covers_the_last_second() {
        let mut timer = TimerState::default();
        timer.reset(0);
        // Nineteen frames of 50 ms and one of 100 ms make 1.05 seconds;
        // the average only updates once a second has passed.
        let mut now = 0;
        for _ in 0..19 {
            now += 50 * MS;
            timer.step(now);
        }
        assert_eq!(timer.average_delta, 0.0);
        now += 100 * MS;
        timer.step(now);
        assert_eq!(timer.fps, 19);
        assert!((timer.average_delta - 0.0525).abs() < 1e-6);
        assert!((timer.delta - 0.1).abs() < 1e-6);
    }

    #[test]
    fn sleeping_waits_and_ignores_nonsense() {
        let start = std::time::Instant::now();
        sleep(0.02);
        assert!(start.elapsed() >= std::time::Duration::from_millis(20));
        let start = std::time::Instant::now();
        sleep(-1.0);
        sleep(f64::NAN);
        sleep(f64::INFINITY);
        assert!(start.elapsed() < std::time::Duration::from_millis(20));
    }
}
