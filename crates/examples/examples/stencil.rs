//! The stencil.
//!
//! A stencil marks some pixels so that later drawing can be limited to
//! them or kept away from them.
//!
//! - The colorful pattern is only drawn inside the circle that follows
//!   the mouse and the fixed star, like looking through holes.
//! - The dark grid is drawn everywhere except inside them.
//! - Escape quits.

use std::f32::consts::TAU;

use smitten::graphics::DrawMode;
use smitten::{Color, CompareMode, Config, Game, Key, event, graphics, mouse, timer};

const WIDTH: u32 = 720;
const HEIGHT: u32 = 480;

struct Stencil;

/// Bright diagonal bands that slide over time.
fn draw_pattern() {
    let time = timer::time() as f32;
    for band in -8..16 {
        let x = band as f32 * 60.0 + (time * 40.0) % 60.0;
        let hue = band as f32 * 0.7;
        graphics::set_color(Color::rgb(
            0.6 + 0.4 * hue.sin(),
            0.6 + 0.4 * (hue + 2.0).sin(),
            0.6 + 0.4 * (hue + 4.0).sin(),
        ));
        graphics::polygon(
            DrawMode::Fill,
            &[
                (x, 0.0),
                (x + 60.0, 0.0),
                (x + 60.0 - 240.0, HEIGHT as f32),
                (x - 240.0, HEIGHT as f32),
            ],
        );
    }
}

fn draw_grid() {
    graphics::set_color(Color::rgb(0.2, 0.22, 0.28));
    for column in 0..=WIDTH / 40 {
        let x = column as f32 * 40.0;
        graphics::line(x, 0.0, x, HEIGHT as f32);
    }
    for row in 0..=HEIGHT / 40 {
        let y = row as f32 * 40.0;
        graphics::line(0.0, y, WIDTH as f32, y);
    }
}

/// A five-pointed star as triangles from its center, since `polygon` can
/// only fill convex shapes.
fn draw_star(x: f32, y: f32, radius: f32, turn: f32) {
    let point = |index: usize| {
        let angle = turn + index as f32 / 10.0 * TAU;
        let reach = if index.is_multiple_of(2) {
            radius
        } else {
            radius * 0.45
        };
        (x + angle.cos() * reach, y + angle.sin() * reach)
    };
    for index in 0..10 {
        graphics::polygon(DrawMode::Fill, &[(x, y), point(index), point(index + 1)]);
    }
}

impl Game for Stencil {
    fn load() -> Self {
        graphics::set_background_color(Color::rgb(0.09, 0.1, 0.14));
        Stencil
    }

    fn draw(&mut self) {
        let (mouse_x, mouse_y) = mouse::position();
        let turn = timer::time() as f32 * 0.5;

        // Mark the holes. Nothing drawn in here shows up by itself.
        graphics::stencil(|| {
            graphics::circle(DrawMode::Fill, mouse_x, mouse_y, 90.0);
            draw_star(520.0, 250.0, 130.0, turn);
        });

        // Only where the stencil was marked.
        graphics::set_stencil_test(CompareMode::Greater, 0);
        draw_pattern();

        // Only where it was not.
        graphics::set_stencil_test(CompareMode::Equal, 0);
        draw_grid();

        graphics::clear_stencil_test();
        graphics::set_color(Color::WHITE);
        graphics::print("Move the mouse to move the hole.", 16.0, 16.0);
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        if key == Key::Escape {
            event::quit();
        }
    }
}

fn main() {
    smitten::run::<Stencil>(Config {
        title: "Smitten stencil".into(),
        width: WIDTH,
        height: HEIGHT,
        ..Config::default()
    });
}
