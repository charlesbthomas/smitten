//! Every kind of input in one window.
//!
//! - Arrow keys, WASD or a gamepad's left stick move the square.
//! - The ring follows the mouse, and the wheel resizes it.
//! - Left click drops a dot, right click clears them.
//! - Each finger on a touch screen gets a green circle, larger the
//!   harder it presses.
//! - Tab swaps the pointer for a cursor drawn from an image, and back.
//! - Typed text and the active gamepad appear in the title bar.
//! - Escape asks whether to quit, in a message box with two buttons.
//!   The gamepad's Start button quits straight away.

use smitten::graphics::{DrawMode, ImageData};
use smitten::{
    Color, Config, Cursor, Game, GamepadAxis, GamepadButton, Joystick, Key, MessageBoxKind,
    MouseButton, event, graphics, joystick, keyboard, mouse, touch, window,
};

const SPEED: f32 = 300.0;
const STICK_DEADZONE: f32 = 0.2;

struct Input {
    x: f32,
    y: f32,
    ring_radius: f32,
    dots: Vec<(f32, f32)>,
    typed: String,
    target_cursor: mouse::ImageCursor,
}

impl Input {
    /// The direction the player is asking to move, from keys and sticks.
    fn movement() -> (f32, f32) {
        let mut dx = 0.0;
        let mut dy = 0.0;
        if keyboard::is_any_down(&[Key::Left, Key::A]) {
            dx -= 1.0;
        }
        if keyboard::is_any_down(&[Key::Right, Key::D]) {
            dx += 1.0;
        }
        if keyboard::is_any_down(&[Key::Up, Key::W]) {
            dy -= 1.0;
        }
        if keyboard::is_any_down(&[Key::Down, Key::S]) {
            dy += 1.0;
        }
        for pad in joystick::gamepads() {
            let (stick_x, stick_y) = (
                pad.gamepad_axis(GamepadAxis::LeftX),
                pad.gamepad_axis(GamepadAxis::LeftY),
            );
            if stick_x.abs() > STICK_DEADZONE {
                dx += stick_x;
            }
            if stick_y.abs() > STICK_DEADZONE {
                dy += stick_y;
            }
        }
        (dx.clamp(-1.0, 1.0), dy.clamp(-1.0, 1.0))
    }

    fn update_title(&self) {
        let pad = joystick::gamepads()
            .first()
            .map_or("none".to_string(), Joystick::name);
        window::set_title(&format!(
            "Input | typed: \"{}\" | gamepad: {pad}",
            self.typed
        ));
    }
}

impl Game for Input {
    fn load() -> Self {
        graphics::set_background_color(Color::rgb(0.1, 0.12, 0.16));
        let game = Input {
            x: 400.0,
            y: 300.0,
            ring_radius: 24.0,
            dots: Vec::new(),
            typed: String::new(),
            target_cursor: target_cursor(),
        };
        game.update_title();
        game
    }

    fn update(&mut self, dt: f32) {
        let (dx, dy) = Self::movement();
        self.x += dx * SPEED * dt;
        self.y += dy * SPEED * dt;
    }

    fn draw(&mut self) {
        graphics::set_color(Color::rgb(0.95, 0.75, 0.3));
        for &(x, y) in &self.dots {
            graphics::circle(DrawMode::Fill, x, y, 6.0);
        }

        graphics::set_color(Color::rgb(0.3, 0.7, 0.95));
        graphics::rectangle(DrawMode::Fill, self.x - 20.0, self.y - 20.0, 40.0, 40.0);

        let held = mouse::is_down(MouseButton::Left);
        graphics::set_color(if held {
            Color::rgb(1.0, 0.4, 0.5)
        } else {
            Color::WHITE
        });
        graphics::set_line_width(3.0);
        let (mouse_x, mouse_y) = mouse::position();
        graphics::circle(DrawMode::Line, mouse_x, mouse_y, self.ring_radius);

        graphics::set_color(Color::rgb(0.4, 0.9, 0.5));
        for id in touch::touches() {
            if let (Some((x, y)), Some(pressure)) = (touch::position(id), touch::pressure(id)) {
                graphics::circle(DrawMode::Fill, x, y, 16.0 + 24.0 * pressure);
            }
        }
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        match key {
            Key::Escape => {
                let choice = window::show_message_box_with_buttons(
                    "Quit?",
                    "Leave the input example?",
                    MessageBoxKind::Warning,
                    &["Quit", "Keep playing"],
                );
                if choice == Some(0) {
                    event::quit();
                }
            }
            Key::Tab => {
                if mouse::image_cursor().is_some() {
                    mouse::set_cursor(Cursor::Arrow);
                } else {
                    mouse::set_image_cursor(&self.target_cursor);
                }
            }
            Key::Backspace => {
                self.typed.pop();
                self.update_title();
            }
            _ => {}
        }
    }

    fn text_input(&mut self, text: &str) {
        self.typed.push_str(text);
        self.update_title();
    }

    fn mouse_pressed(&mut self, x: f32, y: f32, button: MouseButton, _presses: u32) {
        match button {
            MouseButton::Left => self.dots.push((x, y)),
            MouseButton::Right => self.dots.clear(),
            _ => {}
        }
    }

    fn wheel_moved(&mut self, _dx: f32, dy: f32) {
        self.ring_radius = (self.ring_radius + dy * 4.0).clamp(8.0, 120.0);
    }

    fn joystick_added(&mut self, _joystick: Joystick) {
        self.update_title();
    }

    fn joystick_removed(&mut self, _joystick: Joystick) {
        self.update_title();
    }

    fn gamepad_pressed(&mut self, _joystick: Joystick, button: GamepadButton) {
        if button == GamepadButton::Start {
            event::quit();
        }
    }
}

/// A 21 by 21 target, drawn pixel by pixel, that points from its middle.
fn target_cursor() -> mouse::ImageCursor {
    let mut data = ImageData::new(21, 21);
    for y in 0..21_i32 {
        for x in 0..21_i32 {
            let (dx, dy) = (x - 10, y - 10);
            let distance = ((dx * dx + dy * dy) as f32).sqrt();
            let ring = (distance - 8.0).abs() < 1.0;
            let cross = (dx == 0 || dy == 0) && distance <= 10.0;
            if ring || cross {
                data.set_pixel(x as u32, y as u32, Color::rgb(0.4, 0.9, 0.5));
            }
        }
    }
    mouse::ImageCursor::from_image_data(&data, 10, 10).expect("the cursor image is valid")
}

fn main() {
    smitten::run::<Input>(Config {
        title: "Input".to_string(),
        ..Config::default()
    });
}
