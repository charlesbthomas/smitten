//! A box of things to throw around.
//!
//! Click to drop a crate or a ball at the mouse, and hold the right
//! button to drag whatever is under it. A chain of planks hangs from the
//! ceiling and a platform slides back and forth. Things turn bright for a
//! moment when they hit something.
//!
//! Space clears everything you added, D shows what the simulation sees,
//! and Escape quits.

use smitten::MouseClick;
use smitten::Vec2;
use smitten::graphics::DrawMode;
use smitten::physics::{Body, BodyType, World};
use smitten::{Color, Config, Game, Key, MouseButton, event, graphics, math, mouse, timer};

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;

enum Shape {
    Crate(f32),
    Ball(f32),
    Plank(f32, f32),
}

struct Thing {
    body: Body,
    shape: Shape,
    color: Color,
    /// Counts down from 1.0 after a hit, for the flash.
    flash: f32,
}

struct Physics {
    world: World,
    things: Vec<Thing>,
    /// How many things belong to the scene and survive a clear.
    permanent: usize,
    platform: Body,
    dragged: Option<Body>,
    debug: bool,
}

impl Physics {
    fn add(&mut self, body: Body, shape: Shape, color: Color) {
        // The index lets a contact find the thing its body belongs to.
        body.set_user_data(self.things.len() as u64 + 1);
        self.things.push(Thing {
            body,
            shape,
            color,
            flash: 0.0,
        });
    }

    fn drop_something(&mut self, x: f32, y: f32) {
        let body = self.world.new_body((x, y), BodyType::Dynamic);
        let color = Color::rgb(
            math::random_range(0.5, 1.0),
            math::random_range(0.4, 0.8),
            math::random_range(0.3, 0.7),
        );
        let shape = if math::chance(0.5) {
            let size = math::random_range(24.0, 48.0);
            body.add_rectangle((size, size)).set_friction(0.6);
            Shape::Crate(size)
        } else {
            let radius = math::random_range(12.0, 24.0);
            body.add_circle(radius).set_restitution(0.5);
            Shape::Ball(radius)
        };
        body.set_angle(math::random_range(0.0, 3.0));
        self.add(body, shape, color);
    }
}

impl Game for Physics {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.1, 0.11, 0.15));
        let world = World::new((0.0, 900.0));

        // The room: a floor, two walls and a ceiling on one static body.
        let room = world.new_body((0.0, 0.0), BodyType::Static);
        room.add_chain(
            &[
                (10.0, 10.0),
                (WIDTH - 10.0, 10.0),
                (WIDTH - 10.0, HEIGHT - 10.0),
                (10.0, HEIGHT - 10.0),
            ],
            true,
        );
        // A ramp in the bottom-left corner.
        room.add_polygon(&[
            (10.0, HEIGHT - 10.0),
            (10.0, HEIGHT - 110.0),
            (210.0, HEIGHT - 10.0),
        ]);

        let platform = world.new_body((520.0, 380.0), BodyType::Kinematic);
        platform.add_rectangle((160.0, 16.0)).set_friction(0.9);
        platform.set_linear_velocity((80.0, 0.0));

        let mut game = Physics {
            world: world.clone(),
            things: Vec::new(),
            permanent: 0,
            platform: platform.clone(),
            dragged: None,
            debug: false,
        };
        game.add(
            platform,
            Shape::Plank(160.0, 16.0),
            Color::rgb(0.45, 0.7, 0.9),
        );

        // A chain of planks, each hinged to the one before it.
        let mut previous = room;
        for link in 0..6 {
            let x = 250.0 + link as f32 * 40.0;
            let plank = world.new_body((x + 20.0, 60.0), BodyType::Dynamic);
            plank.add_rectangle((36.0, 10.0));
            world.new_revolute_joint(&previous, &plank, (x, 60.0), false);
            previous = plank.clone();
            game.add(plank, Shape::Plank(36.0, 10.0), Color::rgb(0.8, 0.65, 0.4));
        }
        game.permanent = game.things.len();

        for _ in 0..12 {
            let (x, y) = (
                math::random_range(100.0, 700.0),
                math::random_range(80.0, 300.0),
            );
            game.drop_something(x, y);
        }
        Ok(game)
    }

    fn update(&mut self, dt: f32) {
        // The platform turns around at the ends of its track.
        let smitten::Vec2 { x, y: _ } = self.platform.position();
        if x > 620.0 {
            self.platform.set_linear_velocity((-80.0, 0.0));
        } else if x < 420.0 {
            self.platform.set_linear_velocity((80.0, 0.0));
        }

        // Dragging: steer the body toward the mouse by its velocity, so it
        // still pushes what is in its way.
        if let Some(body) = &self.dragged {
            let smitten::Vec2 {
                x: mouse_x,
                y: mouse_y,
            } = mouse::position();
            let smitten::Vec2 { x, y } = body.position();
            body.set_linear_velocity(((mouse_x - x) * 12.0, (mouse_y - y) * 12.0));
        }

        // A long frame is split up so nothing tunnels through the floor.
        self.world.update(dt.min(1.0 / 30.0));

        for contact in self.world.contacts() {
            if !contact.began {
                continue;
            }
            for fixture in [&contact.a, &contact.b] {
                let index = fixture.body().map_or(0, |body| body.user_data()) as usize;
                if let Some(thing) = index.checked_sub(1).and_then(|i| self.things.get_mut(i)) {
                    thing.flash = 1.0;
                }
            }
        }
        for thing in &mut self.things {
            thing.flash = (thing.flash - dt * 3.0).max(0.0);
        }
    }

    fn draw(&mut self) {
        // The room.
        graphics::set_color(Color::rgb(0.3, 0.32, 0.4));
        graphics::set_line_width(4.0);
        graphics::rectangle(DrawMode::Line, (10.0, 10.0), (WIDTH - 20.0, HEIGHT - 20.0));
        graphics::polygon(
            DrawMode::Fill,
            &[
                (10.0, HEIGHT - 10.0),
                (10.0, HEIGHT - 110.0),
                (210.0, HEIGHT - 10.0),
            ],
        );

        for thing in &self.things {
            let smitten::Vec2 { x, y } = thing.body.position();
            let flash = thing.flash;
            graphics::set_color(Color::rgb(
                thing.color.r + (1.0 - thing.color.r) * flash,
                thing.color.g + (1.0 - thing.color.g) * flash,
                thing.color.b + (1.0 - thing.color.b) * flash,
            ));
            // A body's position and angle go straight into the transform.
            graphics::push();
            graphics::translate((x, y));
            graphics::rotate(thing.body.angle());
            match thing.shape {
                Shape::Crate(size) => {
                    graphics::rectangle(DrawMode::Fill, (-size / 2.0, -size / 2.0), (size, size));
                }
                Shape::Plank(width, height) => {
                    graphics::rectangle(
                        DrawMode::Fill,
                        (-width / 2.0, -height / 2.0),
                        (width, height),
                    );
                }
                Shape::Ball(radius) => {
                    graphics::circle(DrawMode::Fill, (0.0, 0.0), radius);
                    graphics::set_color(Color::rgba(0.0, 0.0, 0.0, 0.4));
                    graphics::set_line_width(2.0);
                    graphics::line((0.0, 0.0), (radius, 0.0));
                }
            }
            graphics::pop();
        }

        if self.debug {
            graphics::set_color(Color::rgb(0.3, 1.0, 0.4));
            graphics::set_line_width(1.0);
            self.world.draw_debug();
        }

        graphics::set_color(Color::WHITE);
        graphics::print(
            &format!(
                "{} bodies, {} FPS. Click: drop. Right button: drag. Space: clear. D: outlines.",
                self.world.body_count(),
                timer::fps()
            ),
            (24.0, 22.0),
        );
    }

    fn mouse_pressed(&mut self, click: MouseClick) {
        let MouseClick {
            position: Vec2 { x, y },
            button,
            ..
        } = click;
        match button {
            MouseButton::Left => self.drop_something(x, y),
            MouseButton::Right => {
                self.dragged = self
                    .world
                    .fixtures_at((x, y))
                    .iter()
                    .filter_map(|fixture| fixture.body())
                    .find(|body| body.body_type() == BodyType::Dynamic);
            }
            _ => {}
        }
    }

    fn mouse_released(&mut self, click: MouseClick) {
        let button = click.button;
        if button == MouseButton::Right {
            self.dragged = None;
        }
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        match key {
            Key::Space => {
                for thing in self.things.drain(self.permanent..) {
                    thing.body.destroy();
                }
                self.dragged = None;
            }
            Key::D => self.debug = !self.debug,
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Physics>(Config {
        title: "Smitten physics".to_string(),
        width: WIDTH as u32,
        height: HEIGHT as u32,
        ..Config::default()
    });
}
