//! Opens a window whose color drifts over time.
//! Hold the arrow keys to shift the color, press F11 to switch between a
//! window and fullscreen, and press Escape to quit.

use smitten::{Color, Config, Game, Key, event, graphics, keyboard, timer, window};

struct Hello {
    hue_shift: f32,
}

impl Game for Hello {
    fn load() -> smitten::Result<Self> {
        Ok(Hello { hue_shift: 0.0 })
    }

    fn update(&mut self, dt: f32) {
        if keyboard::is_down(Key::Right) {
            self.hue_shift += dt;
        }
        if keyboard::is_down(Key::Left) {
            self.hue_shift -= dt;
        }
        let (width, height) = graphics::dimensions();
        let (pixel_width, pixel_height) = graphics::pixel_dimensions();
        window::set_title(&format!(
            "Hello, Smitten ({width} x {height} units, {pixel_width} x {pixel_height} pixels, \
             display scale {}, {} FPS)",
            window::dpi_scale(),
            timer::fps()
        ));
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
        match key {
            Key::F11 => window::set_fullscreen(!window::fullscreen()),
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Hello>(Config {
        title: "Hello, Smitten".to_string(),
        resizable: true,
        icon: Some("heart.png".into()),
        ..Config::default()
    });
}
