//! Every shape Smitten can draw, filled on the top row and outlined on
//! the bottom. The arcs open and close over time. Press Escape to quit.

use std::f32::consts::{PI, TAU};

use smitten::graphics::{Align, ArcType, DrawMode};
use smitten::{Color, Config, Game, Key, event, graphics, timer};

const COLUMN: f32 = 130.0;
const NAMES: [&str; 6] = [
    "rectangle",
    "rounded",
    "circle",
    "ellipse",
    "arc",
    "polygon",
];

struct Shapes;

fn draw_row(mode: DrawMode, y: f32, sweep: f32) {
    let center = |column: usize| 75.0 + column as f32 * COLUMN;
    graphics::rectangle(mode, (center(0) - 45.0, y - 35.0), (90.0, 70.0));
    graphics::rounded_rectangle(
        mode,
        (center(1) - 45.0, y - 35.0),
        (90.0, 70.0),
        (18.0, 18.0),
    );
    graphics::circle(mode, (center(2), y), 40.0);
    graphics::ellipse(mode, (center(3), y), (50.0, 28.0));
    graphics::arc(mode, ArcType::Pie, (center(4), y), 42.0, 0.0, sweep);
    graphics::polygon(
        mode,
        &[
            (center(5), y - 42.0),
            (center(5) + 44.0, y - 8.0),
            (center(5) + 28.0, y + 38.0),
            (center(5) - 28.0, y + 38.0),
            (center(5) - 44.0, y - 8.0),
        ],
    );
}

impl Game for Shapes {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.11, 0.12, 0.15));
        Ok(Shapes)
    }

    fn draw(&mut self) {
        let time = timer::time() as f32;
        // Between a sliver and nearly a full turn.
        let sweep = PI + (time * 0.8).sin() * (PI - 0.4);

        graphics::set_color(Color::rgb(0.95, 0.35, 0.45));
        draw_row(DrawMode::Fill, 90.0, sweep);

        graphics::set_color(Color::rgb(0.55, 0.8, 1.0));
        graphics::set_line_width(3.0);
        draw_row(DrawMode::Line, 220.0, sweep);

        graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.7));
        for (column, name) in NAMES.iter().enumerate() {
            let x = 10.0 + column as f32 * COLUMN;
            graphics::printf(name, (x, 285.0), COLUMN, Align::Center);
        }

        // The three ways an outlined arc can end.
        graphics::set_color(Color::rgb(0.5, 0.85, 0.6));
        let kinds = [
            (ArcType::Pie, "pie"),
            (ArcType::Open, "open"),
            (ArcType::Closed, "closed"),
        ];
        for (index, (kind, name)) in kinds.into_iter().enumerate() {
            let x = 75.0 + index as f32 * COLUMN;
            graphics::arc(DrawMode::Line, kind, (x, 390.0), 42.0, -0.5, sweep - 0.5);
            graphics::printf(name, (x - 65.0, 445.0), COLUMN, Align::Center);
        }

        // A line through many points, with joined corners.
        graphics::set_color(Color::rgb(1.0, 0.8, 0.3));
        graphics::set_line_width(6.0);
        let wave: Vec<(f32, f32)> = (0..=8)
            .map(|step| {
                let x = 440.0 + step as f32 * 40.0;
                let y = 390.0 + if step % 2 == 0 { -35.0 } else { 35.0 } * (time * 1.5).cos();
                (x, y)
            })
            .collect();
        graphics::polyline(&wave);
        graphics::printf("polyline", (440.0, 445.0), 320.0, Align::Center);

        // Points, here placed around a spiral.
        graphics::set_color(Color::WHITE);
        graphics::set_point_size(4.0);
        let spiral: Vec<(f32, f32)> = (0..90)
            .map(|step| {
                let turn = step as f32 / 90.0;
                let angle = turn * TAU * 3.0 + time;
                let radius = 8.0 + turn * 50.0;
                (400.0 + radius * angle.cos(), 530.0 + radius * angle.sin())
            })
            .collect();
        graphics::points(&spiral);
        graphics::printf("points", (480.0, 522.0), 100.0, Align::Left);
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        if key == Key::Escape {
            event::quit();
        }
    }
}

fn main() {
    smitten::run::<Shapes>(Config {
        title: "Shapes".to_string(),
        ..Config::default()
    });
}
