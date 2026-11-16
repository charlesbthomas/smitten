//! A game drawn at a fixed design size and scaled to the window.
//!
//! The game always draws in a space of 320 by 180, whatever the window's
//! size, and the mouse position arrives in the same space. Resize the
//! window to see the picture grow in whole steps with black bars around
//! it. Press F11 for fullscreen and Escape to quit.
//!
//! Run it with `smooth` to scale smoothly at the window's full resolution
//! instead, which suits games with high-resolution art:
//!
//! ```text
//! cargo run --example design
//! cargo run --example design -- smooth
//! ```

use smitten::graphics::{DrawMode, Image};
use smitten::{Color, Config, Filter, Game, Key, Scaling, event, graphics, mouse, timer, window};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;

struct Design {
    heart: Image,
}

impl Game for Design {
    fn load() -> Self {
        graphics::set_background_color(Color::rgb(0.13, 0.15, 0.25));
        graphics::set_default_filter(Filter::Nearest);
        Design {
            heart: graphics::new_image("heart.png"),
        }
    }

    fn update(&mut self, _dt: f32) {
        let (width, height) = graphics::dimensions();
        let (pixel_width, pixel_height) = graphics::pixel_dimensions();
        let (window_width, window_height) = window::dimensions();
        window::set_title(&format!(
            "Design: draws {width} x {height} ({pixel_width} x {pixel_height} pixels) \
             in a {window_width} x {window_height} window"
        ));
    }

    fn draw(&mut self) {
        let (width, height) = (WIDTH as f32, HEIGHT as f32);

        // A checkerboard floor, one square every 20 units.
        for row in 0..3 {
            for column in 0..16 {
                let shade = if (row + column) % 2 == 0 { 0.22 } else { 0.28 };
                graphics::set_color(Color::rgb(shade, shade + 0.05, shade + 0.15));
                graphics::rectangle(
                    DrawMode::Fill,
                    column as f32 * 20.0,
                    height - 60.0 + row as f32 * 20.0,
                    20.0,
                    20.0,
                );
            }
        }

        // The frame marks the edge of the design area.
        graphics::set_color(Color::rgb(1.0, 0.8, 0.4));
        graphics::rectangle(DrawMode::Line, 0.5, 0.5, width - 1.0, height - 1.0);
        graphics::circle(DrawMode::Fill, width - 40.0, 36.0, 16.0);

        graphics::set_color(Color::WHITE);
        let bob = (timer::time() as f32 * 3.0).sin() * 4.0;
        for index in 0..5 {
            graphics::draw(&self.heart, 40.0 + index as f32 * 24.0, 60.0 + bob);
        }

        // The mouse is reported in design units, so this heart follows it
        // exactly at any window size.
        let (x, y) = mouse::position();
        graphics::draw_ex(&self.heart, x.round(), y.round(), 0.0, 1.0, 1.0, 8.0, 8.0);
        graphics::print(&format!("mouse {:.0}, {:.0}", x, y), 8.0, 8.0);
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        match key {
            Key::F11 => window::set_fullscreen(!window::fullscreen()),
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    let smooth = std::env::args().any(|argument| argument == "smooth");
    smitten::run::<Design>(Config {
        title: "Design".to_string(),
        width: 960,
        height: 600,
        resizable: true,
        design_size: Some((WIDTH, HEIGHT)),
        design_scaling: if smooth {
            Scaling::Fit
        } else {
            Scaling::PixelPerfect
        },
        ..Config::default()
    });
}
