//! Text and saved files.
//!
//! Shows the built-in font at several sizes, wrapping and alignment,
//! rotated and tinted text, a bitmap font, and a counter that is saved to the game's save
//! directory so it survives restarts. Press Space to count and Escape to
//! quit.

use smitten::graphics::{Align, DrawMode, Font};
use smitten::{Color, Config, Filter, Game, Key, event, filesystem, graphics, timer};

const SAVE_FILE: &str = "presses.txt";
const PARAGRAPH: &str = "Smitten wraps long text to the width you give it, \
breaking lines at spaces. Each column below uses a different alignment.";

struct Text {
    title: Font,
    body: Font,
    pixel: Font,
    presses: u32,
}

impl Game for Text {
    fn load() -> Self {
        graphics::set_background_color(Color::rgb(0.11, 0.12, 0.15));
        // A missing or unreadable save file just means starting from zero.
        let presses = filesystem::read_string(SAVE_FILE)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0);
        // A bitmap font: an image with its characters in a row.
        let pixel = graphics::new_image_font(
            "pixel-font.png",
            " ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,!?:-",
        );
        pixel.set_filter(Filter::Nearest);
        Text {
            pixel,
            title: graphics::new_default_font(48.0),
            body: graphics::new_default_font(18.0),
            presses,
        }
    }

    fn draw(&mut self) {
        let width = graphics::width() as f32;

        graphics::set_font(&self.title);
        graphics::set_color(Color::rgb(0.95, 0.35, 0.45));
        graphics::printf("Hello, Smitten", 0.0, 30.0, width, Align::Center);

        graphics::set_font(&self.body);
        let column = (width - 80.0) / 3.0;
        for (index, align) in [Align::Left, Align::Center, Align::Right]
            .into_iter()
            .enumerate()
        {
            let x = 20.0 + index as f32 * (column + 20.0);
            graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.08));
            graphics::rectangle(DrawMode::Fill, x, 120.0, column, 190.0);
            graphics::set_color(Color::WHITE);
            graphics::printf(PARAGRAPH, x, 120.0, column, align);
        }

        graphics::set_color(Color::rgb(0.5, 0.85, 0.6));
        graphics::print(
            &format!("Space pressed {} times (saved between runs)", self.presses),
            20.0,
            340.0,
        );

        graphics::set_font(&self.pixel);
        graphics::set_color(Color::rgb(0.55, 0.8, 1.0));
        graphics::print_ex(
            "A BITMAP FONT, 5 PIXELS TALL: 0123456789!?",
            20.0,
            385.0,
            0.0,
            4.0,
            4.0,
            0.0,
            0.0,
        );
        graphics::set_font(&self.body);

        // Spun around its own center.
        let spinning = "rotated text";
        graphics::set_color(Color::rgb(1.0, 0.8, 0.3));
        graphics::print_ex(
            spinning,
            width / 2.0,
            470.0,
            timer::time() as f32,
            1.5,
            1.5,
            self.body.width(spinning) / 2.0,
            self.body.height() / 2.0,
        );

        graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.6));
        graphics::print(&format!("{} FPS", timer::fps()), 20.0, 570.0);
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        match key {
            Key::Space if !is_repeat => {
                self.presses += 1;
                if let Err(error) = filesystem::write(SAVE_FILE, self.presses.to_string()) {
                    eprintln!("could not save: {error}");
                }
            }
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Text>(Config {
        title: "Text".to_string(),
        identity: Some("smitten-text-example".to_string()),
        ..Config::default()
    });
}
