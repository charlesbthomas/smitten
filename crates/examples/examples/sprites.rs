//! Bouncing hearts over a few shapes.
//!
//! Shows image loading, tinting, rotation around a sprite's center, shapes,
//! and the transform stack. Press Space to add hearts and Escape to quit.

use smitten::graphics::{DrawMode, Image};
use smitten::{Color, Config, Filter, Game, Key, event, graphics, timer, window};

const HEART_SCALE: f32 = 4.0;

struct Heart {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    angle: f32,
    spin: f32,
    tint: Color,
}

struct Sprites {
    image: Image,
    hearts: Vec<Heart>,
    seed: u32,
}

impl Sprites {
    /// A small deterministic random number from 0.0 to 1.0, enough for a
    /// demo until `smitten::math` exists.
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
}

impl Game for Sprites {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.12, 0.1, 0.18));
        // Keep the pixel art crisp when it is scaled up.
        graphics::set_default_filter(Filter::Nearest);

        let image = Image::load("heart.png")?;
        let mut game = Sprites {
            image,
            hearts: Vec::new(),
            seed: 7,
        };
        game.add_hearts(50);
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
        window::set_title(&format!(
            "Sprites: {} hearts, {} FPS",
            self.hearts.len(),
            timer::fps()
        ));
    }

    fn draw(&mut self) {
        let (width, height) = graphics::dimensions();
        let (width, height) = (width as f32, height as f32);

        // A floor, a sun and a few rays, drawn with shapes.
        graphics::set_color(Color::rgb(0.2, 0.18, 0.3));
        graphics::rectangle(DrawMode::Fill, (0.0, height - 80.0), (width, 80.0));

        graphics::push();
        graphics::translate((width - 120.0, 120.0));
        graphics::rotate(timer::time() as f32 * 0.3);
        graphics::set_color(Color::rgb(1.0, 0.8, 0.4));
        graphics::circle(DrawMode::Fill, (0.0, 0.0), 40.0);
        graphics::set_line_width(4.0);
        graphics::circle(DrawMode::Line, (0.0, 0.0), 56.0);
        for ray in 0..8 {
            graphics::push();
            graphics::rotate(ray as f32 * std::f32::consts::TAU / 8.0);
            graphics::line((70.0, 0.0), (95.0, 0.0));
            graphics::pop();
        }
        graphics::pop();

        graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.4));
        graphics::set_line_width(2.0);
        graphics::rectangle(DrawMode::Line, (20.0, 20.0), (160.0, 90.0));

        // The hearts, each tinted and spun around its center.
        let (center_x, center_y) = (
            self.image.width() as f32 / 2.0,
            self.image.height() as f32 / 2.0,
        );
        for heart in &self.hearts {
            graphics::set_color(heart.tint);
            graphics::draw_ex(
                &self.image,
                heart.x,
                heart.y,
                heart.angle,
                HEART_SCALE,
                HEART_SCALE,
                center_x,
                center_y,
            );
        }
        graphics::set_color(Color::WHITE);
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        match key {
            Key::Space => self.add_hearts(500),
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Sprites>(Config {
        title: "Sprites".to_string(),
        resizable: true,
        ..Config::default()
    });
}
