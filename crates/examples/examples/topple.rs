//! Stack and Topple: build the tallest tower you can on a platform that
//! tips under its load.
//!
//! - Left and Right (or A and D, or the left stick) move the next block.
//! - Up (or W, or X on a gamepad) turns it a quarter turn.
//! - Space (or A on a gamepad) drops it.
//! - P saves a screenshot. R starts again. Escape quits.
//!
//! Every block that falls off costs a heart. With no hearts left the game
//! ends, and the best height is kept between runs.
//!
//! Run it with `cargo run --example topple`.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, TAU};

use smitten::audio::Source;
use smitten::graphics::{Align, DrawMode, Font, Image, LineStyle, ParticleSystem};
use smitten::physics::{Body, BodyType, World};
use smitten::sound::SoundData;
use smitten::{
    Color, Config, Game, GamepadButton, Joystick, Key, event, filesystem, graphics, joystick,
    keyboard, math,
};

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;
/// The side of one cell of a block.
const CELL: f32 = 30.0;
/// How many units count as a meter of tower.
const METER: f32 = 60.0;
/// Where the top of the platform rests.
const PLATFORM_Y: f32 = 500.0;
const PLATFORM_WIDTH: f32 = 260.0;
const PLATFORM_HEIGHT: f32 = 16.0;
/// How far below the top of the screen the next block waits.
const DROPPER_Y: f32 = 70.0;
/// How hard the platform pushes back toward level, as radians per second
/// squared for each radian of tilt, and how much it resists turning.
/// They decide how far a lopsided tower tips it and how long it wobbles
/// after a hit.
const SPRING: f32 = 17.2;
const DAMPING: f32 = 2.8;
const PLATFORM_DENSITY: f32 = 18.0;
/// Air resistance on a falling block, which keeps it from landing so
/// hard that it catapults the tower.
const FALL_DAMPING: f32 = 2.6;
const HEARTS: u32 = 3;
const BEST_FILE: &str = "topple-best.txt";

/// A block's shape: rectangles as `(x, y, width, height)` in cells, with
/// `(x, y)` the rectangle's center.
struct Kind {
    parts: &'static [(f32, f32, f32, f32)],
    color: Color,
}

const KINDS: [Kind; 7] = [
    // A bar.
    Kind {
        parts: &[(0.0, 0.0, 4.0, 1.0)],
        color: Color::rgb(0.36, 0.72, 0.9),
    },
    // A square.
    Kind {
        parts: &[(0.0, 0.0, 2.0, 2.0)],
        color: Color::rgb(0.96, 0.8, 0.3),
    },
    // An L.
    Kind {
        parts: &[(0.0, 0.25, 3.0, 1.0), (-1.0, -0.75, 1.0, 1.0)],
        color: Color::rgb(0.95, 0.55, 0.25),
    },
    // A T.
    Kind {
        parts: &[(0.0, 0.25, 3.0, 1.0), (0.0, -0.75, 1.0, 1.0)],
        color: Color::rgb(0.72, 0.5, 0.9),
    },
    // A brick.
    Kind {
        parts: &[(0.0, 0.0, 3.0, 1.5)],
        color: Color::rgb(0.9, 0.42, 0.42),
    },
    // A long thin plank.
    Kind {
        parts: &[(0.0, 0.0, 5.0, 0.6)],
        color: Color::rgb(0.5, 0.85, 0.55),
    },
    // A U, which other blocks can drop into.
    Kind {
        parts: &[
            (0.0, 0.5, 3.0, 1.0),
            (-1.0, -0.5, 1.0, 1.0),
            (1.0, -0.5, 1.0, 1.0),
        ],
        color: Color::rgb(0.4, 0.8, 0.8),
    },
];

struct Block {
    body: Body,
    kind: usize,
    /// Seconds since it first touched something, or `None` in the air.
    landed: Option<f32>,
    /// Counts down from 1.0 after a hit, for the flash.
    flash: f32,
}

impl Block {
    /// The highest point of the block, as a `y` in the world.
    fn top(&self) -> f32 {
        let mut top = f32::MAX;
        for &(x, y, width, height) in KINDS[self.kind].parts {
            for (corner_x, corner_y) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
                let smitten::Vec2 { x: _, y: world_y } = self.body.world_point((
                    (x + corner_x * width) * CELL,
                    (y + corner_y * height) * CELL,
                ));
                top = top.min(world_y);
            }
        }
        top
    }

    /// Whether it has come to rest, so that it counts toward the height.
    fn is_resting(&self) -> bool {
        let smitten::Vec2 { x: vx, y: vy } = self.body.linear_velocity();
        // Loose enough to keep counting while the tower sways.
        self.landed.is_some_and(|seconds| seconds > 0.6)
            && vx.hypot(vy) < 60.0
            && self.body.angular_velocity().abs() < 1.0
    }
}

struct Sounds {
    thud: Source,
    drop: Source,
    lost: Source,
    over: Source,
}

struct Topple {
    world: World,
    platform: Body,
    /// Every block in play, found by its body when a contact names one.
    blocks: HashMap<Body, Block>,
    /// The kind, position and quarter turns of the block waiting to be
    /// dropped, or `None` while the last one is still falling.
    next: Option<(usize, f32, u32)>,
    upcoming: usize,
    hearts: u32,
    /// The tower's height now and the most it has been this run, in units.
    height: f32,
    run_best: f32,
    /// The best of every run, read from and written to the save directory.
    best: f32,
    new_record: bool,
    over: bool,
    /// The world `y` shown at the top of the screen.
    camera: f32,
    shake: f32,
    dust: ParticleSystem,
    sounds: Sounds,
    heart: Image,
    big_font: Font,
    small_font: Font,
    /// Seconds since the run began, for the opening hint.
    time: f32,
    message: Option<(String, f32)>,
}

/// A soft white dot for the dust.
fn soft_dot(size: u32) -> Image {
    let center = (size as f32 - 1.0) / 2.0;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let distance = (x as f32 - center).hypot(y as f32 - center) / center;
            let alpha = (1.0 - distance).clamp(0.0, 1.0).powi(2);
            pixels.extend_from_slice(&[255, 255, 255, (alpha * 255.0) as u8]);
        }
    }
    Image::from_rgba(size, size, &pixels).expect("the dot is a valid image")
}

/// Makes a sound from a formula: `wave` gets the time in seconds and how
/// far through the sound it is, from 0 to 1.
fn synth(seconds: f32, wave: impl Fn(f32, f32) -> f32) -> Source {
    const RATE: u32 = 44_100;
    let count = (seconds * RATE as f32) as usize;
    let mut data = SoundData::new(count, RATE, 1);
    for index in 0..count {
        let time = index as f32 / RATE as f32;
        data.set_sample(index, 0, wave(time, index as f32 / count as f32));
    }
    Source::from_sound_data(&data)
}

fn make_sounds() -> Sounds {
    Sounds {
        // A low knock that drops in pitch, with a little noise on the hit.
        thud: synth(0.3, |time, progress| {
            let tone = (TAU * (110.0 - 60.0 * progress) * time).sin();
            let noise = math::random_range(-1.0, 1.0) * (1.0 - progress * 8.0).max(0.0);
            (tone * 0.8 + noise * 0.3) * (1.0 - progress).powi(3)
        }),
        // A short rising chirp as a block is let go.
        drop: synth(0.09, |time, progress| {
            (TAU * (500.0 + 500.0 * progress) * time).sin() * (1.0 - progress) * 0.35
        }),
        // A falling whistle for a block that went over the side.
        lost: synth(0.5, |time, progress| {
            (TAU * (520.0 - 330.0 * progress) * time).sin() * (1.0 - progress) * 0.4
        }),
        // Three falling notes for the end of the game.
        over: synth(1.2, |time, progress| {
            let note = [330.0, 262.0, 196.0][((progress * 3.0) as usize).min(2)];
            let fade = 1.0 - (progress * 3.0).fract() * 0.7;
            (TAU * note * time).sin() * fade * (1.0 - progress * 0.5) * 0.4
        }),
    }
}

impl Topple {
    /// Clears the tower and starts a new run. The best height stays.
    fn restart(&mut self) {
        for (body, _) in self.blocks.drain() {
            body.destroy();
        }
        self.platform.set_angle(0.0);
        self.platform.set_angular_velocity(0.0);
        self.upcoming = math::random_int(0, KINDS.len() as i64 - 1) as usize;
        self.next = None;
        self.new_piece();
        self.hearts = HEARTS;
        self.height = 0.0;
        self.run_best = 0.0;
        self.new_record = false;
        self.over = false;
        self.camera = 0.0;
        self.shake = 0.0;
        self.time = 0.0;
        self.message = None;
    }

    /// Puts the upcoming block in the dropper and picks the one after.
    fn new_piece(&mut self) {
        let x = self.next.map_or(WIDTH / 2.0, |(_, x, _)| x);
        self.next = Some((self.upcoming, x, 0));
        self.upcoming = math::random_int(0, KINDS.len() as i64 - 1) as usize;
    }

    /// Lets go of the waiting block.
    fn drop_piece(&mut self) {
        if self.over {
            return;
        }
        let Some((kind, x, turns)) = self.next.take() else {
            return;
        };
        let body = self
            .world
            .new_body((x, self.camera + DROPPER_Y), BodyType::Dynamic);
        for &(part_x, part_y, width, height) in KINDS[kind].parts {
            body.add_rectangle_at(
                (part_x * CELL, part_y * CELL),
                (width * CELL, height * CELL),
                0.0,
            )
            .set_friction(0.9)
            .set_restitution(0.0);
        }
        body.set_angle(turns as f32 * FRAC_PI_2);
        body.set_linear_damping(FALL_DAMPING);
        self.blocks.insert(
            body.clone(),
            Block {
                body,
                kind,
                landed: None,
                flash: 0.0,
            },
        );
        self.sounds
            .drop
            .play_once_with(0.6, math::random_range(0.9, 1.1));
    }

    fn turn_piece(&mut self) {
        if let Some((_, _, turns)) = &mut self.next {
            *turns = (*turns + 1) % 4;
        }
    }

    /// What follows a block hitting something at `speed`.
    fn impact(&mut self, x: f32, y: f32, speed: f32) {
        let strength = (speed / 500.0).clamp(0.0, 1.0);
        if strength < 0.08 {
            return;
        }
        self.shake = self.shake.max(strength * 9.0);
        self.dust.set_position((x, y));
        self.dust.emit((strength * 22.0) as usize + 3);
        self.sounds
            .thud
            .play_once_with(0.25 + strength * 0.75, math::random_range(0.85, 1.2));
        if let Some(pad) = joystick::gamepads().first() {
            pad.set_vibration(strength, strength * 0.4, 0.08 + strength * 0.15);
        }
    }

    /// The first block that has fallen out of the world, if any.
    fn fallen(&self) -> Option<Body> {
        self.blocks
            .keys()
            .find(|body| body.y() > PLATFORM_Y + 420.0)
            .cloned()
    }

    fn lose_heart(&mut self) {
        self.hearts = self.hearts.saturating_sub(1);
        self.shake = self.shake.max(6.0);
        if self.hearts > 0 {
            self.sounds.lost.play_once_with(0.7, 1.0);
            return;
        }
        self.over = true;
        self.next = None;
        self.sounds.over.play_once_with(0.8, 1.0);
        if let Some(pad) = joystick::gamepads().first() {
            pad.set_vibration(1.0, 1.0, 0.5);
        }
        if self.run_best > self.best {
            self.best = self.run_best;
            self.new_record = true;
            if let Err(error) = filesystem::write(BEST_FILE, format!("{}", self.best)) {
                eprintln!("topple: could not save the best height: {error}");
            }
        }
    }

    fn say(&mut self, text: &str) {
        self.message = Some((text.to_string(), 2.5));
    }

    /// How far left or right the player is steering, from -1 to 1.
    fn steering(&self) -> f32 {
        let mut steer = keyboard::axis(&[Key::Left, Key::A], &[Key::Right, Key::D]);
        if let Some(pad) = joystick::gamepads().first() {
            steer += pad.left_stick(0.2).x;
            if pad.is_gamepad_down(GamepadButton::DpadLeft) {
                steer -= 1.0;
            }
            if pad.is_gamepad_down(GamepadButton::DpadRight) {
                steer += 1.0;
            }
        }
        steer.clamp(-1.0, 1.0)
    }
}

impl Game for Topple {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.09, 0.1, 0.16));
        graphics::set_line_style(LineStyle::Smooth);

        let world = World::new((0.0, 900.0));
        world.set_meter(METER);

        // The platform turns on a pin through its middle.
        let post = world.new_body((WIDTH / 2.0, PLATFORM_Y + 60.0), BodyType::Static);
        let platform = world.new_body(
            (WIDTH / 2.0, PLATFORM_Y + PLATFORM_HEIGHT / 2.0),
            BodyType::Dynamic,
        );
        platform
            .add_rectangle((PLATFORM_WIDTH, PLATFORM_HEIGHT))
            .set_friction(1.0)
            .set_density(PLATFORM_DENSITY);
        world.new_revolute_joint(
            &post,
            &platform,
            (WIDTH / 2.0, PLATFORM_Y + PLATFORM_HEIGHT / 2.0),
            false,
        );

        let mut dust = ParticleSystem::new(&soft_dot(24), 400);
        dust.set_particle_lifetime(0.3, 0.8);
        dust.set_spread(TAU);
        dust.set_speed(30.0, 160.0);
        dust.set_linear_damping(2.0, 4.0);
        dust.set_linear_acceleration((0.0, 80.0), (0.0, 200.0));
        dust.set_sizes(&[0.5, 0.9, 0.3]);
        dust.set_colors(&[
            Color::rgba(1.0, 0.95, 0.85, 0.7),
            Color::rgba(0.8, 0.75, 0.7, 0.0),
        ]);

        let best = filesystem::read_string(BEST_FILE)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0.0);

        let mut game = Topple {
            world,
            platform,
            blocks: HashMap::new(),
            next: None,
            upcoming: 0,
            hearts: HEARTS,
            height: 0.0,
            run_best: 0.0,
            best,
            new_record: false,
            over: false,
            camera: 0.0,
            shake: 0.0,
            dust,
            sounds: make_sounds(),
            heart: Image::load("heart.png")?,
            big_font: Font::default_at(44.0),
            small_font: Font::default_at(16.0),
            time: 0.0,
            message: None,
        };
        game.restart();
        Ok(game)
    }

    fn update(&mut self, dt: f32) {
        // A long frame is cut short so nothing tunnels through the tower.
        let dt = dt.min(1.0 / 30.0);
        self.time += dt;

        let steer = self.steering();
        if let Some((kind, x, turns)) = &mut self.next {
            // Keep the whole block on screen, whichever way it is turned.
            let reach = KINDS[*kind]
                .parts
                .iter()
                .map(|&(part_x, part_y, width, height)| {
                    if *turns % 2 == 0 {
                        part_x.abs() + width / 2.0
                    } else {
                        part_y.abs() + height / 2.0
                    }
                })
                .fold(0.0, f32::max)
                * CELL;
            *x = (*x + steer * 320.0 * dt).clamp(reach + 8.0, WIDTH - reach - 8.0);
        }

        // The platform is sprung toward level. A heavier or more lopsided
        // tower pushes it further over.
        let angle = self.platform.angle();
        self.platform.apply_angular_acceleration(
            -angle * SPRING - self.platform.angular_velocity() * DAMPING,
        );

        self.world.update(dt);

        // Blocks that hit something for the first time, or hard.
        let mut impacts = Vec::new();
        for contact in self.world.contacts() {
            if !contact.began {
                continue;
            }
            let hard = contact.speed > 40.0;
            for fixture in [&contact.a, &contact.b] {
                let Some(block) = fixture.body().and_then(|body| self.blocks.get_mut(&body)) else {
                    continue;
                };
                if block.landed.is_none() {
                    block.landed = Some(0.0);
                    block.body.set_linear_damping(0.0);
                }
                if hard {
                    block.flash = 1.0;
                }
            }
            if hard {
                impacts.push((contact.x, contact.y, contact.speed));
            }
        }
        for (x, y, speed) in impacts {
            self.impact(x, y, speed);
        }

        for block in self.blocks.values_mut() {
            block.flash = (block.flash - dt * 4.0).max(0.0);
            if let Some(seconds) = &mut block.landed {
                *seconds += dt;
            }
        }

        // Blocks that went over the side.
        while let Some(body) = self.fallen() {
            self.blocks.remove(&body);
            body.destroy();
            if !self.over {
                self.lose_heart();
            }
        }

        // The tower is as tall as its highest block at rest.
        self.height = self
            .blocks
            .values()
            .filter(|block| block.is_resting())
            .map(|block| PLATFORM_Y - block.top())
            .fold(0.0, f32::max);
        if !self.over {
            self.run_best = self.run_best.max(self.height);
        }

        // The next block appears once the last one has landed.
        let falling = self.blocks.values().any(|block| block.landed.is_none());
        if self.next.is_none() && !falling && !self.over {
            self.new_piece();
        }

        // The camera rises to keep room above the tower.
        let highest = self
            .blocks
            .values()
            .filter(|block| {
                // Not one that is on its way over the side.
                let smitten::Vec2 { x: vx, y: vy } = block.body.linear_velocity();
                block.landed.is_some() && vx.hypot(vy) < 150.0
            })
            .map(Block::top)
            .fold(PLATFORM_Y, f32::min);
        let target = (highest - 300.0).min(0.0);
        self.camera += (target - self.camera) * (dt * 2.5).min(1.0);

        self.shake = (self.shake - dt * 30.0).max(0.0);
        self.dust.update(dt);
        if let Some((_, seconds)) = &mut self.message {
            *seconds -= dt;
            if *seconds <= 0.0 {
                self.message = None;
            }
        }
    }

    fn draw(&mut self) {
        graphics::push();
        graphics::translate((
            math::random_range(-1.0, 1.0) * self.shake,
            math::random_range(-1.0, 1.0) * self.shake - self.camera,
        ));
        self.draw_world();
        graphics::pop();
        self.draw_hud();
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        if is_repeat {
            return;
        }
        match key {
            Key::Space | Key::Down | Key::S => {
                if self.over {
                    self.restart();
                } else {
                    self.drop_piece();
                }
            }
            Key::Up | Key::W => self.turn_piece(),
            Key::Return if self.over => self.restart(),
            Key::R => self.restart(),
            Key::P => {
                graphics::save_screenshot("topple.png");
                self.say("Saved topple.png in the save directory");
            }
            Key::Escape => event::quit(),
            _ => {}
        }
    }

    fn gamepad_pressed(&mut self, _pad: Joystick, button: GamepadButton) {
        match button {
            GamepadButton::A if self.over => self.restart(),
            GamepadButton::A => self.drop_piece(),
            GamepadButton::X | GamepadButton::B | GamepadButton::DpadUp => self.turn_piece(),
            GamepadButton::Start => self.restart(),
            _ => {}
        }
    }
}

impl Topple {
    /// Draws one block's shape around the current origin.
    fn draw_shape(kind: usize, fill: Color, edge: Color) {
        for &(x, y, width, height) in KINDS[kind].parts {
            graphics::set_color(fill);
            graphics::rectangle(
                DrawMode::Fill,
                ((x - width / 2.0) * CELL, (y - height / 2.0) * CELL),
                (width * CELL, height * CELL),
            );
        }
        graphics::set_color(edge);
        graphics::set_line_width(2.0);
        for &(x, y, width, height) in KINDS[kind].parts {
            graphics::rectangle(
                DrawMode::Line,
                (
                    (x - width / 2.0) * CELL + 1.0,
                    (y - height / 2.0) * CELL + 1.0,
                ),
                (width * CELL - 2.0, height * CELL - 2.0),
            );
        }
    }

    fn draw_world(&self) {
        graphics::set_font(&self.small_font);

        // A ruler up the left side, one mark a meter.
        let top_meter = ((PLATFORM_Y - self.camera) / METER).ceil() as i32;
        graphics::set_line_width(1.0);
        for meter in 1..=top_meter {
            let y = PLATFORM_Y - meter as f32 * METER;
            graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.07));
            graphics::line((0.0, y), (WIDTH, y));
            // Not under the height readout in the corner.
            if y - self.camera > 76.0 {
                graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.35));
                graphics::print(&format!("{meter} m"), (8.0, y - 20.0));
            }
        }

        // The best height of every run, as a line to beat.
        if self.best > 0.0 {
            let y = PLATFORM_Y - self.best;
            graphics::set_color(Color::rgba(1.0, 0.85, 0.3, 0.7));
            graphics::set_line_width(2.0);
            graphics::dashed_line((0.0, y), (WIDTH, y), 14.0, 10.0);
            graphics::printf("best", (0.0, y - 22.0), WIDTH - 10.0, Align::Right);
        }

        // The post and the platform on it.
        graphics::set_color(Color::rgb(0.26, 0.28, 0.36));
        graphics::polygon(
            DrawMode::Fill,
            &[
                (WIDTH / 2.0, PLATFORM_Y + PLATFORM_HEIGHT / 2.0),
                (WIDTH / 2.0 + 46.0, PLATFORM_Y + 500.0),
                (WIDTH / 2.0 - 46.0, PLATFORM_Y + 500.0),
            ],
        );
        let smitten::Vec2 { x, y } = self.platform.position();
        graphics::push();
        graphics::translate((x, y));
        graphics::rotate(self.platform.angle());
        graphics::set_color(Color::rgb(0.75, 0.78, 0.86));
        graphics::rounded_rectangle(
            DrawMode::Fill,
            (-PLATFORM_WIDTH / 2.0, -PLATFORM_HEIGHT / 2.0),
            (PLATFORM_WIDTH, PLATFORM_HEIGHT),
            (5.0, 5.0),
        );
        graphics::pop();
        graphics::set_color(Color::rgb(0.45, 0.48, 0.58));
        graphics::circle(DrawMode::Fill, (x, y), 5.0);

        for block in self.blocks.values() {
            let smitten::Vec2 { x, y } = block.body.position();
            let color = KINDS[block.kind].color;
            let fill = color.lerp(Color::WHITE, block.flash * 0.7);
            graphics::push();
            graphics::translate((x, y));
            graphics::rotate(block.body.angle());
            Self::draw_shape(block.kind, fill, Color::rgba(0.0, 0.0, 0.0, 0.3));
            graphics::pop();
        }

        graphics::set_color(Color::WHITE);
        graphics::draw_particles(&self.dust, (0.0, 0.0));

        // The block waiting to be dropped, and a line showing where it
        // will fall.
        if let Some((kind, x, turns)) = self.next {
            let y = self.camera + DROPPER_Y;
            graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.12));
            graphics::set_line_width(1.0);
            graphics::line((x, y), (x, PLATFORM_Y + 200.0));
            graphics::push();
            graphics::translate((x, y));
            graphics::rotate(turns as f32 * FRAC_PI_2);
            let color = KINDS[kind].color;
            Self::draw_shape(
                kind,
                Color::rgba(color.r, color.g, color.b, 0.75),
                Color::rgba(1.0, 1.0, 1.0, 0.5),
            );
            graphics::pop();
        }
    }

    fn draw_hud(&self) {
        graphics::set_font(&self.small_font);
        graphics::set_color(Color::WHITE);
        graphics::print(
            &format!("Height {:.1} m", self.height / METER),
            (16.0, 12.0),
        );
        graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.6));
        graphics::print(
            &format!(
                "This run {:.1} m    Best {:.1} m",
                self.run_best / METER,
                self.best.max(self.run_best) / METER
            ),
            (16.0, 34.0),
        );

        // Hearts, with the lost ones faded.
        for heart in 0..HEARTS {
            let alpha = if heart < self.hearts { 1.0 } else { 0.18 };
            graphics::set_color(Color::rgba(1.0, 1.0, 1.0, alpha));
            graphics::draw_ex(
                &self.heart,
                WIDTH - 40.0 - heart as f32 * 36.0,
                12.0,
                0.0,
                2.0,
                2.0,
                0.0,
                0.0,
            );
        }

        // The block that comes after this one.
        if !self.over {
            graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.6));
            graphics::printf("next", (WIDTH - 130.0, 56.0), 114.0, Align::Right);
            graphics::push();
            graphics::translate((WIDTH - 60.0, 104.0));
            graphics::scale(0.5, 0.5);
            let color = KINDS[self.upcoming].color;
            Self::draw_shape(self.upcoming, color, Color::rgba(0.0, 0.0, 0.0, 0.3));
            graphics::pop();
        }

        if self.time < 6.0 && !self.over {
            let alpha = (6.0 - self.time).min(1.0) * 0.8;
            graphics::set_color(Color::rgba(1.0, 1.0, 1.0, alpha));
            graphics::printf(
                "Left and Right move.  Up turns.  Space drops.",
                (0.0, HEIGHT - 36.0),
                WIDTH,
                Align::Center,
            );
        }
        if let Some((text, seconds)) = &self.message {
            graphics::set_color(Color::rgba(1.0, 1.0, 1.0, seconds.min(1.0)));
            graphics::printf(text, (0.0, HEIGHT - 62.0), WIDTH, Align::Center);
        }

        if self.over {
            graphics::set_color(Color::rgba(0.0, 0.0, 0.0, 0.55));
            graphics::rectangle(DrawMode::Fill, (0.0, 0.0), (WIDTH, HEIGHT));
            graphics::set_color(Color::WHITE);
            graphics::set_font(&self.big_font);
            graphics::printf("Toppled!", (0.0, 190.0), WIDTH, Align::Center);
            graphics::set_font(&self.small_font);
            graphics::printf(
                &format!("You built {:.1} m.", self.run_best / METER),
                (0.0, 260.0),
                WIDTH,
                Align::Center,
            );
            if self.new_record {
                graphics::set_color(Color::rgb(1.0, 0.85, 0.3));
                graphics::printf("A new best!", (0.0, 288.0), WIDTH, Align::Center);
            }
            graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.7));
            graphics::printf(
                "Space to build again.  Escape to quit.",
                (0.0, 340.0),
                WIDTH,
                Align::Center,
            );
        }
    }
}

fn main() {
    smitten::run::<Topple>(Config {
        title: "Stack and Topple".to_string(),
        width: WIDTH as u32,
        height: HEIGHT as u32,
        identity: Some("smitten-topple".to_string()),
        ..Config::default()
    });
}
