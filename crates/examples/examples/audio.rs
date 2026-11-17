//! Sound effects and music.
//!
//! - Space plays a blip. Each press overlaps the last, at a random pitch.
//! - M pauses and resumes the music, which is streamed and loops.
//! - Up and Down change the overall volume.
//! - Escape quits.

use smitten::audio::{Source, SourceType};
use smitten::graphics::DrawMode;
use smitten::{Color, Config, Game, Key, audio, event, graphics, keyboard, timer};

struct Audio {
    blip: Source,
    /// Blips that are still sounding. Each is a copy of `blip`, so they
    /// can overlap.
    blips: Vec<Source>,
    music: Source,
}

impl Game for Audio {
    fn load() -> Self {
        graphics::set_background_color(Color::rgb(0.11, 0.12, 0.15));
        // Holding Up or Down keeps changing the volume.
        keyboard::set_key_repeat(true);
        let music = audio::new_source("loop.ogg", SourceType::Stream);
        music.set_looping(true);
        music.set_volume(0.6);
        music.play();
        Audio {
            blip: audio::new_source("blip.wav", SourceType::Static),
            blips: Vec::new(),
            music,
        }
    }

    fn update(&mut self, _dt: f32) {
        self.blips.retain(Source::is_playing);
    }

    fn draw(&mut self) {
        let lines = [
            format!("Space: blip ({} sounding)", self.blips.len()),
            format!(
                "M: music is {}",
                if self.music.is_playing() {
                    "playing"
                } else {
                    "paused"
                }
            ),
            format!("Up / Down: volume {:.0}%", audio::volume() * 100.0),
            format!("Sources playing: {}", audio::active_source_count()),
        ];
        graphics::set_color(Color::WHITE);
        for (row, line) in lines.iter().enumerate() {
            graphics::print(line, 40.0, 40.0 + row as f32 * 28.0);
        }

        // A bar showing how far through the music is.
        let width = graphics::width() as f32 - 80.0;
        let progress = match self.music.duration() {
            Some(duration) if duration > 0.0 => (self.music.tell() / duration) as f32,
            _ => 0.0,
        };
        graphics::set_color(Color::rgb(0.3, 0.32, 0.38));
        graphics::rectangle(DrawMode::Fill, 40.0, 180.0, width, 12.0);
        graphics::set_color(Color::rgb(0.95, 0.35, 0.45));
        graphics::rectangle(DrawMode::Fill, 40.0, 180.0, width * progress, 12.0);
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        match key {
            Key::Space if !is_repeat => {
                // Vary the pitch a little so repeats do not sound robotic.
                let wobble = (timer::time() * 1000.0) as u32 % 5;
                let blip = self.blip.clone();
                blip.set_pitch(0.8 + wobble as f32 * 0.1);
                blip.play();
                self.blips.push(blip);
            }
            Key::M if !is_repeat => {
                if self.music.is_playing() {
                    self.music.pause();
                } else {
                    self.music.play();
                }
            }
            Key::Up => audio::set_volume((audio::volume() + 0.1).min(1.0)),
            Key::Down => audio::set_volume((audio::volume() - 0.1).max(0.0)),
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Audio>(Config {
        title: "Smitten audio".into(),
        width: 640,
        height: 240,
        ..Config::default()
    });
}
