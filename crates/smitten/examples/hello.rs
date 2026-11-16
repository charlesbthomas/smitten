//! Opens a window whose color drifts over time.
//! Hold the arrow keys to shift the color, press Escape to quit.

use smitten::{Color, Config, Game, Key, event, graphics, keyboard, timer, window};

struct Hello {
    hue_shift: f32,
}

impl Game for Hello {
    fn load() -> Self {
        Hello { hue_shift: 0.0 }
    }

    fn update(&mut self, dt: f32) {
        if keyboard::is_down(Key::Right) {
            self.hue_shift += dt;
        }
        if keyboard::is_down(Key::Left) {
            self.hue_shift -= dt;
        }
        window::set_title(&format!("Hello, Smitten ({} FPS)", timer::fps()));
    }

    fn draw(&mut self) {
        let t = timer::time() as f32 * 0.5 + self.hue_shift;
        graphics::clear(Color::rgb(
            0.5 + 0.4 * t.sin(),
            0.5 + 0.4 * (t + 2.1).sin(),
            0.5 + 0.4 * (t + 4.2).sin(),
        ));
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        if key == Key::Escape {
            event::quit();
        }
    }
}

fn main() {
    smitten::run::<Hello>(Config {
        title: "Hello, Smitten".to_string(),
        resizable: true,
        ..Config::default()
    });
}
