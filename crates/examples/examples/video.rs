//! A video with its sound track, and a tone made as the game runs.
//!
//! - Space pauses and resumes the video. R rewinds it.
//! - Left and Right skip back and forward a second.
//! - T plays a tone through a queueable source, made a buffer at a time.
//! - Escape quits.
//!
//! The video is an Ogg Theora file made with FFmpeg's `mandelbrot` and
//! `sine` sources.

use smitten::audio::Source;
use smitten::graphics::DrawParams;
use smitten::graphics::{DrawMode, Video};
use smitten::sound::SoundData;
use smitten::{Color, Config, Game, Key, event, graphics};
use std::time::Duration;

const TONE_RATE: u32 = 44_100;
const TONE_BUFFER: usize = 1_024;

struct Player {
    video: Video,
    tone: Source,
    tone_on: bool,
    /// Where the tone's sine wave has got to, so each buffer carries on
    /// from the last without a click.
    phase: f32,
}

impl Player {
    /// Keeps the tone's queue topped up while it is on.
    fn feed_tone(&mut self) {
        while self.tone_on && self.tone.free_buffer_count() > 0 {
            let mut buffer = SoundData::new(TONE_BUFFER, TONE_RATE, 1);
            for sample in buffer.samples_mut() {
                *sample = self.phase.sin() * 0.15;
                self.phase += std::f32::consts::TAU * 330.0 / TONE_RATE as f32;
            }
            self.phase %= std::f32::consts::TAU;
            self.tone.queue(&buffer);
        }
        if self.tone_on && !self.tone.is_playing() {
            self.tone.play();
        }
    }
}

impl Game for Player {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.08, 0.08, 0.1));
        let video = Video::load("mandelbrot.ogv")?;
        video.play();
        Ok(Player {
            video,
            tone: Source::queueable(TONE_RATE, 1, 4),
            tone_on: false,
            phase: 0.0,
        })
    }

    fn update(&mut self, _dt: f32) {
        self.feed_tone();
    }

    fn draw(&mut self) {
        let (screen_w, screen_h) = (graphics::width() as f32, graphics::height() as f32);
        let (video_w, video_h) = self.video.dimensions();
        let scale = ((screen_w - 80.0) / video_w as f32).min((screen_h - 160.0) / video_h as f32);
        let x = (screen_w - video_w as f32 * scale) / 2.0;
        graphics::set_color(Color::WHITE);
        graphics::draw(&self.video, DrawParams::at((x, 40.0)).scale(scale));

        // How far through the video is.
        let bar_y = 40.0 + video_h as f32 * scale + 20.0;
        let bar_w = video_w as f32 * scale;
        let through = self.video.tell().div_duration_f32(self.video.duration());
        graphics::set_color(Color::rgb(0.25, 0.25, 0.3));
        graphics::rectangle(DrawMode::Fill, (x, bar_y), (bar_w, 8.0));
        graphics::set_color(Color::rgb(0.95, 0.6, 0.3));
        graphics::rectangle(DrawMode::Fill, (x, bar_y), (bar_w * through, 8.0));

        graphics::set_color(Color::WHITE);
        let state = if self.video.is_playing() {
            "playing"
        } else {
            "paused"
        };
        graphics::print(
            &format!(
                "{state} {:.1} / {:.1} s    Space: pause    R: rewind    Left / Right: skip    T: tone {}",
                self.video.tell().as_secs_f32(),
                self.video.duration().as_secs_f32(),
                if self.tone_on { "on" } else { "off" },
            ),
            (x, bar_y + 24.0),
        );
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        match key {
            Key::Space if self.video.is_playing() => self.video.pause(),
            Key::Space => self.video.play(),
            Key::R => {
                self.video.rewind();
                self.video.play();
            }
            Key::Left => self
                .video
                .seek(self.video.tell().saturating_sub(Duration::from_secs(1))),
            Key::Right => self.video.seek(self.video.tell() + Duration::from_secs(1)),
            Key::T => {
                self.tone_on = !self.tone_on;
                if !self.tone_on {
                    // Stopping empties the queue.
                    self.tone.stop();
                }
            }
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Player>(Config {
        title: "Video".into(),
        ..Config::default()
    });
}
