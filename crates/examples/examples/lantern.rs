//! Lantern, a small game about carrying a light through dark rooms.
//!
//! Shades live in the dark. They creep toward you whenever your lantern
//! is not on them, and stand still for as long as it is. Gather every
//! ember in a room to open its door, and get through all three rooms.
//!
//! You hear a shade before you see it: each one whispers while it moves,
//! from the side it is on, and sounds muffled when a wall is between
//! you. Silence means they are all caught in the light, or gone.
//!
//! - W, A, S, D or the arrow keys walk. On a gamepad, the left stick.
//! - The lantern points at the mouse, or where the right stick points.
//! - Space or A starts again after the game ends.
//! - Escape quits.
//!
//! Besides being a game, this shows how the pieces fit together: a light
//! map on a canvas with shadows cut out by the stencil, a shader for the
//! lantern's beam, positional and filtered sound, sounds made in code,
//! gamepad rumble and a best time kept in the save directory.

use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

use smitten::audio::{self, Filter, Reverb, Source};
use smitten::graphics::{Align, Canvas, DrawMode, Font, Image, Shader};
use smitten::sound::SoundData;
use smitten::{
    BlendMode, Color, CompareMode, Config, Game, GamepadAxis, GamepadButton, Joystick, Key, event,
    filesystem, graphics, joystick, keyboard, math, mouse,
};

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;
const TILE: f32 = 40.0;
const COLUMNS: usize = 20;
const ROWS: usize = 15;

const PLAYER_SPEED: f32 = 135.0;
const PLAYER_RADIUS: f32 = 10.0;
const SHADE_RADIUS: f32 = 13.0;
/// How far the lantern's beam reaches, and half its width in radians.
const BEAM_REACH: f32 = 330.0;
const BEAM_HALF_ANGLE: f32 = 0.44;
/// The dim glow right around the player, which holds nothing back.
const HALO_REACH: f32 = 80.0;
/// How long a shade waits after the light leaves it before it moves.
const SHADE_PATIENCE: f32 = 0.3;
const HEARTS: u32 = 3;
const BEST_FILE: &str = "lantern-best.txt";
const SHADOW_LENGTH: f32 = 2000.0;

/// The rooms. `#` is wall, `@` where you start, `e` an ember, `s` a
/// shade and `X` the door out.
const ROOMS: [[&str; ROWS]; 3] = [
    [
        "####################",
        "#@.....#...........#",
        "#......#.....e.....#",
        "#......#...........#",
        "#..#####....####...#",
        "#..............#...#",
        "#..............#.s.#",
        "#....e.........#...#",
        "#.......###........#",
        "#.........#........#",
        "#.........#....e...#",
        "#..s......#........#",
        "#.........#####....#",
        "#.................X#",
        "####################",
    ],
    [
        "####################",
        "#@...#.......#....e#",
        "#....#..s....#.....#",
        "#....#.......#..#..#",
        "#.......##......#..#",
        "#.......##......#..#",
        "###..#######..###..#",
        "#....#.....#.......#",
        "#.e..#..e..#...s...#",
        "#....#.....#.......#",
        "#..........####..###",
        "#....#.............#",
        "#..s.#.....#....e..#",
        "#....#.....#......X#",
        "####################",
    ],
    [
        "####################",
        "#@.................#",
        "#..##..##..##..##..#",
        "#..##..##..##..##.e#",
        "#........s.........#",
        "#.e................#",
        "#..##..##..##..##..#",
        "#..##..##..##..##..#",
        "#.....s......s.....#",
        "#..................#",
        "#..##..##..##..##..#",
        "#..##.e##..##..##..#",
        "#...........s....e.#",
        "#..s..............X#",
        "####################",
    ],
];

/// The lantern's light: a glow that fades with distance and is kept to
/// a cone around `direction`. A `spread` below -1 lets it shine all the
/// way round.
const BEAM: &str = "
struct Params {
    // A unit vector along the middle of the beam.
    direction: vec2<f32>,
    // Higher values give a smaller bright core and a longer dim tail.
    falloff: f32,
    // The cosine of half the beam's angle.
    spread: f32,
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let offset = (uv - vec2<f32>(0.5, 0.5)) * 2.0;
    let reach = length(offset);
    let along = dot(offset / max(reach, 0.0001), params.direction);
    let cone = smoothstep(params.spread - 0.07, params.spread + 0.07, along);
    let strength = pow(clamp(1.0 - reach, 0.0, 1.0), params.falloff) * cone;
    return vec4<f32>(color.rgb, color.a * strength);
}
";

struct Shade {
    x: f32,
    y: f32,
    home: (f32, f32),
    /// Whether the beam is on it now.
    lit: bool,
    /// Seconds since the beam left it.
    unlit_for: f32,
    /// Whether it moved this frame.
    moving: bool,
    /// Its whisper, placed where it stands.
    voice: Source,
    loudness: f32,
    muffled: bool,
    /// Sets its sway apart from the others'.
    seed: f32,
}

struct Ember {
    x: f32,
    y: f32,
    taken: bool,
}

#[derive(PartialEq)]
enum State {
    Playing,
    Lost,
    Won,
}

struct Sounds {
    step: Source,
    chime: Source,
    door: Source,
    sting: Source,
    thump: Source,
    over: Source,
    won: Source,
}

struct Lantern {
    room: usize,
    walls: [[bool; COLUMNS]; ROWS],
    /// Runs of wall in each row, as rectangles that cast shadows.
    wall_boxes: Vec<(f32, f32, f32, f32)>,
    /// For each tile, how many steps it is from the player.
    steps: [[u16; COLUMNS]; ROWS],
    start: (f32, f32),
    exit: (f32, f32),
    player: (f32, f32),
    /// The angle the lantern points at.
    aim: f32,
    last_mouse: (f32, f32),
    /// Whether the mouse was the last thing to point the lantern.
    aim_at_mouse: bool,
    shades: Vec<Shade>,
    embers: Vec<Ember>,
    hearts: u32,
    /// Seconds left during which nothing can catch the player.
    safe_for: f32,
    /// Fades from 1.0 after being caught, for the red flash.
    hurt: f32,
    state: State,
    /// Seconds spent playing this run, and the best finished run.
    time: f32,
    best: Option<f32>,
    new_record: bool,
    /// Seconds in this room, for the hint.
    room_time: f32,
    until_step: f32,
    until_thump: f32,
    light_map: Canvas,
    beam: Shader,
    halo: Shader,
    square: Image,
    heart: Image,
    whisper: SoundData,
    sounds: Sounds,
    big_font: Font,
    small_font: Font,
}

/// Makes a sound from a formula: `wave` gets the time in seconds and how
/// far through the sound it is, from 0 to 1.
fn synth_data(seconds: f32, mut wave: impl FnMut(f32, f32) -> f32) -> SoundData {
    const RATE: u32 = 44_100;
    let count = (seconds * RATE as f32) as usize;
    let mut data = SoundData::new(count, RATE, 1);
    for index in 0..count {
        let time = index as f32 / RATE as f32;
        data.set_sample(index, 0, wave(time, index as f32 / count as f32));
    }
    data
}

fn synth(seconds: f32, wave: impl FnMut(f32, f32) -> f32) -> Source {
    Source::from_sound_data(&synth_data(seconds, wave))
}

/// Two seconds of breathy noise that swells and dies away twice, and
/// loops without a click because it is silent at both ends.
fn make_whisper() -> SoundData {
    let mut smooth = 0.0;
    let mut smoother = 0.0;
    synth_data(2.0, |time, progress| {
        // Noise with the hiss taken off, twice, leaves a breath.
        let noise = math::random_range(-1.0, 1.0);
        smooth += (noise - smooth) * 0.18;
        smoother += (smooth - smoother) * 0.3;
        let swell = (progress * TAU).sin().powi(2);
        let flutter = 0.7 + 0.3 * (time * 23.0).sin();
        smoother * swell * flutter * 2.4
    })
}

fn make_sounds() -> Sounds {
    let sounds = Sounds {
        // A soft footfall.
        step: synth(0.06, |_, progress| {
            math::random_range(-1.0, 1.0) * (1.0 - progress).powi(4) * 0.25
        }),
        // Two rising notes for an ember.
        chime: synth(0.5, |time, progress| {
            let note = if progress < 0.3 { 660.0 } else { 990.0 };
            (TAU * note * time).sin() * (1.0 - progress).powi(2) * 0.35
        }),
        // A low swell as the door opens.
        door: synth(1.0, |time, progress| {
            let tone = (TAU * 110.0 * time).sin() + 0.5 * (TAU * 165.0 * time).sin();
            tone * (progress * PI).sin() * 0.3
        }),
        // A harsh falling shriek for being caught.
        sting: synth(0.6, |time, progress| {
            let tone = (TAU * (700.0 - 500.0 * progress) * time).sin();
            let grit = math::random_range(-1.0, 1.0) * 0.5;
            (tone + grit) * (1.0 - progress).powi(2) * 0.4
        }),
        // One beat of a heart.
        thump: synth(0.16, |time, progress| {
            (TAU * (70.0 - 25.0 * progress) * time).sin() * (1.0 - progress).powi(2) * 0.9
        }),
        // Three falling notes for the end of the game.
        over: synth(1.4, |time, progress| {
            let note = [294.0, 233.0, 175.0][((progress * 3.0) as usize).min(2)];
            (TAU * note * time).sin() * (1.0 - progress * 0.6) * 0.35
        }),
        // Four rising notes for getting out.
        won: synth(1.4, |time, progress| {
            let note = [392.0, 494.0, 587.0, 784.0][((progress * 4.0) as usize).min(3)];
            let fade = 1.0 - (progress * 4.0).fract() * 0.5;
            (TAU * note * time).sin() * fade * 0.3
        }),
    };
    // The louder sounds ring in the room a little.
    for source in [&sounds.chime, &sounds.door, &sounds.sting, &sounds.won] {
        source.set_effect("room", None);
    }
    sounds
}

/// The middle of a tile.
fn center(column: usize, row: usize) -> (f32, f32) {
    (
        column as f32 * TILE + TILE / 2.0,
        row as f32 * TILE + TILE / 2.0,
    )
}

/// The tile a point is in, kept inside the room.
fn tile_of(x: f32, y: f32) -> (usize, usize) {
    (
        ((x / TILE) as usize).min(COLUMNS - 1),
        ((y / TILE) as usize).min(ROWS - 1),
    )
}

/// The smallest turn that takes angle `from` to angle `to`.
fn angle_between(from: f32, to: f32) -> f32 {
    (to - from + PI).rem_euclid(TAU) - PI
}

impl Lantern {
    fn is_wall(&self, x: f32, y: f32) -> bool {
        if x < 0.0 || y < 0.0 || x >= WIDTH || y >= HEIGHT {
            return true;
        }
        let (column, row) = tile_of(x, y);
        self.walls[row][column]
    }

    /// Whether nothing stands between two points.
    fn can_see(&self, from: (f32, f32), to: (f32, f32)) -> bool {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let pieces = (dx.hypot(dy) / 6.0).ceil().max(1.0) as usize;
        (1..pieces).all(|piece| {
            let t = piece as f32 / pieces as f32;
            !self.is_wall(from.0 + dx * t, from.1 + dy * t)
        })
    }

    /// Whether the lantern's beam falls on a point.
    fn beam_reaches(&self, x: f32, y: f32) -> bool {
        let (dx, dy) = (x - self.player.0, y - self.player.1);
        dx.hypot(dy) < BEAM_REACH * 0.92
            && angle_between(self.aim, dy.atan2(dx)).abs() < BEAM_HALF_ANGLE
            && self.can_see(self.player, (x, y))
    }

    /// Sets the room up as it is at the start, keeping hearts and time.
    fn enter_room(&mut self, room: usize) {
        for shade in self.shades.drain(..) {
            shade.voice.stop();
        }
        self.room = room;
        self.embers.clear();
        self.wall_boxes.clear();
        for (row, line) in ROOMS[room].iter().enumerate() {
            assert_eq!(
                line.len(),
                COLUMNS,
                "room {room} row {row} is the wrong width"
            );
            let mut run_start = None;
            for (column, tile) in line.chars().enumerate() {
                let (x, y) = center(column, row);
                self.walls[row][column] = tile == '#';
                match tile {
                    '@' => self.start = (x, y),
                    'X' => self.exit = (x, y),
                    'e' => self.embers.push(Ember { x, y, taken: false }),
                    's' => {
                        let voice = Source::from_sound_data(&self.whisper);
                        voice.set_looping(true);
                        voice.set_volume(0.0);
                        // Each shade has its own voice and its own place
                        // in the breath.
                        voice.set_pitch(0.8 + 0.12 * self.shades.len() as f32);
                        voice.set_position(x, y, 0.0);
                        voice.set_attenuation_distances(70.0, 700.0);
                        voice.play();
                        voice.seek(math::random_range(0.0, 1.9) as f64);
                        self.shades.push(Shade {
                            x,
                            y,
                            home: (x, y),
                            lit: false,
                            unlit_for: 0.0,
                            moving: false,
                            voice,
                            loudness: 0.0,
                            muffled: false,
                            seed: math::random_range(0.0, 100.0),
                        });
                    }
                    _ => {}
                }
                // Walls next to each other in a row cast one shadow.
                if tile == '#' && run_start.is_none() {
                    run_start = Some(column);
                }
                let ends = tile != '#' || column == COLUMNS - 1;
                if let (Some(first), true) = (run_start, ends) {
                    let last = if tile == '#' { column + 1 } else { column };
                    self.wall_boxes.push((
                        first as f32 * TILE,
                        row as f32 * TILE,
                        (last - first) as f32 * TILE,
                        TILE,
                    ));
                    run_start = None;
                }
            }
        }
        self.player = self.start;
        self.safe_for = 1.5;
        self.room_time = 0.0;
    }

    /// Starts the whole game again.
    fn restart(&mut self) {
        self.hearts = HEARTS;
        self.time = 0.0;
        self.hurt = 0.0;
        self.new_record = false;
        self.state = State::Playing;
        self.enter_room(0);
    }

    fn door_is_open(&self) -> bool {
        self.embers.iter().all(|ember| ember.taken)
    }

    /// Which way the player wants to walk, no longer than 1.
    fn walking(&self) -> (f32, f32) {
        let (mut x, mut y) = (0.0, 0.0);
        if keyboard::is_any_down(&[Key::Left, Key::A]) {
            x -= 1.0;
        }
        if keyboard::is_any_down(&[Key::Right, Key::D]) {
            x += 1.0;
        }
        if keyboard::is_any_down(&[Key::Up, Key::W]) {
            y -= 1.0;
        }
        if keyboard::is_any_down(&[Key::Down, Key::S]) {
            y += 1.0;
        }
        if let Some(pad) = joystick::joysticks().first() {
            let (stick_x, stick_y) = (
                pad.gamepad_axis(GamepadAxis::LeftX),
                pad.gamepad_axis(GamepadAxis::LeftY),
            );
            if stick_x.hypot(stick_y) > 0.25 {
                x += stick_x;
                y += stick_y;
            }
        }
        let length = x.hypot(y);
        if length > 1.0 {
            (x / length, y / length)
        } else {
            (x, y)
        }
    }

    /// Points the lantern: at the mouse when it moves, or along the right
    /// stick when that is pushed.
    fn take_aim(&mut self) {
        let mouse = mouse::position();
        if mouse != self.last_mouse {
            self.last_mouse = mouse;
            self.aim_at_mouse = true;
        }
        if self.aim_at_mouse {
            self.aim = (mouse.1 - self.player.1).atan2(mouse.0 - self.player.0);
        }
        if let Some(pad) = joystick::joysticks().first() {
            let (x, y) = (
                pad.gamepad_axis(GamepadAxis::RightX),
                pad.gamepad_axis(GamepadAxis::RightY),
            );
            if x.hypot(y) > 0.4 {
                self.aim_at_mouse = false;
                self.aim = y.atan2(x);
            }
        }
    }

    /// Moves the player, sliding along walls.
    fn walk(&mut self, dx: f32, dy: f32) {
        let fits = |game: &Lantern, x: f32, y: f32| {
            let r = PLAYER_RADIUS;
            [(-r, -r), (r, -r), (r, r), (-r, r)]
                .iter()
                .all(|(ox, oy)| !game.is_wall(x + ox, y + oy))
        };
        if fits(self, self.player.0 + dx, self.player.1) {
            self.player.0 += dx;
        }
        if fits(self, self.player.0, self.player.1 + dy) {
            self.player.1 += dy;
        }
    }

    /// Counts, for every tile, the steps from it to the player, so each
    /// shade can find its way round the walls by walking downhill.
    fn count_steps(&mut self) {
        self.steps = [[u16::MAX; COLUMNS]; ROWS];
        let (column, row) = tile_of(self.player.0, self.player.1);
        self.steps[row][column] = 0;
        let mut waiting = VecDeque::from([(column, row)]);
        while let Some((column, row)) = waiting.pop_front() {
            let next = self.steps[row][column] + 1;
            for (dc, dr) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (c, r) = (column as i32 + dc, row as i32 + dr);
                if c < 0 || r < 0 || c >= COLUMNS as i32 || r >= ROWS as i32 {
                    continue;
                }
                let (c, r) = (c as usize, r as usize);
                if !self.walls[r][c] && self.steps[r][c] == u16::MAX {
                    self.steps[r][c] = next;
                    waiting.push_back((c, r));
                }
            }
        }
    }

    /// Where a shade at `(x, y)` should head for next.
    fn way_to_player(&self, x: f32, y: f32) -> (f32, f32) {
        let (column, row) = tile_of(x, y);
        let here = self.steps[row][column];
        if here <= 1 || self.can_see((x, y), self.player) {
            return self.player;
        }
        let mut best = (here, center(column, row));
        for (dc, dr) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (c, r) = (column as i32 + dc, row as i32 + dr);
            if c < 0 || r < 0 || c >= COLUMNS as i32 || r >= ROWS as i32 {
                continue;
            }
            let steps = self.steps[r as usize][c as usize];
            if steps < best.0 {
                best = (steps, center(c as usize, r as usize));
            }
        }
        best.1
    }

    fn update_shades(&mut self, dt: f32) {
        self.count_steps();
        let speed = 48.0 + 6.0 * self.room as f32;
        for index in 0..self.shades.len() {
            let (x, y) = (self.shades[index].x, self.shades[index].y);
            let lit = self.beam_reaches(x, y);
            let target = self.way_to_player(x, y);
            let in_sight = self.can_see((x, y), self.player);
            let shade = &mut self.shades[index];
            shade.lit = lit;
            shade.unlit_for = if lit { 0.0 } else { shade.unlit_for + dt };
            shade.moving = !lit && shade.unlit_for > SHADE_PATIENCE;
            if shade.moving {
                let (dx, dy) = (target.0 - x, target.1 - y);
                let distance = dx.hypot(dy).max(0.001);
                let step = (speed * dt).min(distance);
                shade.x += dx / distance * step;
                shade.y += dy / distance * step;
            }

            // The whisper follows the shade, and fades in and out with
            // its moving.
            let wanted = if shade.moving { 1.0 } else { 0.0 };
            shade.loudness += (wanted - shade.loudness) * (dt * 6.0).min(1.0);
            shade.voice.set_volume(shade.loudness);
            shade.voice.set_position(shade.x, shade.y, 0.0);
            if shade.muffled == in_sight {
                shade.muffled = !in_sight;
                shade
                    .voice
                    .set_filter(shade.muffled.then(|| Filter::low_pass(0.12)));
            }
        }
    }

    /// The player was touched: lose a heart and start the room over.
    fn caught(&mut self) {
        self.hearts = self.hearts.saturating_sub(1);
        self.hurt = 1.0;
        self.sounds.sting.play_once();
        if let Some(pad) = joystick::joysticks().first() {
            pad.set_vibration(1.0, 0.6, 0.35);
        }
        if self.hearts == 0 {
            self.state = State::Lost;
            self.sounds.over.play_once();
            self.hush();
            return;
        }
        self.player = self.start;
        self.safe_for = 1.5;
        for shade in &mut self.shades {
            (shade.x, shade.y) = shade.home;
            shade.unlit_for = 0.0;
        }
    }

    /// Silences every shade.
    fn hush(&mut self) {
        for shade in &mut self.shades {
            shade.loudness = 0.0;
            shade.moving = false;
            shade.voice.set_volume(0.0);
        }
    }

    fn leave_room(&mut self) {
        if self.room + 1 < ROOMS.len() {
            self.sounds.door.play_once();
            self.enter_room(self.room + 1);
            return;
        }
        self.state = State::Won;
        self.hush();
        self.sounds.won.play_once();
        if self.best.is_none_or(|best| self.time < best) {
            self.best = Some(self.time);
            self.new_record = true;
            if let Err(error) = filesystem::write(BEST_FILE, format!("{}", self.time)) {
                eprintln!("could not save the best time: {error}");
            }
        }
    }

    /// The room as it would look in full light.
    fn draw_room(&self) {
        graphics::clear(Color::rgb(0.2, 0.19, 0.2));
        for row in 0..ROWS {
            for column in 0..COLUMNS {
                let (x, y) = (column as f32 * TILE, row as f32 * TILE);
                let grain = math::noise2(column as f32 * 0.73, row as f32 * 0.61);
                if self.walls[row][column] {
                    let shade = 0.5 + grain * 0.12;
                    graphics::set_color(Color::rgb(shade * 0.85, shade * 0.8, shade));
                    graphics::rectangle(DrawMode::Fill, x, y, TILE, TILE);
                    // A lighter cap, so the walls read as standing up.
                    graphics::set_color(Color::rgba(1.0, 1.0, 1.0, 0.12));
                    graphics::rectangle(DrawMode::Fill, x, y, TILE, 6.0);
                } else {
                    let shade = 0.36 + grain * 0.14;
                    graphics::set_color(Color::rgb(shade, shade * 0.93, shade * 0.86));
                    graphics::rectangle(DrawMode::Fill, x + 1.0, y + 1.0, TILE - 2.0, TILE - 2.0);
                }
            }
        }

        // The door: shut and dull, or open and bright.
        let (x, y) = self.exit;
        let open = self.door_is_open();
        graphics::set_color(if open {
            Color::rgb(0.5, 1.0, 0.7)
        } else {
            Color::rgb(0.3, 0.2, 0.15)
        });
        graphics::rounded_rectangle(DrawMode::Fill, x - 15.0, y - 17.0, 30.0, 34.0, 12.0, 12.0);
        graphics::set_color(Color::rgb(0.12, 0.1, 0.1));
        if open {
            graphics::rounded_rectangle(DrawMode::Fill, x - 9.0, y - 11.0, 18.0, 28.0, 8.0, 8.0);
        } else {
            graphics::circle(DrawMode::Fill, x + 6.0, y + 2.0, 2.5);
        }

        for ember in self.embers.iter().filter(|ember| !ember.taken) {
            let pulse = 0.5 + 0.5 * (self.time * 4.0 + ember.x).sin();
            graphics::set_color(Color::rgb(1.0, 0.55, 0.15));
            graphics::circle(DrawMode::Fill, ember.x, ember.y, 6.0 + pulse * 1.5);
            graphics::set_color(Color::rgb(1.0, 0.9, 0.5));
            graphics::circle(DrawMode::Fill, ember.x, ember.y, 3.0);
        }

        // Shades are part of the room, so they show only where light
        // falls on them.
        for shade in &self.shades {
            // Not even a faint outline in the dark.
            let distance = (shade.x - self.player.0).hypot(shade.y - self.player.1);
            if !shade.lit && distance > HALO_REACH + SHADE_RADIUS {
                continue;
            }
            let sway = (self.time * 3.0 + shade.seed).sin() * if shade.lit { 0.0 } else { 1.5 };
            let (x, y) = (shade.x + sway, shade.y);
            graphics::set_color(Color::rgb(0.75, 0.7, 0.9));
            graphics::circle(DrawMode::Fill, x, y - 2.0, SHADE_RADIUS);
            graphics::polygon(
                DrawMode::Fill,
                &[
                    (x - SHADE_RADIUS, y - 2.0),
                    (x + SHADE_RADIUS, y - 2.0),
                    (x + SHADE_RADIUS, y + 15.0),
                    (x + 6.0, y + 10.0),
                    (x, y + 15.0),
                    (x - 6.0, y + 10.0),
                    (x - SHADE_RADIUS, y + 15.0),
                ],
            );
            graphics::set_color(Color::rgb(0.1, 0.05, 0.15));
            graphics::ellipse(DrawMode::Fill, x - 5.0, y - 3.0, 2.5, 4.0);
            graphics::ellipse(DrawMode::Fill, x + 5.0, y - 3.0, 2.5, 4.0);
        }
    }

    /// Marks in the stencil everything a light at `(x, y)` cannot reach:
    /// the far side of every wall, stretched away from the light.
    fn draw_shadows(&self, x: f32, y: f32) {
        for &(left, top, width, height) in &self.wall_boxes {
            let corners = [
                (left, top),
                (left + width, top),
                (left + width, top + height),
                (left, top + height),
            ];
            for index in 0..4 {
                let (ax, ay) = corners[index];
                let (bx, by) = corners[(index + 1) % 4];
                // Only edges that face away from the light cast shadows,
                // so the wall itself stays lit.
                let (outward_x, outward_y) = (by - ay, ax - bx);
                if outward_x * (x - ax) + outward_y * (y - ay) >= 0.0 {
                    continue;
                }
                let far = |px: f32, py: f32| {
                    let (dx, dy) = (px - x, py - y);
                    let length = dx.hypot(dy).max(0.001);
                    (
                        px + dx / length * SHADOW_LENGTH,
                        py + dy / length * SHADOW_LENGTH,
                    )
                };
                graphics::polygon(
                    DrawMode::Fill,
                    &[(ax, ay), (bx, by), far(bx, by), far(ax, ay)],
                );
            }
        }
    }

    /// Draws one glow onto the light map with the shader that is set.
    fn draw_glow(&self, x: f32, y: f32, reach: f32, color: Color) {
        graphics::set_color(color);
        let scale = reach * 2.0 / self.square.dimensions().0 as f32;
        graphics::draw_ex(
            &self.square,
            x - reach,
            y - reach,
            0.0,
            scale,
            scale,
            0.0,
            0.0,
        );
    }

    /// Fills the light map: near darkness, plus every light.
    fn draw_light_map(&self) {
        let (x, y) = self.player;
        let flicker = math::noise(self.time * 7.0);
        self.beam
            .send("direction", (self.aim.cos(), self.aim.sin()));
        self.light_map.render_to(|| {
            graphics::clear(Color::rgb(0.02, 0.02, 0.035));
            graphics::set_blend_mode(BlendMode::Add);

            // The lantern's two lights start at the same place, so they
            // share one set of shadows.
            graphics::stencil(|| self.draw_shadows(x, y));
            graphics::set_stencil_test(CompareMode::Equal, 0);
            graphics::set_shader(Some(&self.beam));
            self.draw_glow(
                x,
                y,
                BEAM_REACH + flicker * 14.0,
                Color::rgb(1.0, 0.9, 0.68),
            );
            graphics::set_shader(Some(&self.halo));
            self.draw_glow(x, y, HALO_REACH, Color::rgba(1.0, 0.8, 0.55, 0.5));
            graphics::clear_stencil_test();

            // Small lights that need no shadows.
            for ember in self.embers.iter().filter(|ember| !ember.taken) {
                let pulse = 0.5 + 0.5 * (self.time * 4.0 + ember.x).sin();
                self.draw_glow(
                    ember.x,
                    ember.y,
                    34.0 + pulse * 8.0,
                    Color::rgb(1.0, 0.6, 0.25),
                );
            }
            if self.door_is_open() {
                let (x, y) = self.exit;
                self.draw_glow(x, y, 120.0, Color::rgb(0.45, 1.0, 0.65));
            }
            graphics::set_shader(None);
            graphics::set_blend_mode(BlendMode::Alpha);
        });
    }

    /// What shows whatever the light: eyes in the dark, and the player.
    fn draw_over_the_dark(&self) {
        for shade in self.shades.iter().filter(|shade| !shade.lit) {
            let distance = (shade.x - self.player.0).hypot(shade.y - self.player.1);
            if distance > 260.0 || !self.can_see((shade.x, shade.y), self.player) {
                continue;
            }
            let blink = if (self.time * 0.7 + shade.seed).fract() < 0.06 {
                0.0
            } else {
                1.0
            };
            let alpha = (1.0 - distance / 260.0) * 0.85 * blink;
            graphics::set_color(Color::rgb(1.0, 0.25, 0.2).with_alpha(alpha));
            graphics::circle(DrawMode::Fill, shade.x - 5.0, shade.y - 3.0, 1.8);
            graphics::circle(DrawMode::Fill, shade.x + 5.0, shade.y - 3.0, 1.8);
        }

        let (x, y) = self.player;
        // Blinks while nothing can catch the player.
        if self.safe_for > 0.0 && (self.safe_for * 10.0) as i32 % 2 == 0 {
            return;
        }
        graphics::set_color(Color::rgb(0.25, 0.3, 0.45));
        graphics::circle(DrawMode::Fill, x, y, PLAYER_RADIUS);
        graphics::set_color(Color::rgb(0.9, 0.8, 0.7));
        graphics::circle(DrawMode::Fill, x, y - 2.0, 5.0);
        // The lantern, held out toward where it shines.
        let (lx, ly) = (x + self.aim.cos() * 12.0, y + self.aim.sin() * 12.0);
        graphics::set_color(Color::rgb(1.0, 0.85, 0.45));
        graphics::circle(DrawMode::Fill, lx, ly, 4.0);
    }

    fn draw_hud(&self) {
        graphics::set_font(&self.small_font);
        let found = self.embers.iter().filter(|ember| ember.taken).count();
        graphics::set_color(Color::WHITE);
        graphics::print(
            &format!(
                "Room {} of {}    Embers {found} of {}    {:.1} s",
                self.room + 1,
                ROOMS.len(),
                self.embers.len(),
                self.time
            ),
            12.0,
            10.0,
        );
        for heart in 0..HEARTS {
            let alpha = if heart < self.hearts { 1.0 } else { 0.18 };
            graphics::set_color(Color::WHITE.with_alpha(alpha));
            graphics::draw_ex(
                &self.heart,
                WIDTH - 36.0 - heart as f32 * 30.0,
                6.0,
                0.0,
                1.7,
                1.7,
                0.0,
                0.0,
            );
        }

        if self.state == State::Playing && self.room_time < 7.0 {
            let hint = match self.room {
                0 => "Shades move only in the dark. Keep the light on them, and listen.",
                1 => "A muffled whisper is behind a wall.",
                _ => "You cannot watch them all.",
            };
            let alpha = (7.0 - self.room_time).min(1.0) * 0.85;
            graphics::set_color(Color::WHITE.with_alpha(alpha));
            graphics::printf(hint, 0.0, HEIGHT - 32.0, WIDTH, Align::Center);
        } else if self.state == State::Playing && self.door_is_open() {
            graphics::set_color(Color::rgb(0.5, 1.0, 0.7).with_alpha(0.85));
            graphics::printf(
                "The door is open.",
                0.0,
                HEIGHT - 32.0,
                WIDTH,
                Align::Center,
            );
        }

        if self.hurt > 0.0 {
            graphics::set_color(Color::rgb(0.8, 0.05, 0.05).with_alpha(self.hurt * 0.5));
            graphics::rectangle(DrawMode::Fill, 0.0, 0.0, WIDTH, HEIGHT);
        }

        if self.state == State::Playing {
            return;
        }
        graphics::set_color(Color::BLACK.with_alpha(0.6));
        graphics::rectangle(DrawMode::Fill, 0.0, 0.0, WIDTH, HEIGHT);
        graphics::set_color(Color::WHITE);
        graphics::set_font(&self.big_font);
        let won = self.state == State::Won;
        let title = if won {
            "You got out."
        } else {
            "The dark took you."
        };
        graphics::printf(title, 0.0, 190.0, WIDTH, Align::Center);
        graphics::set_font(&self.small_font);
        if won {
            graphics::printf(
                &format!("It took {:.1} seconds.", self.time),
                0.0,
                260.0,
                WIDTH,
                Align::Center,
            );
            if self.new_record {
                graphics::set_color(Color::rgb(1.0, 0.85, 0.3));
                graphics::printf("A new best!", 0.0, 288.0, WIDTH, Align::Center);
            } else if let Some(best) = self.best {
                graphics::set_color(Color::WHITE.with_alpha(0.7));
                graphics::printf(
                    &format!("Your best is {best:.1}."),
                    0.0,
                    288.0,
                    WIDTH,
                    Align::Center,
                );
            }
        }
        graphics::set_color(Color::WHITE.with_alpha(0.7));
        graphics::printf(
            "Space to try again.  Escape to quit.",
            0.0,
            340.0,
            WIDTH,
            Align::Center,
        );
    }
}

impl Game for Lantern {
    fn load() -> Self {
        graphics::set_background_color(Color::BLACK);
        audio::set_effect(
            "room",
            Reverb {
                decay_time: 1.8,
                ..Reverb::default()
            },
        );

        let beam = graphics::new_shader(BEAM);
        beam.send("falloff", 1.1_f32);
        beam.send("spread", BEAM_HALF_ANGLE.cos());
        beam.send("direction", (1.0_f32, 0.0_f32));
        // The same shader shining all the way round.
        let halo = graphics::new_shader(BEAM);
        halo.send("falloff", 1.6_f32);
        halo.send("spread", -2.0_f32);
        halo.send("direction", (1.0_f32, 0.0_f32));

        let best = filesystem::read_string(BEST_FILE)
            .ok()
            .and_then(|text| text.trim().parse().ok());

        let mut game = Lantern {
            room: 0,
            walls: [[false; COLUMNS]; ROWS],
            wall_boxes: Vec::new(),
            steps: [[u16::MAX; COLUMNS]; ROWS],
            start: (0.0, 0.0),
            exit: (0.0, 0.0),
            player: (0.0, 0.0),
            aim: 0.0,
            last_mouse: mouse::position(),
            aim_at_mouse: true,
            shades: Vec::new(),
            embers: Vec::new(),
            hearts: HEARTS,
            safe_for: 0.0,
            hurt: 0.0,
            state: State::Playing,
            time: 0.0,
            best,
            new_record: false,
            room_time: 0.0,
            until_step: 0.0,
            until_thump: 0.0,
            light_map: graphics::new_canvas(WIDTH as u32, HEIGHT as u32),
            beam,
            halo,
            square: Image::from_rgba(4, 4, &[255; 4 * 4 * 4]).expect("a 4 by 4 image is valid"),
            heart: graphics::new_image("heart.png"),
            whisper: make_whisper(),
            sounds: make_sounds(),
            big_font: graphics::new_default_font(44.0),
            small_font: graphics::new_default_font(16.0),
        };
        game.restart();
        game
    }

    fn update(&mut self, dt: f32) {
        let dt = dt.min(1.0 / 30.0);
        self.hurt = (self.hurt - dt * 1.5).max(0.0);
        if self.state != State::Playing {
            return;
        }
        self.time += dt;
        self.room_time += dt;
        self.safe_for = (self.safe_for - dt).max(0.0);

        let (walk_x, walk_y) = self.walking();
        self.walk(walk_x * PLAYER_SPEED * dt, walk_y * PLAYER_SPEED * dt);
        self.take_aim();
        // The player is the one listening.
        audio::set_position(self.player.0, self.player.1, 0.0);

        if walk_x != 0.0 || walk_y != 0.0 {
            self.until_step -= dt;
            if self.until_step <= 0.0 {
                self.until_step = 0.3;
                self.sounds
                    .step
                    .play_once_with(0.5, math::random_range(0.8, 1.2));
            }
        }

        self.update_shades(dt);

        // Embers, the door and the shades themselves.
        let (px, py) = self.player;
        let mut picked = false;
        for ember in &mut self.embers {
            if !ember.taken && (ember.x - px).hypot(ember.y - py) < 20.0 {
                ember.taken = true;
                picked = true;
            }
        }
        if picked {
            self.sounds.chime.play_once();
            if self.door_is_open() {
                self.sounds.door.play_once();
            }
        }
        if self.door_is_open() && (self.exit.0 - px).hypot(self.exit.1 - py) < 20.0 {
            self.leave_room();
            return;
        }
        let nearest = self
            .shades
            .iter()
            .map(|shade| (shade.x - px).hypot(shade.y - py))
            .fold(f32::MAX, f32::min);
        if nearest < PLAYER_RADIUS + SHADE_RADIUS - 3.0 && self.safe_for <= 0.0 {
            self.caught();
            return;
        }

        // A heartbeat that quickens as something gets close.
        self.until_thump -= dt;
        if nearest < 220.0 && self.until_thump <= 0.0 {
            let closeness = 1.0 - nearest / 220.0;
            self.until_thump = 1.0 - closeness * 0.7;
            self.sounds.thump.play_once_with(0.3 + closeness * 0.7, 1.0);
            if let Some(pad) = joystick::joysticks().first() {
                pad.set_vibration(closeness * 0.5, 0.0, 0.08);
            }
        }
    }

    fn draw(&mut self) {
        self.draw_room();
        self.draw_light_map();
        // White in the light map leaves the room as it is and black
        // hides it.
        graphics::set_color(Color::WHITE);
        graphics::set_blend_mode(BlendMode::Multiply);
        graphics::draw(&self.light_map, 0.0, 0.0);
        graphics::set_blend_mode(BlendMode::Alpha);
        self.draw_over_the_dark();
        self.draw_hud();
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        if is_repeat {
            return;
        }
        match key {
            Key::Space | Key::Return if self.state != State::Playing => self.restart(),
            Key::Escape => event::quit(),
            _ => {}
        }
    }

    fn gamepad_pressed(&mut self, _pad: Joystick, button: GamepadButton) {
        if button == GamepadButton::A && self.state != State::Playing {
            self.restart();
        }
    }
}

fn main() {
    smitten::run::<Lantern>(Config {
        title: "Lantern".to_string(),
        width: WIDTH as u32,
        height: HEIGHT as u32,
        identity: Some("smitten-lantern".to_string()),
        ..Config::default()
    });
}
