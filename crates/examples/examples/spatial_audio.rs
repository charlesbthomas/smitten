//! Positional sound, filters and effects.
//!
//! A humming sound is made in code with a `SoundData` and follows the
//! mouse. The listener is the ring in the middle of the window: move the
//! sound left or right to hear it pan, and away to hear it fade.
//!
//! - R turns the reverb on and off.
//! - E turns the echo on and off.
//! - F turns a low-pass filter on and off, as if behind a wall.
//! - Space plays a blip made from a sound file, at the mouse.
//! - Escape quits.
//!
//! Run it with `cargo run --example spatial_audio`. Headphones make the
//! panning easiest to hear.

use std::f32::consts::TAU;

use smitten::audio::{self, Echo, Filter, Reverb, Source, SourceType};
use smitten::graphics::{self, DrawMode};
use smitten::sound::SoundData;
use smitten::{Color, Config, Game, Key, event, mouse};

/// Distances are in pixels. Within this many the sound is at full volume,
/// and each time the distance doubles beyond it the sound halves.
const REFERENCE_DISTANCE: f32 = 80.0;

struct SpatialAudio {
    hum: Source,
    /// The hum's samples, drawn as a waveform.
    hum_data: SoundData,
    blip: Source,
    blips: Vec<Source>,
    reverb: bool,
    echo: bool,
    muffled: bool,
}

/// One second of a soft chord that loops without a click, since every
/// note fits a whole number of cycles in the second.
fn make_hum() -> SoundData {
    let rate = 44_100;
    let mut data = SoundData::new(rate as usize, rate, 1);
    for i in 0..data.sample_count() {
        let t = i as f32 / rate as f32;
        let value = [110.0, 165.0, 220.0, 330.0]
            .iter()
            .enumerate()
            .map(|(n, frequency)| (TAU * frequency * t).sin() / (n + 1) as f32)
            .sum::<f32>()
            * 0.3;
        data.set_sample(i, 0, value);
    }
    data
}

/// Puts a positional source where a point on the screen is, with the
/// listener at the centre of the window.
fn place(source: &Source, x: f32, y: f32) {
    source.set_position(x, y, 0.0);
    source.set_attenuation_distances(REFERENCE_DISTANCE, f32::MAX);
}

impl SpatialAudio {
    /// Sends `source` through whichever effects are switched on.
    fn route(&self, source: &Source) {
        for (name, on) in [("reverb", self.reverb), ("echo", self.echo)] {
            if on {
                source.set_effect(name, None);
            } else {
                source.remove_effect(name);
            }
        }
        source.set_filter(self.muffled.then(|| Filter::low_pass(0.1)));
    }
}

impl Game for SpatialAudio {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.08, 0.09, 0.12));
        audio::set_position(
            graphics::width() as f32 / 2.0,
            graphics::height() as f32 / 2.0,
            0.0,
        );
        audio::set_effect(
            "reverb",
            Reverb {
                decay_time: 2.5,
                ..Reverb::default()
            },
        );
        audio::set_effect(
            "echo",
            Echo {
                delay: 0.2,
                tap_delay: 0.15,
                feedback: 0.4,
                ..Echo::default()
            },
        );

        let hum_data = make_hum();
        let hum = Source::from_sound_data(&hum_data);
        hum.set_looping(true);
        hum.play();

        Ok(SpatialAudio {
            hum,
            hum_data,
            blip: Source::load("blip.wav", SourceType::Static)?,
            blips: Vec::new(),
            reverb: false,
            echo: false,
            muffled: false,
        })
    }

    fn update(&mut self, _dt: f32) {
        let (x, y) = mouse::position();
        place(&self.hum, x, y);
        self.blips.retain(Source::is_playing);
    }

    fn draw(&mut self) {
        let (cx, cy) = (
            graphics::width() as f32 / 2.0,
            graphics::height() as f32 / 2.0,
        );
        // Rings at the distances where the sound is a half and a quarter as
        // loud.
        graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.12));
        for ring in [1.0, 2.0, 4.0] {
            graphics::circle(DrawMode::Line, cx, cy, REFERENCE_DISTANCE * ring);
        }
        graphics::set_color(Color::rgb(0.4, 0.8, 1.0));
        graphics::circle(DrawMode::Line, cx, cy, 12.0);

        let (x, y) = mouse::position();
        graphics::set_color(Color::rgb(0.95, 0.35, 0.45));
        graphics::circle(DrawMode::Fill, x, y, 8.0);

        // The first fiftieth of a second of the hum, read from its samples.
        let samples = self.hum_data.sample_count().min(882);
        let width = 240.0;
        let points: Vec<(f32, f32)> = (0..samples)
            .map(|i| {
                let value = self.hum_data.sample(i, 0);
                (
                    20.0 + i as f32 / samples as f32 * width,
                    graphics::height() as f32 - 50.0 - value * 30.0,
                )
            })
            .collect();
        graphics::set_color(Color::rgba(0.95, 0.35, 0.45, 0.7));
        for pair in points.windows(2) {
            graphics::line(pair[0].0, pair[0].1, pair[1].0, pair[1].1);
        }

        let on = |flag: bool| if flag { "on" } else { "off" };
        let lines = [
            format!("R: reverb {}", on(self.reverb)),
            format!("E: echo {}", on(self.echo)),
            format!("F: low-pass filter {}", on(self.muffled)),
            "Space: blip at the mouse".to_string(),
        ];
        graphics::set_color(Color::WHITE);
        for (row, line) in lines.iter().enumerate() {
            graphics::print(line, 20.0, 20.0 + row as f32 * 24.0);
        }
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        if is_repeat {
            return;
        }
        match key {
            Key::R => self.reverb = !self.reverb,
            Key::E => self.echo = !self.echo,
            Key::F => self.muffled = !self.muffled,
            Key::Space => {
                let blip = self.blip.clone();
                let (x, y) = mouse::position();
                place(&blip, x, y);
                self.route(&blip);
                blip.play();
                self.blips.push(blip);
                return;
            }
            Key::Escape => event::quit(),
            _ => return,
        }
        self.route(&self.hum);
        for blip in &self.blips {
            self.route(blip);
        }
    }
}

fn main() {
    smitten::run::<SpatialAudio>(Config {
        title: "Smitten positional audio".into(),
        width: 800,
        height: 600,
        ..Config::default()
    });
}
