//! A debug overlay: frame time with a graph, draw statistics, and values
//! the game asks to watch, toggled with a key.
//!
//! ```no_run
//! use smitten_kit::debug::DebugOverlay;
//! # let (dt, player_speed, enemies) = (0.016, 3.5_f32, 12);
//!
//! let mut overlay = DebugOverlay::new(); // F3 shows and hides it
//!
//! // each frame:
//! overlay.update(dt);
//! overlay.watch("speed", format_args!("{player_speed:.1}"));
//! overlay.watch("enemies", enemies);
//! // ... draw the game ...
//! overlay.draw();
//! ```

use std::collections::VecDeque;
use std::fmt::{Display, Write};

use smitten::event::Event;
use smitten::graphics::{self, DrawMode};
use smitten::{Color, Key};

/// How many frames the graph and the numbers cover.
pub const HISTORY: usize = 120;

/// Frame time numbers over the last [`HISTORY`] frames.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct FrameStats {
    /// Frames a second, from the average frame time.
    pub fps: f32,
    /// The average frame time in milliseconds.
    pub average_ms: f32,
    /// The longest frame in milliseconds: the hitch to look for.
    pub worst_ms: f32,
}

/// The overlay. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct DebugOverlay {
    visible: bool,
    key: Option<Key>,
    times: VecDeque<f32>,
    watches: Vec<(String, String)>,
}

impl Default for DebugOverlay {
    fn default() -> Self {
        DebugOverlay {
            visible: false,
            key: Some(Key::F3),
            times: VecDeque::with_capacity(HISTORY),
            watches: Vec::new(),
        }
    }
}

impl DebugOverlay {
    /// A hidden overlay that F3 toggles.
    pub fn new() -> DebugOverlay {
        DebugOverlay::default()
    }

    /// Toggles with `key` instead, or with no key for `None`.
    #[must_use]
    pub fn with_key(mut self, key: Option<Key>) -> DebugOverlay {
        self.key = key;
        self
    }

    /// Whether it is showing.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Shows or hides it.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Shows it if hidden, hides it if shown.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Toggles on the overlay's key. Returns true if the event was that.
    pub fn handle(&mut self, event: &Event) -> bool {
        if let Event::KeyPressed {
            key,
            is_repeat: false,
        } = event
            && Some(*key) == self.key
        {
            self.toggle();
            return true;
        }
        false
    }

    /// Records a frame of `dt` seconds and forgets last frame's watches.
    /// Call it at the start of each frame.
    pub fn update(&mut self, dt: f32) {
        if self.times.len() == HISTORY {
            self.times.pop_front();
        }
        self.times.push_back(dt.max(0.0));
        self.watches.clear();
    }

    /// Shows `value` beside `name` this frame. Watching the same name
    /// again replaces it.
    pub fn watch(&mut self, name: &str, value: impl Display) {
        let mut text = String::new();
        let _ = write!(text, "{value}");
        match self.watches.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = text,
            None => self.watches.push((name.to_owned(), text)),
        }
    }

    /// This frame's watches, in the order first watched.
    pub fn watches(&self) -> impl Iterator<Item = (&str, &str)> {
        self.watches.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }

    /// Frame time numbers over the recorded frames.
    pub fn stats(&self) -> FrameStats {
        if self.times.is_empty() {
            return FrameStats::default();
        }
        let total: f32 = self.times.iter().sum();
        let average = total / self.times.len() as f32;
        let worst = self.times.iter().copied().fold(0.0, f32::max);
        FrameStats {
            fps: if average > 0.0 { 1.0 / average } else { 0.0 },
            average_ms: average * 1000.0,
            worst_ms: worst * 1000.0,
        }
    }

    /// The lines of text the overlay shows, without the graph.
    pub fn lines(&self) -> Vec<String> {
        let frame = self.stats();
        let mut lines = vec![format!(
            "{:.0} fps  {:.1} ms  worst {:.1} ms",
            frame.fps, frame.average_ms, frame.worst_ms
        )];
        for (name, value) in self.watches() {
            lines.push(format!("{name}: {value}"));
        }
        lines
    }

    /// Draws it in the top-left corner of the screen, if showing.
    pub fn draw(&self) {
        if !self.visible {
            return;
        }
        let draws = graphics::stats();
        let mut lines = self.lines();
        lines.insert(
            1,
            format!(
                "{} draws ({} batched)  {} images  {:.1} MB textures",
                draws.draw_calls,
                draws.draw_calls_batched,
                draws.images,
                draws.texture_memory as f64 / 1_048_576.0
            ),
        );
        let line_height = graphics::font().height() + 2.0;
        let graph_height = 40.0;
        let width = 340.0;
        let height = 8.0 + line_height * lines.len() as f32 + graph_height + 8.0;
        let before = graphics::color();
        graphics::push_all();
        graphics::origin();
        graphics::set_color(Color::rgba(0.0, 0.0, 0.0, 0.7));
        graphics::rectangle(DrawMode::Fill, (4.0, 4.0), (width, height));
        graphics::set_color(Color::WHITE);
        for (i, line) in lines.iter().enumerate() {
            graphics::print(line.as_str(), (12.0, 8.0 + line_height * i as f32));
        }
        // One bar per frame, scaled so 1/30 s fills the graph; a line marks
        // 1/60 s.
        let top = 8.0 + line_height * lines.len() as f32;
        let bottom = top + graph_height;
        let bar = (width - 16.0) / HISTORY as f32;
        for (i, &dt) in self.times.iter().enumerate() {
            let h = (dt * 30.0).min(1.0) * graph_height;
            graphics::set_color(if dt > 1.0 / 50.0 {
                Color::rgb(1.0, 0.4, 0.3)
            } else {
                Color::rgb(0.4, 0.9, 0.5)
            });
            graphics::rectangle(
                DrawMode::Fill,
                (12.0 + bar * i as f32, bottom - h),
                (bar.max(1.0), h),
            );
        }
        graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.4));
        let target = bottom - graph_height / 2.0;
        graphics::line((12.0, target), (width - 4.0, target));
        graphics::pop();
        graphics::set_color(before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_over_the_history() {
        let mut overlay = DebugOverlay::new();
        for _ in 0..HISTORY {
            overlay.update(0.02);
        }
        overlay.update(0.1); // pushes out one 0.02
        let stats = overlay.stats();
        assert!((stats.worst_ms - 100.0).abs() < 1e-3);
        let expected = (0.02 * (HISTORY - 1) as f32 + 0.1) / HISTORY as f32;
        assert!((stats.average_ms - expected * 1000.0).abs() < 1e-2);
    }

    #[test]
    fn watches_last_one_frame() {
        let mut overlay = DebugOverlay::new();
        overlay.update(0.016);
        overlay.watch("hp", 3);
        overlay.watch("hp", 2);
        overlay.watch("pos", format_args!("{:.1}", 1.24));
        assert_eq!(overlay.lines()[1..], ["hp: 2", "pos: 1.2"]);
        overlay.update(0.016);
        assert_eq!(overlay.lines().len(), 1);
    }

    #[test]
    fn its_key_toggles_it() {
        let mut overlay = DebugOverlay::new();
        let f3 = Event::KeyPressed {
            key: Key::F3,
            is_repeat: false,
        };
        assert!(overlay.handle(&f3));
        assert!(overlay.is_visible());
        assert!(!overlay.handle(&Event::KeyPressed {
            key: Key::A,
            is_repeat: false
        }));
        let mut silent = DebugOverlay::new().with_key(None);
        assert!(!silent.handle(&f3));
    }
}
