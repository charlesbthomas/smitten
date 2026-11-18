//! How many sprites can be drawn at 60 frames a second?
//!
//! Hearts bounce around, each one moved, turned and tinted on its own.
//! The title bar shows how many there are and how long a frame takes.
//!
//! Up adds 1000 hearts and Down removes 1000. Tab switches between drawing
//! each heart with `graphics::draw_ex` and putting them all in a sprite
//! batch that is rebuilt every frame. Escape quits.
//!
//! Run it in release mode, since debug builds are several times slower:
//!
//! ```text
//! cargo run --release --example benchmark
//! cargo run --release --example benchmark -- 20000
//! ```
//!
//! The number is how many hearts to start with. Add `--report` to measure
//! for five seconds, print the average frame time, and quit.

use std::time::Instant;

use smitten::graphics::{Image, SpriteBatch};
use smitten::{Color, Config, Filter, Game, Key, event, graphics, timer, window};

const STEP: usize = 1000;
const REPORT_AFTER: f64 = 5.0;
/// Frames at the start are skipped while everything warms up.
const WARM_UP: f64 = 1.0;

struct Heart {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    angle: f32,
    spin: f32,
    tint: Color,
}

struct Benchmark {
    image: Image,
    batch: SpriteBatch,
    use_batch: bool,
    hearts: Vec<Heart>,
    seed: u32,
    /// With `--report`: when measuring started and the frames since.
    report: bool,
    measured: Option<(Instant, u32)>,
}

impl Benchmark {
    fn random(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    fn add_hearts(&mut self, count: usize) {
        let (width, height) = graphics::dimensions();
        for _ in 0..count {
            let heart = Heart {
                x: self.random() * width as f32,
                y: self.random() * height as f32,
                vx: (self.random() - 0.5) * 300.0,
                vy: (self.random() - 0.5) * 300.0,
                angle: 0.0,
                spin: (self.random() - 0.5) * 4.0,
                tint: Color::rgb(1.0, 0.6 + 0.4 * self.random(), 0.6 + 0.4 * self.random()),
            };
            self.hearts.push(heart);
        }
    }

    fn mode(&self) -> &'static str {
        if self.use_batch {
            "sprite batch"
        } else {
            "one draw each"
        }
    }
}

impl Game for Benchmark {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.12, 0.1, 0.18));
        graphics::set_default_filter(Filter::Nearest);
        let image = Image::load("heart.png")?;

        let mut count = 5000;
        let mut report = false;
        let mut use_batch = false;
        for argument in std::env::args().skip(1) {
            match argument.as_str() {
                "--report" => report = true,
                "--batch" => use_batch = true,
                other => count = other.parse().unwrap_or(count),
            }
        }

        let mut game = Benchmark {
            batch: SpriteBatch::new(&image, count),
            image,
            use_batch,
            hearts: Vec::new(),
            seed: 7,
            report,
            measured: None,
        };
        game.add_hearts(count);
        Ok(game)
    }

    fn update(&mut self, dt: f32) {
        let (width, height) = graphics::dimensions();
        let (width, height) = (width as f32, height as f32);
        for heart in &mut self.hearts {
            heart.x += heart.vx * dt;
            heart.y += heart.vy * dt;
            heart.angle += heart.spin * dt;
            if (heart.x < 0.0 && heart.vx < 0.0) || (heart.x > width && heart.vx > 0.0) {
                heart.vx = -heart.vx;
            }
            if (heart.y < 0.0 && heart.vy < 0.0) || (heart.y > height && heart.vy > 0.0) {
                heart.vy = -heart.vy;
            }
        }

        let fps = timer::fps();
        window::set_title(&format!(
            "Benchmark: {} hearts, {}, {} FPS, {:.2} ms a frame",
            self.hearts.len(),
            self.mode(),
            fps,
            1000.0 / fps.max(1) as f32,
        ));

        if self.report {
            let time = timer::time();
            match &mut self.measured {
                None if time >= WARM_UP => self.measured = Some((Instant::now(), 0)),
                Some((_, frames)) => *frames += 1,
                None => {}
            }
            if time >= REPORT_AFTER
                && let Some((start, frames)) = self.measured
            {
                let seconds = start.elapsed().as_secs_f64();
                println!(
                    "{} hearts, {}: {:.2} ms a frame, {:.0} FPS",
                    self.hearts.len(),
                    self.mode(),
                    1000.0 * seconds / frames as f64,
                    frames as f64 / seconds,
                );
                event::quit();
            }
        }
    }

    fn draw(&mut self) {
        let (center_x, center_y) = (
            self.image.width() as f32 / 2.0,
            self.image.height() as f32 / 2.0,
        );
        if self.use_batch {
            self.batch.clear();
            for heart in &self.hearts {
                self.batch.set_color(heart.tint);
                self.batch
                    .add_ex(heart.x, heart.y, heart.angle, 1.0, 1.0, center_x, center_y);
            }
            graphics::draw_sprite_batch(&self.batch, (0.0, 0.0));
        } else {
            for heart in &self.hearts {
                graphics::set_color(heart.tint);
                graphics::draw_ex(
                    &self.image,
                    heart.x,
                    heart.y,
                    heart.angle,
                    1.0,
                    1.0,
                    center_x,
                    center_y,
                );
            }
            graphics::set_color(Color::WHITE);
        }
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        match key {
            Key::Up => self.add_hearts(STEP),
            Key::Down => {
                let keep = self.hearts.len().saturating_sub(STEP);
                self.hearts.truncate(keep);
            }
            Key::Tab => self.use_batch = !self.use_batch,
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Benchmark>(Config {
        title: "Benchmark".to_string(),
        resizable: true,
        // Without this the display's refresh rate would cap the result.
        vsync: false,
        ..Config::default()
    });
}
