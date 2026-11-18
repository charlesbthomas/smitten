//! Makes a gamepad rumble, to try `Joystick::set_vibration` with a real
//! controller in your hands.
//!
//! - Left trigger: the heavy, low motor, as hard as you squeeze.
//! - Right trigger: the light, buzzing motor, as hard as you squeeze.
//! - A: a short tap on both motors.
//! - B: a long, strong rumble on both motors for two seconds.
//! - X: stop.
//! - Escape: quit.
//!
//! Run it with `cargo run --example rumble`.

use smitten::graphics::{self, DrawMode};
use smitten::{Color, Config, Game, GamepadAxis, GamepadButton, Joystick, Key, event, joystick};

/// How long each request from the triggers lasts. It is renewed every
/// frame while a trigger is held, so letting go stops the rumble quickly.
const HOLD_SECONDS: f32 = 0.1;

struct Rumble {
    /// What happened last, shown on screen.
    status: String,
    /// Whether the triggers were driving the motors last frame.
    held: bool,
}

impl Rumble {
    /// Asks the gamepad to rumble and says on screen how it answered.
    fn request(&mut self, pad: Joystick, what: &str, left: f32, right: f32, seconds: f32) {
        let taken = pad.set_vibration(left, right, seconds);
        self.status = format!(
            "{what}: left {left:.2}, right {right:.2}, {seconds:.1} s. The gamepad {}.",
            if taken { "took it" } else { "REFUSED it" }
        );
    }
}

impl Game for Rumble {
    fn load() -> smitten::Result<Self> {
        Ok(Rumble {
            status: "Nothing asked for yet.".to_string(),
            held: false,
        })
    }

    fn update(&mut self, _dt: f32) {
        let Some(pad) = joystick::gamepads().into_iter().next() else {
            return;
        };
        let left = pad.gamepad_axis(GamepadAxis::TriggerLeft);
        let right = pad.gamepad_axis(GamepadAxis::TriggerRight);
        let held = left > 0.05 || right > 0.05;
        if held {
            self.request(pad, "Triggers", left, right, HOLD_SECONDS);
        } else if self.held {
            // The triggers were just let go.
            pad.stop_vibration();
            self.status = "Triggers released: stopped.".to_string();
        }
        self.held = held;
    }

    fn gamepad_pressed(&mut self, pad: Joystick, button: GamepadButton) {
        match button {
            GamepadButton::A => self.request(pad, "A, a short tap", 0.5, 0.5, 0.2),
            GamepadButton::B => self.request(pad, "B, a long rumble", 1.0, 1.0, 2.0),
            GamepadButton::X => {
                let taken = pad.stop_vibration();
                self.status = format!(
                    "X, stop. The gamepad {}.",
                    if taken { "took it" } else { "REFUSED it" }
                );
            }
            _ => {}
        }
    }

    fn joystick_added(&mut self, pad: Joystick) {
        self.status = format!("{} was plugged in.", pad.name());
    }

    fn joystick_removed(&mut self, _pad: Joystick) {
        self.status = "The gamepad was unplugged.".to_string();
        self.held = false;
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        if key == Key::Escape {
            event::quit();
        }
    }

    fn draw(&mut self) {
        graphics::clear(Color::rgb(0.09, 0.09, 0.14));
        graphics::set_color(Color::WHITE);

        let Some(pad) = joystick::gamepads().into_iter().next() else {
            graphics::print("Plug in a gamepad to try its rumble.", (40.0, 40.0));
            graphics::print("Escape quits.", (40.0, 70.0));
            return;
        };

        graphics::print(&format!("Gamepad: {}", pad.name()), (40.0, 40.0));
        let supported = pad.is_vibration_supported();
        graphics::set_color(if supported {
            Color::rgb(0.5, 0.9, 0.5)
        } else {
            Color::rgb(1.0, 0.5, 0.4)
        });
        graphics::print(
            if supported {
                "It says it can rumble."
            } else {
                "It says it cannot rumble. Try the buttons anyway."
            },
            (40.0, 70.0),
        );

        // A bar for each motor, filled as far as its trigger is squeezed.
        let bars = [
            ("Left trigger: heavy motor", GamepadAxis::TriggerLeft, 150.0),
            (
                "Right trigger: light motor",
                GamepadAxis::TriggerRight,
                230.0,
            ),
        ];
        for (label, axis, y) in bars {
            let amount = pad.gamepad_axis(axis).clamp(0.0, 1.0);
            graphics::set_color(Color::WHITE);
            graphics::print(label, (40.0, y));
            graphics::set_color(Color::rgb(0.95, 0.6, 0.2));
            graphics::rectangle(DrawMode::Fill, (40.0, y + 28.0), (400.0 * amount, 24.0));
            graphics::set_color(Color::WHITE);
            graphics::rectangle(DrawMode::Line, (40.0, y + 28.0), (400.0, 24.0));
        }

        graphics::print(
            "A: short tap     B: two strong seconds     X: stop",
            (40.0, 330.0),
        );
        graphics::print("Escape quits.", (40.0, 360.0));
        graphics::set_color(Color::rgb(0.7, 0.8, 1.0));
        graphics::print(&self.status, (40.0, 420.0));
    }
}

fn main() {
    smitten::run::<Rumble>(Config {
        title: "Smitten: rumble".to_string(),
        ..Config::default()
    });
}
