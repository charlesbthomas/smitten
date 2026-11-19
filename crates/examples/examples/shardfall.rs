//! Shardfall, a twin-stick arena shooter.
//!
//! Waves of glowing shapes warp into an arena larger than the screen. Steer
//! with one stick, shoot with the other, and dash through trouble.
//!
//! - Gamepad: the left stick moves, the right stick aims and fires, and the
//!   right trigger fires the way you last aimed. A or either shoulder
//!   dashes. Start pauses and Back switches the screen effects.
//! - Keyboard and mouse: W, A, S and D move, the mouse aims, the left
//!   button fires, and Space or the right button dashes. P pauses and Tab
//!   switches the screen effects.
//! - Escape goes back to the title screen, and quits from there.
//!
//! What it shows:
//!
//! - Gamepad sticks with dead zones, triggers and rumble, falling back to
//!   the keyboard and mouse.
//! - A fixed-size bullet pool that never allocates while playing.
//! - Thousands of particles from a handful of `ParticleSystem`s.
//! - Screen shake driven by a "trauma" value and smooth noise.
//! - The whole arena drawn to a `Canvas`, then to the screen through a
//!   shader from `assets/shardfall-post.wgsl`: shockwaves, color split,
//!   bloom and a vignette.
//! - Every sound made in code, and played where it happens in the arena so
//!   it pans and fades as the camera moves.
//!
//! Run it with `cargo run --example shardfall`.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use smitten::audio::{self, Source};
use smitten::graphics::{
    Align, Canvas, DrawMode, EmissionArea, Font, Image, ParticleSystem, Shader,
};
use smitten::math::{self, RandomGenerator};
use smitten::sound::SoundData;
use smitten::{
    BlendMode, Color, Config, Game, GamepadAxis, GamepadButton, Joystick, Key, MouseButton,
    MouseClick, MouseMotion, Vec2, event, graphics, joystick, keyboard, mouse, timer,
};

const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 540.0;
const ARENA: Vec2 = Vec2::new(1800.0, 1200.0);

const PLAYER_RADIUS: f32 = 12.0;
const PLAYER_SPEED: f32 = 330.0;
/// How quickly the ship reaches the speed the stick asks for. Higher is
/// snappier.
const PLAYER_GRIP: f32 = 12.0;
const HULL: u32 = 3;
const INVULNERABLE_SECONDS: f32 = 1.6;

const DASH_SPEED: f32 = 1050.0;
const DASH_SECONDS: f32 = 0.16;
const DASH_COOLDOWN: f32 = 0.9;

const FIRE_INTERVAL: f32 = 0.075;
const BULLET_SPEED: f32 = 920.0;
const BULLET_SECONDS: f32 = 0.9;
const BULLET_RADIUS: f32 = 4.0;
const HOSTILE_SPEED: f32 = 290.0;
const HOSTILE_RADIUS: f32 = 6.0;
/// Every bullet, the player's and the enemies', comes from one pool of
/// this many.
const POOL_SIZE: usize = 600;

const MOVE_DEAD_ZONE: f32 = 0.2;
const AIM_DEAD_ZONE: f32 = 0.35;
const TRIGGER_THRESHOLD: f32 = 0.3;

/// The furthest the screen moves and turns at full trauma.
const MAX_SHAKE: f32 = 22.0;
const MAX_SHAKE_ANGLE: f32 = 0.04;
/// How much trauma drains each second.
const TRAUMA_DECAY: f32 = 1.4;

/// Sounds are at full volume within this many pixels of the listener, and
/// halve each time the distance doubles beyond it.
const HEARING_DISTANCE: f32 = 320.0;
/// How far above the arena the listener sits, so a sound right next to the
/// player is not panned hard to one side.
const LISTENER_HEIGHT: f32 = 240.0;

const BACKGROUND: Color = Color::rgb(0.02, 0.025, 0.05);
const GRID: Color = Color::rgba(0.25, 0.35, 0.7, 0.18);
const TEXT: Color = Color::rgb(0.9, 0.93, 1.0);
const DIM: Color = Color::rgba(0.9, 0.93, 1.0, 0.45);

/// The colors things come in. Each has its own debris particle system,
/// since a system's colors apply to all of its particles.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hue {
    Ship,
    Pink,
    Amber,
    Cyan,
}

impl Hue {
    const ALL: [Hue; 4] = [Hue::Ship, Hue::Pink, Hue::Amber, Hue::Cyan];

    fn color(self) -> Color {
        match self {
            Hue::Ship => Color::rgb(0.45, 0.85, 1.0),
            Hue::Pink => Color::rgb(1.0, 0.3, 0.65),
            Hue::Amber => Color::rgb(1.0, 0.65, 0.2),
            Hue::Cyan => Color::rgb(0.3, 1.0, 0.75),
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    Color::rgba(color.r, color.g, color.b, alpha)
}

// The bullet pool.

#[derive(Clone, Copy)]
struct Bullet {
    position: Vec2,
    velocity: Vec2,
    /// Seconds left before it fizzles out.
    life: f32,
    /// Fired by an enemy, so it hurts the player instead of enemies.
    hostile: bool,
    alive: bool,
}

impl Bullet {
    const DEAD: Bullet = Bullet {
        position: Vec2::ZERO,
        velocity: Vec2::ZERO,
        life: 0.0,
        hostile: false,
        alive: false,
    };

    fn radius(&self) -> f32 {
        if self.hostile {
            HOSTILE_RADIUS
        } else {
            BULLET_RADIUS
        }
    }
}

/// A fixed set of bullets, allocated once. Firing takes a free slot and a
/// bullet that dies gives its slot back, so a storm of bullets never
/// touches the allocator.
struct BulletPool {
    slots: Vec<Bullet>,
    /// The indices of the slots not in use.
    free: Vec<usize>,
}

impl BulletPool {
    fn new(capacity: usize) -> BulletPool {
        BulletPool {
            slots: vec![Bullet::DEAD; capacity],
            free: (0..capacity).rev().collect(),
        }
    }

    /// Fires a bullet, or does nothing if every slot is in use.
    fn fire(&mut self, position: Vec2, velocity: Vec2, life: f32, hostile: bool) -> bool {
        let Some(index) = self.free.pop() else {
            return false;
        };
        self.slots[index] = Bullet {
            position,
            velocity,
            life,
            hostile,
            alive: true,
        };
        true
    }

    fn release(&mut self, index: usize) {
        if self.slots[index].alive {
            self.slots[index].alive = false;
            self.free.push(index);
        }
    }

    fn release_all(&mut self) {
        for index in 0..self.slots.len() {
            self.release(index);
        }
    }

    fn live_count(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    fn capacity(&self) -> usize {
        self.slots.len()
    }
}

// Enemies.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Chases the player head on.
    Seeker,
    /// Slow and tough, and bursts into three shards.
    Splitter,
    /// A splitter's fragment: small, quick and fragile.
    Shard,
    /// Keeps its distance, circles, and fires bursts of slow orbs.
    Lancer,
}

impl Kind {
    fn hue(self) -> Hue {
        match self {
            Kind::Seeker => Hue::Pink,
            Kind::Splitter | Kind::Shard => Hue::Amber,
            Kind::Lancer => Hue::Cyan,
        }
    }

    fn radius(self) -> f32 {
        match self {
            Kind::Seeker => 13.0,
            Kind::Splitter => 24.0,
            Kind::Shard => 9.0,
            Kind::Lancer => 16.0,
        }
    }

    fn health(self) -> f32 {
        match self {
            Kind::Seeker => 2.0,
            Kind::Splitter => 9.0,
            Kind::Shard => 1.0,
            Kind::Lancer => 4.0,
        }
    }

    fn points(self) -> u32 {
        match self {
            Kind::Seeker => 100,
            Kind::Splitter => 250,
            Kind::Shard => 50,
            Kind::Lancer => 300,
        }
    }

    /// Big enough to send a shockwave across the screen when it dies.
    fn is_big(self) -> bool {
        matches!(self, Kind::Splitter | Kind::Lancer)
    }
}

struct Enemy {
    kind: Kind,
    position: Vec2,
    velocity: Vec2,
    health: f32,
    /// Turns the drawing, for the shapes that spin.
    spin: f32,
    /// Counts down to the lancer's next burst.
    cooldown: f32,
    /// Which way a lancer circles the player.
    orbit: f32,
    /// Flashes white for a moment after a hit.
    flash: f32,
    /// Seconds left warping in. Until then it cannot hurt or be hurt.
    warp: f32,
}

const WARP_SECONDS: f32 = 0.9;

impl Enemy {
    fn new(kind: Kind, position: Vec2, rng: &mut RandomGenerator) -> Enemy {
        Enemy {
            kind,
            position,
            velocity: Vec2::ZERO,
            health: kind.health(),
            spin: rng.random_range(0.0, TAU),
            cooldown: rng.random_range(1.5, 2.5),
            orbit: if rng.chance(0.5) { 1.0 } else { -1.0 },
            flash: 0.0,
            warp: WARP_SECONDS,
        }
    }

    fn is_active(&self) -> bool {
        self.warp <= 0.0
    }
}

/// The corners of a regular shape with `sides` sides.
fn regular(center: Vec2, radius: f32, sides: usize, angle: f32) -> Vec<Vec2> {
    (0..sides)
        .map(|i| center + Vec2::from_angle(angle + i as f32 * TAU / sides as f32) * radius)
        .collect()
}

/// A neon outline: a faint wide stroke under a bright thin one.
fn glow_outline(points: &[Vec2], color: Color, fill: f32) {
    graphics::set_color(with_alpha(color, fill));
    graphics::polygon(DrawMode::Fill, points);
    graphics::set_line_width(6.0);
    graphics::set_color(with_alpha(color, 0.25));
    graphics::polygon(DrawMode::Line, points);
    graphics::set_line_width(2.0);
    graphics::set_color(color);
    graphics::polygon(DrawMode::Line, points);
}

// The player.

struct Player {
    position: Vec2,
    velocity: Vec2,
    /// The way the guns point, always of length 1.
    aim: Vec2,
    hull: u32,
    invulnerable: f32,
    dash: f32,
    dash_direction: Vec2,
    dash_cooldown: f32,
    fire_cooldown: f32,
    /// The guns take turns, so the stream of bullets comes from both sides.
    left_gun: bool,
    alive: bool,
}

impl Player {
    fn new() -> Player {
        Player {
            position: ARENA / 2.0,
            velocity: Vec2::ZERO,
            aim: Vec2::new(0.0, -1.0),
            hull: HULL,
            invulnerable: 0.0,
            dash: 0.0,
            dash_direction: Vec2::ZERO,
            dash_cooldown: 0.0,
            fire_cooldown: 0.0,
            left_gun: false,
            alive: true,
        }
    }

    fn is_safe(&self) -> bool {
        self.invulnerable > 0.0 || self.dash > 0.0
    }
}

/// What the player is asking for this frame, from whichever device they
/// are using.
struct Controls {
    /// Never longer than 1.
    movement: Vec2,
    aim: Option<Vec2>,
    fire: bool,
}

// Effects that are not particles.

/// An expanding ring, left behind by explosions and warp-ins.
struct Ring {
    position: Vec2,
    age: f32,
    lifetime: f32,
    radius: f32,
    color: Color,
}

/// The "+300 x4" that floats up from a kill.
struct Popup {
    position: Vec2,
    text: String,
    age: f32,
}

const POPUP_SECONDS: f32 = 0.9;

struct Shockwave {
    position: Vec2,
    age: f32,
}

const SHOCK_SECONDS: f32 = 0.7;
const SHOCK_SPEED: f32 = 900.0;

/// One particle system per job. Bursts move the emitter, set its
/// direction and emit, so one system serves every explosion of its color.
struct Particles {
    /// White-hot specks that cool to orange: hits, impacts and explosions.
    sparks: ParticleSystem,
    /// Colored chunks, one system per [`Hue`].
    debris: Vec<ParticleSystem>,
    /// Slow dark puffs drawn under everything else.
    smoke: ParticleSystem,
    /// The ship's exhaust, emitted continuously from behind it.
    trail: ParticleSystem,
    /// Faint motes drifting around the arena.
    dust: ParticleSystem,
}

impl Particles {
    fn new(dot: &Image) -> Particles {
        let mut sparks = ParticleSystem::new(dot, 6000);
        sparks.set_particle_lifetime(0.2, 0.6);
        sparks.set_speed(150.0, 650.0);
        sparks.set_linear_damping(3.0, 6.0);
        sparks.set_sizes(&[0.4, 0.25, 0.05]);
        sparks.set_colors(&[
            Color::rgba(1.0, 1.0, 0.9, 1.0),
            Color::rgba(1.0, 0.75, 0.3, 0.8),
            Color::rgba(1.0, 0.3, 0.1, 0.0),
        ]);

        let debris = Hue::ALL
            .iter()
            .map(|hue| {
                let color = hue.color();
                let mut system = ParticleSystem::new(dot, 4000);
                system.set_particle_lifetime(0.5, 1.3);
                system.set_speed(60.0, 380.0);
                system.set_linear_damping(1.5, 3.0);
                system.set_spin(-6.0, 6.0);
                system.set_sizes(&[0.55, 0.3, 0.0]);
                system.set_size_variation(0.6);
                system.set_colors(&[
                    with_alpha(color, 1.0),
                    with_alpha(color, 0.7),
                    with_alpha(color, 0.0),
                ]);
                system
            })
            .collect();

        let mut smoke = ParticleSystem::new(dot, 600);
        smoke.set_particle_lifetime(0.8, 1.8);
        smoke.set_speed(20.0, 90.0);
        smoke.set_linear_damping(1.0, 2.0);
        smoke.set_sizes(&[1.0, 2.5, 3.5]);
        smoke.set_size_variation(0.5);
        smoke.set_colors(&[
            Color::rgba(0.3, 0.3, 0.45, 0.0),
            Color::rgba(0.2, 0.2, 0.3, 0.35),
            Color::rgba(0.1, 0.1, 0.2, 0.0),
        ]);

        let ship = Hue::Ship.color();
        let mut trail = ParticleSystem::new(dot, 800);
        trail.set_particle_lifetime(0.25, 0.45);
        trail.set_speed(40.0, 120.0);
        trail.set_spread(0.5);
        trail.set_linear_damping(2.0, 4.0);
        trail.set_sizes(&[0.5, 0.2]);
        trail.set_colors(&[with_alpha(ship, 0.8), with_alpha(ship, 0.0)]);

        let mut dust = ParticleSystem::new(dot, 500);
        dust.set_position(ARENA / 2.0);
        dust.set_emission_area(EmissionArea::uniform(ARENA / 2.0));
        dust.set_emission_rate(90.0);
        dust.set_particle_lifetime(3.0, 6.0);
        dust.set_spread(TAU);
        dust.set_speed(5.0, 25.0);
        dust.set_sizes(&[0.1, 0.25, 0.1]);
        dust.set_colors(&[
            Color::rgba(0.5, 0.6, 1.0, 0.0),
            Color::rgba(0.5, 0.6, 1.0, 0.35),
            Color::rgba(0.5, 0.6, 1.0, 0.0),
        ]);
        // Fill the arena before the first frame, rather than letting the
        // dust fade in.
        for _ in 0..120 {
            dust.update(0.05);
        }

        Particles {
            sparks,
            debris,
            smoke,
            trail,
            dust,
        }
    }

    fn update(&mut self, dt: f32) {
        self.sparks.update(dt);
        for system in &mut self.debris {
            system.update(dt);
        }
        self.smoke.update(dt);
        self.trail.update(dt);
        self.dust.update(dt);
    }

    fn count(&self) -> usize {
        self.sparks.count()
            + self.debris.iter().map(ParticleSystem::count).sum::<usize>()
            + self.smoke.count()
            + self.trail.count()
            + self.dust.count()
    }

    /// Throws `count` sparks out from `position`, within `spread` radians
    /// either side of `direction`.
    fn sparks(&mut self, position: Vec2, direction: f32, spread: f32, count: usize) {
        self.sparks.set_position(position);
        self.sparks.set_direction(direction);
        self.sparks.set_spread(spread);
        self.sparks.emit(count);
    }

    fn debris(&mut self, hue: Hue, position: Vec2, count: usize) {
        let system = &mut self.debris[hue.index()];
        system.set_position(position);
        system.set_spread(TAU);
        system.emit(count);
    }

    fn smoke(&mut self, position: Vec2, count: usize) {
        self.smoke.set_position(position);
        self.smoke.set_spread(TAU);
        self.smoke.emit(count);
    }
}

/// A soft white dot, bright in the middle and fading to nothing at the
/// edge. Particles are tinted, so one picture serves every effect.
fn soft_dot(size: u32) -> Image {
    let center = (size as f32 - 1.0) / 2.0;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let distance = (x as f32 - center).hypot(y as f32 - center) / center;
            let alpha = (1.0 - distance).clamp(0.0, 1.0).powf(1.5);
            pixels.extend_from_slice(&[255, 255, 255, (alpha * 255.0) as u8]);
        }
    }
    Image::from_rgba(size, size, &pixels).expect("the dot is a valid image")
}

// Sound, all of it synthesized when the game loads.

const RATE: u32 = 44_100;

/// Makes `seconds` of mono sound from `wave`, which is given the time in
/// seconds and how far through the sound it is, from 0.0 to 1.0.
fn synth(seconds: f32, mut wave: impl FnMut(f32, f32) -> f32) -> SoundData {
    let count = (seconds * RATE as f32) as usize;
    let mut data = SoundData::new(count, RATE, 1);
    for i in 0..count {
        let t = i as f32 / RATE as f32;
        data.set_sample(i, 0, wave(t, i as f32 / count as f32).clamp(-1.0, 1.0));
    }
    data
}

/// A sine wave whose pitch slides, kept in step by tracking its phase.
struct Sweep {
    phase: f32,
}

impl Sweep {
    fn next(&mut self, frequency: f32) -> f32 {
        self.phase = (self.phase + frequency / RATE as f32).fract();
        (self.phase * TAU).sin()
    }
}

/// White noise passed through a one-pole low-pass filter. Lower `cutoff`
/// is duller.
struct Rumble {
    rng: RandomGenerator,
    last: f32,
}

impl Rumble {
    fn new(seed: u64) -> Rumble {
        Rumble {
            rng: RandomGenerator::new(seed),
            last: 0.0,
        }
    }

    fn next(&mut self, cutoff: f32) -> f32 {
        let white = self.rng.random_range(-1.0, 1.0);
        self.last += (white - self.last) * cutoff;
        self.last
    }
}

struct Sounds {
    shot: Source,
    hostile_shot: Source,
    hit: Source,
    pop: Source,
    boom: Source,
    warp: Source,
    dash: Source,
    hurt: Source,
}

impl Sounds {
    fn new() -> Sounds {
        let mut sweep = Sweep { phase: 0.0 };
        let shot = synth(0.08, |t, _| {
            let tone = sweep.next(1500.0 - t * 12_000.0);
            // Push the sine toward a square for some bite.
            (tone * 3.0).tanh() * (-t * 45.0).exp() * 0.35
        });

        let mut sweep = Sweep { phase: 0.0 };
        let hostile_shot = synth(0.22, |t, _| {
            sweep.next(420.0 + t * 1800.0) * (-t * 14.0).exp() * 0.4
        });

        let mut noise = Rumble::new(7);
        let hit = synth(0.05, |t, _| noise.next(0.7) * (-t * 90.0).exp() * 0.5);

        let mut noise = Rumble::new(11);
        let mut sweep = Sweep { phase: 0.0 };
        let pop = synth(0.35, |t, _| {
            let body = sweep.next(240.0 - t * 400.0) * 0.5;
            (noise.next(0.35) + body) * (-t * 11.0).exp() * 0.7
        });

        let mut noise = Rumble::new(13);
        let mut sweep = Sweep { phase: 0.0 };
        let boom = synth(1.2, |t, progress| {
            // A dull roar with a low thump under it, and the filter closing
            // as it fades.
            let roar = noise.next(0.25 - progress * 0.2);
            let thump = sweep.next(70.0 - t * 30.0) * (-t * 6.0).exp();
            (roar * 1.6 + thump) * (-t * 3.2).exp() * 0.8
        });

        let mut sweep = Sweep { phase: 0.0 };
        let warp = synth(0.7, |t, progress| {
            let wobble = 1.0 + (t * 40.0).sin() * 0.04;
            sweep.next((180.0 + progress * progress * 900.0) * wobble)
                * (progress * PI).sin()
                * 0.25
        });

        let mut noise = Rumble::new(17);
        let dash = synth(0.28, |_, progress| {
            noise.next(0.05 + progress * 0.4) * 2.2 * (progress * PI).sin()
        });

        let mut sweep = Sweep { phase: 0.0 };
        let mut noise = Rumble::new(19);
        let hurt = synth(0.5, |t, _| {
            let buzz = (sweep.next(140.0 - t * 140.0) * 4.0).tanh();
            (buzz * 0.6 + noise.next(0.3)) * (-t * 6.0).exp() * 0.6
        });

        let source = |data: &SoundData| {
            let source = Source::from_sound_data(data);
            source.set_attenuation_distances(HEARING_DISTANCE, f32::MAX);
            source
        };
        Sounds {
            shot: source(&shot),
            hostile_shot: source(&hostile_shot),
            hit: source(&hit),
            pop: source(&pop),
            boom: source(&boom),
            warp: source(&warp),
            dash: source(&dash),
            hurt: source(&hurt),
        }
    }
}

/// Plays a copy of `sound` at a point in the arena. Each copy keeps the
/// position the source had when it started, so overlapping explosions in
/// different places each sound from their own spot.
fn play_at(sound: &Source, position: Vec2, volume: f32, pitch: f32) {
    sound.set_position((position.x, position.y, 0.0));
    sound.play_once_with(volume, pitch);
}

// The game.

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Title,
    Playing,
    Paused,
    GameOver,
}

struct Shardfall {
    state: State,
    rng: RandomGenerator,
    player: Player,
    enemies: Vec<Enemy>,
    bullets: BulletPool,
    /// Enemies still to warp in this wave, with the seconds until each does.
    arrivals: Vec<(f32, Kind, Vec2)>,
    wave: u32,
    /// Seconds until the next wave, once the arena is clear.
    wave_break: f32,
    /// Seconds the "WAVE n" banner has been up.
    banner: f32,

    score: u32,
    best: u32,
    multiplier: u32,
    /// Seconds left before the multiplier drops back to 1.
    combo: f32,

    camera: Vec2,
    /// 0.0 to 1.0. Hits and explosions add to it, and it drains away. The
    /// screen shakes by its square, so small knocks barely register.
    trauma: f32,
    /// Slows the whole game for a moment when the player is hit.
    time_scale: f32,

    particles: Particles,
    rings: Vec<Ring>,
    popups: Vec<Popup>,
    shockwave: Option<Shockwave>,

    sounds: Sounds,

    /// The gamepad being played with, for rumble and for showing which
    /// controls are in use.
    pad: Option<Joystick>,
    using_pad: bool,

    scene: Canvas,
    post: Shader,
    effects_on: bool,

    hud_font: Font,
    big_font: Font,
    title_font: Font,
}

impl Shardfall {
    fn start(&mut self) {
        self.player = Player::new();
        self.player.invulnerable = INVULNERABLE_SECONDS;
        self.enemies.clear();
        self.bullets.release_all();
        self.arrivals.clear();
        self.wave = 0;
        self.wave_break = 0.8;
        self.score = 0;
        self.multiplier = 1;
        self.combo = 0.0;
        self.time_scale = 1.0;
        self.state = State::Playing;
    }

    fn back(&mut self) {
        match self.state {
            State::Title => event::quit(),
            _ => {
                self.best = self.best.max(self.score);
                self.state = State::Title;
                self.enemies.clear();
                self.bullets.release_all();
                self.arrivals.clear();
            }
        }
    }

    fn confirm(&mut self) {
        match self.state {
            State::Title | State::GameOver => self.start(),
            State::Paused => self.state = State::Playing,
            State::Playing => {}
        }
    }

    fn toggle_pause(&mut self) {
        self.state = match self.state {
            State::Playing => State::Paused,
            State::Paused => State::Playing,
            other => other,
        };
    }

    fn add_trauma(&mut self, amount: f32) {
        self.trauma = (self.trauma + amount).min(1.0);
    }

    fn rumble(&self, heavy: f32, light: f32, seconds: f32) {
        if self.using_pad
            && let Some(pad) = self.pad
        {
            pad.set_vibration(heavy, light, seconds);
        }
    }

    /// Where the mouse points in the arena.
    fn mouse_in_arena(&self) -> Vec2 {
        mouse::position() - Vec2::new(WIDTH, HEIGHT) / 2.0 + self.camera
    }

    fn controls(&mut self) -> Controls {
        let mut movement = keyboard::direction();
        let mut aim = None;
        let mut fire = false;

        if let Some(pad) = self.pad.filter(Joystick::is_connected) {
            let stick = pad.left_stick(MOVE_DEAD_ZONE);
            let look = pad.right_stick(AIM_DEAD_ZONE);
            let trigger = pad.gamepad_axis(GamepadAxis::TriggerRight) > TRIGGER_THRESHOLD;
            if stick != Vec2::ZERO || look != Vec2::ZERO || trigger {
                self.using_pad = true;
            }
            movement += stick;
            if look != Vec2::ZERO {
                aim = Some(look);
                fire = true;
            }
            fire |= trigger;
        }

        if !self.using_pad {
            let toward = self.mouse_in_arena() - self.player.position;
            if toward.length() > 1.0 {
                aim = Some(toward);
            }
            fire = mouse::is_down(MouseButton::Left);
        }

        Controls {
            movement: movement.clamp_length_max(1.0),
            aim: aim.map(Vec2::normalize_or_zero),
            fire,
        }
    }

    fn dash(&mut self) {
        let player = &mut self.player;
        if self.state != State::Playing || !player.alive || player.dash_cooldown > 0.0 {
            return;
        }
        let movement = self.controls().movement;
        let player = &mut self.player;
        player.dash_direction = if movement.length() > 0.1 {
            movement.normalize()
        } else {
            player.aim
        };
        player.dash = DASH_SECONDS;
        player.dash_cooldown = DASH_COOLDOWN;
        let (position, direction) = (player.position, player.dash_direction);
        self.particles.trail.set_position(position);
        self.particles.trail.set_direction((-direction).to_angle());
        self.particles.trail.emit(40);
        play_at(&self.sounds.dash, position, 0.8, 1.0);
        self.rumble(0.0, 0.4, 0.1);
    }

    fn update_player(&mut self, dt: f32) {
        let controls = self.controls();
        let player = &mut self.player;
        if !player.alive {
            return;
        }
        player.invulnerable = (player.invulnerable - dt).max(0.0);
        player.dash_cooldown = (player.dash_cooldown - dt).max(0.0);
        player.fire_cooldown -= dt;

        if player.dash > 0.0 {
            player.dash -= dt;
            player.velocity = player.dash_direction * DASH_SPEED;
        } else {
            let wanted = controls.movement * PLAYER_SPEED;
            player.velocity += (wanted - player.velocity) * (PLAYER_GRIP * dt).min(1.0);
        }
        player.position += player.velocity * dt;
        let min = Vec2::splat(PLAYER_RADIUS);
        player.position = player.position.clamp(min, ARENA - min);

        if let Some(aim) = controls.aim
            && aim != Vec2::ZERO
        {
            player.aim = aim;
        }

        // Exhaust, heavier the faster the ship goes.
        let speed = player.velocity.length();
        let behind = player.position - player.aim * 10.0;
        let trail = &mut self.particles.trail;
        trail.move_to(behind);
        trail.set_direction((-player.velocity).to_angle());
        trail.set_emission_rate(if speed > 20.0 { speed * 0.4 } else { 0.0 });

        if controls.fire && player.fire_cooldown <= 0.0 {
            player.fire_cooldown += FIRE_INTERVAL;
            player.fire_cooldown = player.fire_cooldown.max(0.0);
            player.left_gun = !player.left_gun;
            let side = if player.left_gun { 1.0 } else { -1.0 };
            let muzzle = player.position + player.aim * 14.0 + player.aim.perp() * side * 6.0;
            let wobble = self.rng.random_range(-0.04, 0.04);
            let direction = Vec2::from_angle(player.aim.to_angle() + wobble);
            // Bullets inherit a little of the ship's speed, so strafing
            // feels right.
            let velocity = direction * BULLET_SPEED + player.velocity * 0.3;
            let aim = player.aim;
            if self.bullets.fire(muzzle, velocity, BULLET_SECONDS, false) {
                self.particles.sparks(muzzle, aim.to_angle(), 0.35, 5);
                let pitch = self.rng.random_range(0.92, 1.08);
                play_at(&self.sounds.shot, muzzle, 0.35, pitch);
            }
        }
    }

    fn update_bullets(&mut self, dt: f32) {
        for index in 0..self.bullets.capacity() {
            let bullet = &mut self.bullets.slots[index];
            if !bullet.alive {
                continue;
            }
            bullet.position += bullet.velocity * dt;
            bullet.life -= dt;
            let position = bullet.position;
            let outside = position.x < 0.0
                || position.y < 0.0
                || position.x > ARENA.x
                || position.y > ARENA.y;
            if outside {
                // Sparks bounce back into the arena off the wall.
                let normal = if position.x < 0.0 {
                    Vec2::X
                } else if position.x > ARENA.x {
                    -Vec2::X
                } else if position.y < 0.0 {
                    Vec2::Y
                } else {
                    -Vec2::Y
                };
                let at = position.clamp(Vec2::ZERO, ARENA);
                self.particles.sparks(at, normal.to_angle(), FRAC_PI_2, 8);
                self.bullets.release(index);
            } else if bullet.life <= 0.0 {
                self.bullets.release(index);
            }
        }
    }

    fn update_enemies(&mut self, dt: f32) {
        let target = self.player.position;
        let chasing = self.player.alive;
        let mut volleys = Vec::new();

        for enemy in &mut self.enemies {
            enemy.flash = (enemy.flash - dt * 6.0).max(0.0);
            if enemy.warp > 0.0 {
                enemy.warp -= dt;
                enemy.spin += dt * 8.0;
                continue;
            }
            let to_player = target - enemy.position;
            let distance = to_player.length();
            let toward = to_player.normalize_or_zero();
            let (wanted, grip) = if !chasing {
                (Vec2::from_angle(enemy.spin) * 60.0, 1.0)
            } else {
                match enemy.kind {
                    Kind::Seeker => (toward * 175.0, 2.5),
                    Kind::Shard => (toward * 245.0, 4.0),
                    Kind::Splitter => (toward * 80.0, 1.0),
                    Kind::Lancer => {
                        let closer = if distance > 380.0 {
                            1.0
                        } else if distance < 260.0 {
                            -1.0
                        } else {
                            0.0
                        };
                        let around = toward.perp() * enemy.orbit;
                        (
                            (toward * closer + around * 0.8).normalize_or_zero() * 140.0,
                            2.0,
                        )
                    }
                }
            };
            enemy.velocity += (wanted - enemy.velocity) * (grip * dt).min(1.0);
            enemy.position += enemy.velocity * dt;

            enemy.spin += dt
                * match enemy.kind {
                    Kind::Splitter => 0.8,
                    Kind::Shard => 7.0,
                    Kind::Lancer => 1.5,
                    Kind::Seeker => 0.0,
                };
            if enemy.kind == Kind::Lancer && chasing {
                enemy.cooldown -= dt;
                if enemy.cooldown <= 0.0 {
                    enemy.cooldown = self.rng.random_range(2.2, 3.0);
                    if distance < 700.0 {
                        volleys.push((enemy.position, toward));
                    }
                }
            }
        }

        // Push overlapping enemies apart, so a crowd spreads into a swarm
        // rather than one blob.
        for i in 0..self.enemies.len() {
            for j in i + 1..self.enemies.len() {
                let (a, b) = (&self.enemies[i], &self.enemies[j]);
                if !a.is_active() || !b.is_active() {
                    continue;
                }
                let apart = b.position - a.position;
                let overlap = a.kind.radius() + b.kind.radius() - apart.length();
                if overlap > 0.0 {
                    let push = apart.normalize_or(Vec2::X) * overlap / 2.0;
                    self.enemies[i].position -= push;
                    self.enemies[j].position += push;
                }
            }
        }
        for enemy in &mut self.enemies {
            let min = Vec2::splat(enemy.kind.radius());
            enemy.position = enemy.position.clamp(min, ARENA - min);
        }

        for (position, toward) in volleys {
            for spread in [-0.2, 0.0, 0.2] {
                let direction = Vec2::from_angle(toward.to_angle() + spread);
                self.bullets.fire(
                    position + direction * 18.0,
                    direction * HOSTILE_SPEED,
                    4.0,
                    true,
                );
            }
            self.particles.sparks(position, toward.to_angle(), 0.4, 10);
            play_at(&self.sounds.hostile_shot, position, 0.8, 1.0);
        }
    }

    fn collide(&mut self) {
        // The player's bullets against enemies.
        for index in 0..self.bullets.capacity() {
            let bullet = self.bullets.slots[index];
            if !bullet.alive || bullet.hostile {
                continue;
            }
            let struck = self.enemies.iter().position(|enemy| {
                enemy.is_active()
                    && enemy.position.distance(bullet.position)
                        < enemy.kind.radius() + bullet.radius()
            });
            let Some(struck) = struck else {
                continue;
            };
            self.bullets.release(index);
            let enemy = &mut self.enemies[struck];
            enemy.health -= 1.0;
            enemy.flash = 1.0;
            enemy.velocity += bullet.velocity.normalize_or_zero() * 40.0;
            let back = (-bullet.velocity).to_angle();
            self.particles.sparks(bullet.position, back, 0.7, 10);
            let pitch = self.rng.random_range(0.9, 1.2);
            play_at(&self.sounds.hit, bullet.position, 0.5, pitch);
            if self.enemies[struck].health <= 0.0 {
                self.kill(struck);
            }
        }

        if !self.player.alive || self.player.is_safe() {
            return;
        }
        let position = self.player.position;

        // Enemy orbs against the player.
        let mut struck = false;
        for index in 0..self.bullets.capacity() {
            let bullet = self.bullets.slots[index];
            if bullet.alive
                && bullet.hostile
                && bullet.position.distance(position) < PLAYER_RADIUS + bullet.radius()
            {
                self.bullets.release(index);
                struck = true;
            }
        }
        // And enemies ramming the player.
        struck |= self.enemies.iter().any(|enemy| {
            enemy.is_active()
                && enemy.position.distance(position) < enemy.kind.radius() + PLAYER_RADIUS - 2.0
        });
        if struck {
            self.hurt_player();
        }
    }

    fn kill(&mut self, index: usize) {
        let enemy = self.enemies.swap_remove(index);
        let hue = enemy.kind.hue();
        let position = enemy.position;
        let big = enemy.kind.is_big();

        self.multiplier = (self.multiplier + 1).min(16);
        self.combo = 2.0;
        let points = enemy.kind.points() * self.multiplier;
        self.score += points;
        self.popups.push(Popup {
            position,
            text: format!("+{points}"),
            age: 0.0,
        });

        let size = if big { 2.0 } else { 1.0 };
        self.particles
            .sparks(position, 0.0, TAU, (90.0 * size) as usize);
        self.particles
            .debris(hue, position, (140.0 * size) as usize);
        self.particles.smoke(position, (8.0 * size) as usize);
        self.rings.push(Ring {
            position,
            age: 0.0,
            lifetime: 0.35 * size,
            radius: 60.0 * size,
            color: hue.color(),
        });
        self.add_trauma(if big { 0.45 } else { 0.18 });
        if big {
            self.shockwave = Some(Shockwave { position, age: 0.0 });
            play_at(
                &self.sounds.boom,
                position,
                1.0,
                self.rng.random_range(0.9, 1.1),
            );
            self.rumble(0.7, 0.5, 0.25);
        } else {
            play_at(
                &self.sounds.pop,
                position,
                0.8,
                self.rng.random_range(0.85, 1.25),
            );
            self.rumble(0.15, 0.35, 0.08);
        }

        if enemy.kind == Kind::Splitter {
            for i in 0..3 {
                let direction = Vec2::from_angle(enemy.spin + i as f32 * TAU / 3.0);
                let mut shard = Enemy::new(Kind::Shard, position + direction * 14.0, &mut self.rng);
                shard.warp = 0.0;
                shard.velocity = direction * 320.0;
                self.enemies.push(shard);
            }
        }
    }

    fn hurt_player(&mut self) {
        let player = &mut self.player;
        player.hull = player.hull.saturating_sub(1);
        player.invulnerable = INVULNERABLE_SECONDS;
        let position = player.position;
        self.multiplier = 1;
        self.combo = 0.0;
        self.time_scale = 0.15;
        self.add_trauma(0.7);
        self.rumble(1.0, 1.0, 0.4);
        self.particles.sparks(position, 0.0, TAU, 60);
        self.particles.debris(Hue::Ship, position, 50);
        play_at(&self.sounds.hurt, position, 1.0, 1.0);
        self.shockwave = Some(Shockwave { position, age: 0.0 });

        // Clear the orbs close by, so one mistake does not cost two hull.
        for index in 0..self.bullets.capacity() {
            let bullet = self.bullets.slots[index];
            if bullet.alive && bullet.hostile && bullet.position.distance(position) < 250.0 {
                self.particles.sparks(bullet.position, 0.0, TAU, 4);
                self.bullets.release(index);
            }
        }

        if self.player.hull == 0 {
            self.player.alive = false;
            self.particles.trail.set_emission_rate(0.0);
            self.particles.sparks(position, 0.0, TAU, 200);
            self.particles.debris(Hue::Ship, position, 250);
            self.particles.smoke(position, 30);
            self.rings.push(Ring {
                position,
                age: 0.0,
                lifetime: 0.9,
                radius: 220.0,
                color: Hue::Ship.color(),
            });
            self.add_trauma(1.0);
            play_at(&self.sounds.boom, position, 1.0, 0.7);
            self.best = self.best.max(self.score);
            self.state = State::GameOver;
        }
    }

    fn update_waves(&mut self, dt: f32) {
        for arrival in &mut self.arrivals {
            arrival.0 -= dt;
        }
        let mut index = 0;
        while index < self.arrivals.len() {
            if self.arrivals[index].0 > 0.0 {
                index += 1;
                continue;
            }
            let (_, kind, position) = self.arrivals.swap_remove(index);
            self.enemies.push(Enemy::new(kind, position, &mut self.rng));
            self.rings.push(Ring {
                position,
                age: 0.0,
                lifetime: WARP_SECONDS,
                radius: 70.0,
                color: kind.hue().color(),
            });
            play_at(
                &self.sounds.warp,
                position,
                0.6,
                self.rng.random_range(0.9, 1.1),
            );
        }

        if self.enemies.is_empty() && self.arrivals.is_empty() {
            self.wave_break -= dt;
            if self.wave_break <= 0.0 {
                self.next_wave();
            }
        }
    }

    /// Plans the next wave: more enemies each time, arriving in clusters
    /// from rifts away from the player.
    fn next_wave(&mut self) {
        self.wave += 1;
        self.wave_break = 2.0;
        self.banner = 0.0;
        let wave = self.wave as f32;
        let count = 5 + self.wave * 3;
        let mut delay = 0.0;
        let mut rift = self.rift_position();
        for i in 0..count {
            if i % 4 == 0 {
                rift = self.rift_position();
                delay += self.rng.random_range(0.6, 1.4);
            }
            let roll = self.rng.random();
            let kind = if self.wave >= 3 && roll < 0.12 + wave * 0.01 {
                Kind::Lancer
            } else if self.wave >= 2 && roll < 0.35 {
                Kind::Splitter
            } else {
                Kind::Seeker
            };
            let offset = Vec2::from_angle(self.rng.random_range(0.0, TAU))
                * self.rng.random_range(0.0, 70.0);
            self.arrivals
                .push((delay + i as f32 * 0.12, kind, rift + offset));
        }
    }

    /// Somewhere in the arena, but not too close to the player.
    fn rift_position(&mut self) -> Vec2 {
        let margin = 80.0;
        loop {
            let position = Vec2::new(
                self.rng.random_range(margin, ARENA.x - margin),
                self.rng.random_range(margin, ARENA.y - margin),
            );
            if position.distance(self.player.position) > 420.0 {
                return position;
            }
        }
    }

    fn update_effects(&mut self, dt: f32) {
        self.particles.update(dt);
        for ring in &mut self.rings {
            ring.age += dt;
        }
        self.rings.retain(|ring| ring.age < ring.lifetime);
        for popup in &mut self.popups {
            popup.age += dt;
            popup.position.y -= 40.0 * dt;
        }
        self.popups.retain(|popup| popup.age < POPUP_SECONDS);
        if let Some(shock) = &mut self.shockwave {
            shock.age += dt;
            if shock.age > SHOCK_SECONDS {
                self.shockwave = None;
            }
        }
        self.banner += dt;
        self.combo -= dt;
        if self.combo <= 0.0 {
            self.multiplier = 1;
        }
    }

    /// Follows the player, leaning toward where they aim, and keeps the
    /// listener with the camera.
    fn update_camera(&mut self, dt: f32) {
        let goal = if self.state == State::Title {
            ARENA / 2.0 + Vec2::new((timer::time() * 0.2).sin() as f32, 0.0) * 300.0
        } else {
            self.player.position + self.player.aim * 70.0
        };
        self.camera += (goal - self.camera) * (5.0 * dt).min(1.0);
        let half = Vec2::new(WIDTH, HEIGHT) / 2.0;
        let slack = Vec2::splat(80.0);
        self.camera = self.camera.clamp(half - slack, ARENA - half + slack);
        audio::set_position((self.camera.x, self.camera.y, LISTENER_HEIGHT));
    }

    /// How far the screen is shaken this frame, from smooth noise so it
    /// rolls rather than jitters.
    fn shake(&self) -> (Vec2, f32) {
        let amount = self.trauma * self.trauma;
        let t = timer::time() as f32 * 18.0;
        let wobble = |seed: f32| math::noise2(t, seed) * 2.0 - 1.0;
        (
            Vec2::new(wobble(1.0), wobble(2.0)) * MAX_SHAKE * amount,
            wobble(3.0) * MAX_SHAKE_ANGLE * amount,
        )
    }

    // Drawing.

    fn draw_arena(&self) {
        graphics::set_line_width(1.0);
        graphics::set_color(GRID);
        let step = 75.0;
        let mut x = step;
        while x < ARENA.x {
            graphics::line((x, 0.0), (x, ARENA.y));
            x += step;
        }
        let mut y = step;
        while y < ARENA.y {
            graphics::line((0.0, y), (ARENA.x, y));
            y += step;
        }

        let edge = Hue::Ship.color();
        for (width, alpha) in [(14.0, 0.08), (6.0, 0.25), (2.0, 0.9)] {
            graphics::set_line_width(width);
            graphics::set_color(with_alpha(edge, alpha));
            graphics::rectangle(DrawMode::Line, (0.0, 0.0), ARENA);
        }
    }

    fn draw_enemy(&self, enemy: &Enemy) {
        let color = enemy.kind.hue().color();
        let r = enemy.kind.radius();
        let at = enemy.position;

        if !enemy.is_active() {
            // Warping in: a spinning outline that closes on its final size.
            let progress = 1.0 - enemy.warp / WARP_SECONDS;
            let points = regular(at, r * (3.0 - progress * 2.0), 4, enemy.spin);
            graphics::set_line_width(1.5);
            graphics::set_color(with_alpha(color, progress * 0.8));
            graphics::polygon(DrawMode::Line, &points);
            return;
        }

        let color = if enemy.flash > 0.0 {
            Color::rgb(
                color.r + (1.0 - color.r) * enemy.flash,
                color.g + (1.0 - color.g) * enemy.flash,
                color.b + (1.0 - color.b) * enemy.flash,
            )
        } else {
            color
        };
        match enemy.kind {
            Kind::Seeker => {
                let heading = enemy.velocity.to_angle();
                let points = [
                    at + Vec2::from_angle(heading) * r * 1.3,
                    at + Vec2::from_angle(heading + 2.5) * r,
                    at + Vec2::from_angle(heading + PI) * r * 0.35,
                    at + Vec2::from_angle(heading - 2.5) * r,
                ];
                glow_outline(&points, color, 0.25);
            }
            Kind::Splitter => {
                glow_outline(&regular(at, r, 4, enemy.spin), color, 0.2);
                glow_outline(&regular(at, r * 0.45, 4, -enemy.spin * 2.0), color, 0.4);
            }
            Kind::Shard => {
                glow_outline(&regular(at, r, 4, enemy.spin), color, 0.4);
            }
            Kind::Lancer => {
                glow_outline(&regular(at, r, 6, enemy.spin), color, 0.2);
                // The core brightens as the next burst charges.
                let charge = (1.0 - enemy.cooldown / 0.6).clamp(0.0, 1.0);
                graphics::set_color(with_alpha(color, 0.3 + charge * 0.7));
                graphics::circle(DrawMode::Fill, at, 4.0 + charge * 5.0);
                let toward = (self.player.position - at).normalize_or_zero();
                graphics::set_line_width(2.0);
                graphics::set_color(color);
                graphics::line(at + toward * r * 0.6, at + toward * r * 1.5);
            }
        }
    }

    fn draw_player(&self) {
        let player = &self.player;
        if !player.alive {
            return;
        }
        // Blink while invulnerable.
        if player.invulnerable > 0.0 && (timer::time() * 16.0) as i64 % 2 == 0 {
            return;
        }
        let angle = player.aim.to_angle();
        let at = player.position;
        let ship = |at: Vec2| {
            [
                at + Vec2::from_angle(angle) * 20.0,
                at + Vec2::from_angle(angle + 2.55) * 12.0,
                at + Vec2::from_angle(angle + PI) * 5.0,
                at + Vec2::from_angle(angle - 2.55) * 12.0,
            ]
        };
        let color = Hue::Ship.color();
        if player.dash > 0.0 {
            // Afterimages along the dash.
            for i in 1..4 {
                let back = at - player.dash_direction * i as f32 * 22.0;
                graphics::set_color(with_alpha(color, 0.35 / i as f32));
                graphics::polygon(DrawMode::Fill, &ship(back));
            }
        }
        glow_outline(&ship(at), color, 0.35);
    }

    fn draw_bullets(&self) {
        for bullet in self.bullets.slots.iter().filter(|bullet| bullet.alive) {
            if bullet.hostile {
                let color = Hue::Cyan.color();
                graphics::set_color(with_alpha(color, 0.25));
                graphics::circle(DrawMode::Fill, bullet.position, HOSTILE_RADIUS * 2.0);
                graphics::set_color(color);
                graphics::circle(DrawMode::Fill, bullet.position, HOSTILE_RADIUS);
                graphics::set_color(Color::WHITE);
                graphics::circle(DrawMode::Fill, bullet.position, HOSTILE_RADIUS * 0.4);
            } else {
                // A streak along the bullet's path.
                let tail = bullet.position - bullet.velocity * 0.025;
                graphics::set_line_width(6.0);
                graphics::set_color(Color::rgba(1.0, 0.8, 0.4, 0.3));
                graphics::line(tail, bullet.position);
                graphics::set_line_width(2.5);
                graphics::set_color(Color::rgb(1.0, 0.95, 0.75));
                graphics::line(tail, bullet.position);
            }
        }
    }

    fn draw_world(&self) {
        graphics::clear(BACKGROUND);
        let (offset, angle) = self.shake();
        graphics::scoped(|| {
            graphics::translate((WIDTH / 2.0, HEIGHT / 2.0));
            graphics::rotate(angle);
            graphics::translate(offset - self.camera);

            graphics::set_color(Color::WHITE);
            graphics::draw(&self.particles.smoke, (0.0, 0.0));
            self.draw_arena();

            // Everything that glows adds its light to what is under it.
            graphics::set_blend_mode(BlendMode::Add);
            graphics::set_color(Color::WHITE);
            graphics::draw(&self.particles.dust, (0.0, 0.0));
            for ring in &self.rings {
                let progress = ring.age / ring.lifetime;
                let radius = ring.radius * (1.0 - (1.0 - progress).powi(3));
                graphics::set_line_width(1.0 + 5.0 * (1.0 - progress));
                graphics::set_color(with_alpha(ring.color, 1.0 - progress));
                graphics::circle(DrawMode::Line, ring.position, radius);
            }
            for enemy in &self.enemies {
                self.draw_enemy(enemy);
            }
            self.draw_player();
            graphics::set_color(Color::WHITE);
            graphics::draw(&self.particles.trail, (0.0, 0.0));
            for debris in &self.particles.debris {
                graphics::draw(debris, (0.0, 0.0));
            }
            graphics::draw(&self.particles.sparks, (0.0, 0.0));
            self.draw_bullets();
            graphics::set_blend_mode(BlendMode::Alpha);

            graphics::set_font(&self.hud_font);
            for popup in &self.popups {
                let fade = 1.0 - popup.age / POPUP_SECONDS;
                graphics::set_color(with_alpha(TEXT, fade));
                graphics::printf(
                    &popup.text,
                    popup.position - Vec2::new(60.0, 10.0),
                    120.0,
                    Align::Center,
                );
            }

            if self.state == State::Playing && !self.using_pad && self.player.alive {
                let at = self.mouse_in_arena();
                graphics::set_line_width(1.5);
                graphics::set_color(with_alpha(Hue::Ship.color(), 0.8));
                graphics::circle(DrawMode::Line, at, 9.0);
                for i in 0..4 {
                    let direction = Vec2::from_angle(i as f32 * FRAC_PI_2 + FRAC_PI_4);
                    graphics::line(at + direction * 5.0, at + direction * 13.0);
                }
            }
        });
    }

    fn draw_post(&self) {
        let strength = if self.effects_on { 1.0f32 } else { 0.0 };
        let (radius, power, center) = match &self.shockwave {
            Some(shock) => {
                let progress = shock.age / SHOCK_SECONDS;
                let on_screen = shock.position - self.camera + Vec2::new(WIDTH, HEIGHT) / 2.0;
                (shock.age * SHOCK_SPEED, 1.0 - progress, on_screen)
            }
            None => (0.0, 0.0, Vec2::ZERO),
        };
        self.post.send("strength", strength);
        self.post.send("aberration", self.trauma * 6.0);
        self.post.send("shock_radius", radius);
        self.post.send("shock_power", power);
        self.post.send("shock_center", (center.x, center.y));
        self.post.send("size", (WIDTH, HEIGHT));

        graphics::set_color(Color::WHITE);
        graphics::set_blend_mode(BlendMode::Premultiplied);
        graphics::set_shader(Some(&self.post));
        graphics::draw(&self.scene, (0.0, 0.0));
        graphics::set_shader(None);
        graphics::set_blend_mode(BlendMode::Alpha);
    }

    fn draw_hud(&self) {
        graphics::set_font(&self.hud_font);
        graphics::set_color(TEXT);
        graphics::print(&format!("{:>8}", self.score), (20.0, 16.0));
        if self.multiplier > 1 {
            graphics::set_color(Hue::Amber.color());
            graphics::print(&format!("x{}", self.multiplier), (120.0, 16.0));
        }
        graphics::set_color(DIM);
        graphics::print(&format!("Wave {}", self.wave.max(1)), (20.0, 40.0));

        // Hull pips and the dash meter, top right.
        let ship = Hue::Ship.color();
        for i in 0..HULL {
            let filled = i < self.player.hull;
            graphics::set_color(if filled { ship } else { with_alpha(ship, 0.25) });
            let mode = if filled {
                DrawMode::Fill
            } else {
                DrawMode::Line
            };
            graphics::set_line_width(1.5);
            graphics::rectangle(mode, (WIDTH - 110.0 + i as f32 * 30.0, 18.0), (20.0, 12.0));
        }
        let ready = 1.0 - self.player.dash_cooldown / DASH_COOLDOWN;
        graphics::set_color(with_alpha(ship, 0.25));
        graphics::rectangle(DrawMode::Fill, (WIDTH - 110.0, 38.0), (80.0, 4.0));
        graphics::set_color(if ready >= 1.0 { ship } else { DIM });
        graphics::rectangle(DrawMode::Fill, (WIDTH - 110.0, 38.0), (80.0 * ready, 4.0));

        graphics::set_color(DIM);
        let effects = if self.effects_on { "on" } else { "off" };
        let toggle = if self.using_pad { "Back" } else { "Tab" };
        graphics::print(
            &format!(
                "{} fps   {} / {} bullets   {} particles   effects {effects} ({toggle})",
                timer::fps(),
                self.bullets.live_count(),
                self.bullets.capacity(),
                self.particles.count(),
            ),
            (20.0, HEIGHT - 30.0),
        );

        if self.state == State::Playing && self.banner < 2.0 && self.wave > 0 {
            let fade = (2.0 - self.banner).min(1.0);
            graphics::set_font(&self.big_font);
            graphics::set_color(with_alpha(TEXT, fade));
            graphics::printf(
                &format!("WAVE {}", self.wave),
                (0.0, HEIGHT / 2.0 - 110.0),
                WIDTH,
                Align::Center,
            );
        }
    }

    fn draw_centered(&self, lines: &[(&str, &Font, Color)], top: f32) {
        let mut y = top;
        for (text, font, color) in lines {
            graphics::set_font(font);
            graphics::set_color(*color);
            graphics::printf(text, (0.0, y), WIDTH, Align::Center);
            y += font.height() * 1.3;
        }
    }

    fn draw_screens(&self) {
        let fire = if self.using_pad {
            "Left stick moves   Right stick aims and fires   A dashes"
        } else {
            "W A S D move   Mouse aims   Click fires   Space dashes"
        };
        match self.state {
            State::Title => {
                let best = format!("Best {}", self.best);
                self.draw_centered(
                    &[
                        ("SHARDFALL", &self.title_font, Hue::Ship.color()),
                        ("", &self.hud_font, TEXT),
                        ("Press Enter or A to play", &self.big_font, TEXT),
                        ("", &self.hud_font, TEXT),
                        (fire, &self.hud_font, DIM),
                        ("Escape quits", &self.hud_font, DIM),
                        (
                            if self.best > 0 { best.as_str() } else { "" },
                            &self.hud_font,
                            DIM,
                        ),
                    ],
                    120.0,
                );
            }
            State::Paused => {
                self.draw_centered(
                    &[
                        ("Paused", &self.big_font, TEXT),
                        ("P or Start resumes   Escape gives up", &self.hud_font, DIM),
                    ],
                    HEIGHT / 2.0 - 50.0,
                );
            }
            State::GameOver => {
                let score = format!(
                    "Score {}   Wave {}   Best {}",
                    self.score, self.wave, self.best
                );
                self.draw_centered(
                    &[
                        ("Hull breached", &self.big_font, Hue::Pink.color()),
                        (score.as_str(), &self.hud_font, TEXT),
                        ("", &self.hud_font, TEXT),
                        (
                            "Enter or A plays again   Escape for the title",
                            &self.hud_font,
                            DIM,
                        ),
                    ],
                    HEIGHT / 2.0 - 70.0,
                );
            }
            State::Playing => {}
        }
    }
}

impl Game for Shardfall {
    fn load() -> smitten::Result<Self> {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |since| since.as_nanos() as u64);
        let dot = soft_dot(32);
        let post =
            Shader::from_file("shardfall-post.wgsl").unwrap_or_else(|error| panic!("{error}"));
        let mut player = Player::new();
        player.alive = false;
        Ok(Shardfall {
            state: State::Title,
            rng: RandomGenerator::new(seed),
            player,
            enemies: Vec::new(),
            bullets: BulletPool::new(POOL_SIZE),
            arrivals: Vec::new(),
            wave: 0,
            wave_break: 0.0,
            banner: 10.0,
            score: 0,
            best: 0,
            multiplier: 1,
            combo: 0.0,
            camera: ARENA / 2.0,
            trauma: 0.0,
            time_scale: 1.0,
            particles: Particles::new(&dot),
            rings: Vec::new(),
            popups: Vec::new(),
            shockwave: None,
            sounds: Sounds::new(),
            pad: joystick::gamepads().into_iter().next(),
            using_pad: false,
            scene: Canvas::new(WIDTH as u32, HEIGHT as u32)?,
            post,
            effects_on: true,
            hud_font: Font::default_at(16.0),
            big_font: Font::default_at(34.0),
            title_font: Font::default_at(84.0),
        })
    }

    fn update(&mut self, dt: f32) {
        // A long stall, such as dragging the window, should not teleport
        // everything across the arena.
        let real_dt = dt.min(1.0 / 20.0);
        if self.state == State::Paused {
            return;
        }
        self.time_scale += (1.0 - self.time_scale) * (3.0 * real_dt).min(1.0);
        let dt = real_dt * self.time_scale;

        if self.state == State::Playing {
            self.update_player(dt);
            self.update_waves(dt);
        }
        self.update_bullets(dt);
        self.update_enemies(dt);
        if self.state == State::Playing {
            self.collide();
        }
        self.update_effects(dt);
        self.update_camera(real_dt);
        self.trauma = (self.trauma - TRAUMA_DECAY * real_dt).max(0.0);
    }

    fn draw(&mut self) {
        // The title and game over text go through the screen effects too,
        // so they glow; the HUD is drawn after, to stay crisp.
        self.scene.render_to(|| {
            self.draw_world();
            self.draw_screens();
        });
        self.draw_post();
        if self.state != State::Title {
            self.draw_hud();
        }
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        if is_repeat {
            return;
        }
        self.using_pad = false;
        match key {
            Key::Return | Key::KpEnter => self.confirm(),
            Key::Space => self.dash(),
            Key::P => self.toggle_pause(),
            Key::Tab => self.effects_on = !self.effects_on,
            Key::Escape => self.back(),
            _ => {}
        }
    }

    fn mouse_moved(&mut self, _motion: MouseMotion) {
        self.using_pad = false;
    }

    fn mouse_pressed(&mut self, click: MouseClick) {
        self.using_pad = false;
        if click.button == MouseButton::Right {
            self.dash();
        }
    }

    fn gamepad_pressed(&mut self, pad: Joystick, button: GamepadButton) {
        self.pad = Some(pad);
        self.using_pad = true;
        match button {
            GamepadButton::A if self.state != State::Playing => self.confirm(),
            GamepadButton::A | GamepadButton::LeftShoulder | GamepadButton::RightShoulder => {
                self.dash();
            }
            GamepadButton::Start => {
                if self.state == State::Playing || self.state == State::Paused {
                    self.toggle_pause();
                } else {
                    self.confirm();
                }
            }
            GamepadButton::Back => self.effects_on = !self.effects_on,
            GamepadButton::B if self.state != State::Playing => self.back(),
            _ => {}
        }
    }

    fn joystick_added(&mut self, pad: Joystick) {
        if self.pad.is_none() && pad.is_gamepad() {
            self.pad = Some(pad);
        }
    }

    fn joystick_removed(&mut self, pad: Joystick) {
        if self.pad == Some(pad) {
            self.pad = joystick::gamepads().into_iter().next();
            self.using_pad = false;
            // Losing the controller mid-fight should not cost a life.
            if self.state == State::Playing {
                self.state = State::Paused;
            }
        }
    }
}

fn main() {
    smitten::run::<Shardfall>(Config {
        title: "Shardfall".to_string(),
        width: WIDTH as u32,
        height: HEIGHT as u32,
        resizable: true,
        design_size: Some((WIDTH as u32, HEIGHT as u32)),
        ..Config::default()
    });
}
