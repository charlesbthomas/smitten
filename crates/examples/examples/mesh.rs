//! Meshes: shapes with a color and a place in a texture at every corner.
//!
//! Shows a gradient, a color wheel built as a fan, and an image stretched
//! over a strip whose corners move, which makes it wave like a flag.
//! Press Escape to quit.

use std::f32::consts::TAU;

use smitten::graphics::{Align, Mesh, MeshDrawMode, MeshVertex};
use smitten::{Color, Config, Filter, Game, Key, event, graphics, timer};

/// How many pairs of corners the flag has along its length.
const FLAG_COLUMNS: usize = 24;
const FLAG_WIDTH: f32 = 320.0;
const FLAG_HEIGHT: f32 = 200.0;

struct Meshes {
    gradient: Mesh,
    wheel: Mesh,
    flag: Mesh,
}

/// A color around the rainbow, where `turn` goes from 0 to 1.
fn rainbow(turn: f32) -> Color {
    let channel = |offset: f32| 0.5 + 0.5 * ((turn + offset) * TAU).cos();
    Color::rgb(channel(0.0), channel(2.0 / 3.0), channel(1.0 / 3.0))
}

/// The flag's corners at `time`: a top and bottom corner for each column,
/// in the order a strip wants them.
fn flag_vertices(time: f32) -> Vec<MeshVertex> {
    (0..=FLAG_COLUMNS)
        .flat_map(|column| {
            let along = column as f32 / FLAG_COLUMNS as f32;
            // The flag is pinned at the pole and swings more further out.
            let lift = (along * 9.0 - time * 5.0).sin() * 18.0 * along;
            let x = along * FLAG_WIDTH;
            [
                MeshVertex::new(x, lift, along, 0.0),
                MeshVertex::new(x, FLAG_HEIGHT + lift, along, 1.0),
            ]
        })
        .collect()
}

impl Game for Meshes {
    fn load() -> Self {
        graphics::set_background_color(Color::rgb(0.11, 0.12, 0.15));

        let (dusk, night) = (Color::rgb(0.95, 0.45, 0.35), Color::rgb(0.15, 0.1, 0.4));
        let gradient = graphics::new_mesh(
            &[
                MeshVertex::colored(0.0, 0.0, night),
                MeshVertex::colored(220.0, 0.0, night),
                MeshVertex::colored(220.0, 160.0, dusk),
                MeshVertex::colored(0.0, 160.0, dusk),
            ],
            MeshDrawMode::Fan,
        );

        // A white center, then a ring of colors that ends where it began.
        let mut rim = vec![MeshVertex::colored(0.0, 0.0, Color::WHITE)];
        rim.extend((0..=48).map(|step| {
            let turn = step as f32 / 48.0;
            let angle = turn * TAU;
            MeshVertex::colored(90.0 * angle.cos(), 90.0 * angle.sin(), rainbow(turn))
        }));
        let wheel = graphics::new_mesh(&rim, MeshDrawMode::Fan);

        let heart = graphics::new_image("heart.png");
        heart.set_filter(Filter::Nearest);
        let mut flag = graphics::new_mesh(&flag_vertices(0.0), MeshDrawMode::Strip);
        flag.set_texture(Some(&heart));

        Meshes {
            gradient,
            wheel,
            flag,
        }
    }

    fn update(&mut self, _dt: f32) {
        self.flag.set_vertices(&flag_vertices(timer::time() as f32));
    }

    fn draw(&mut self) {
        graphics::set_color(Color::WHITE);
        graphics::draw_mesh(&self.gradient, 50.0, 60.0);
        graphics::draw_mesh_ex(
            &self.wheel,
            160.0,
            400.0,
            timer::time() as f32 * 0.3,
            1.0,
            1.0,
            0.0,
            0.0,
        );

        // The pole, then the flag.
        graphics::set_color(Color::rgb(0.6, 0.6, 0.65));
        graphics::rectangle(graphics::DrawMode::Fill, 392.0, 110.0, 8.0, 400.0);
        graphics::set_color(Color::WHITE);
        graphics::draw_mesh(&self.flag, 400.0, 120.0);

        graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.7));
        graphics::printf("a color at each corner", 50.0, 230.0, 220.0, Align::Center);
        graphics::printf("a fan", 50.0, 505.0, 220.0, Align::Center);
        graphics::printf(
            "an image on a moving strip",
            400.0,
            530.0,
            320.0,
            Align::Center,
        );
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        if key == Key::Escape {
            event::quit();
        }
    }
}

fn main() {
    smitten::run::<Meshes>(Config {
        title: "Meshes".to_string(),
        ..Config::default()
    });
}
