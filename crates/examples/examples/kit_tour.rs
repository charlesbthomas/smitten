//! A tour of smitten-kit: a one-room platformer where you collect every
//! coin as fast as you can.
//!
//! - A and D (or the arrows, or a gamepad's stick) run, and Space (or A on
//!   a gamepad) jumps.
//! - Escape (or B) pauses. The menus work with the keyboard, a gamepad or
//!   the mouse.
//! - F3 shows the debug overlay.
//!
//! Each part comes from the kit:
//!
//! - The level is a map in the Tiled editor's JSON format, read with
//!   `tilemap`, drawn culled to what the camera sees. Which tiles are solid
//!   is a property set in the tileset.
//! - The hero is an `Animator` playing clips from a `SpriteSheet`, and
//!   moves with `collision::move_and_slide` against the map.
//! - Coins live in a `Pool`; picking one up sends an event through
//!   `Events`, which shakes the `Camera` and floats a `Tween`ed "+1".
//! - The title, settings and pause screens are scenes, with menus built by
//!   `ui::Ui` and driven by `input::Actions`.
//! - The settings and best time are kept in a `save::SaveSlots` slot.
//!
//! The level and its art are made in code so the example needs no asset
//! files; a game would draw them in Tiled and an image editor and call
//! `TiledMap::load` instead.
//!
//! Run it with `cargo run --example kit_tour`.

use serde::{Deserialize, Serialize};
use smitten::graphics::{self, Align, DrawMode, Image, ImageData};
use smitten::{Color, Config, GamepadAxis, GamepadButton, Key, MouseButton, mouse};
use smitten_kit::prelude::*;
use smitten_kit::save::SaveSlots;
use smitten_kit::tilemap::{TileMap, TiledMap};
use smitten_kit::tween::{Ease, Tween};

const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 540.0;
const TILE: f32 = 16.0;

const GRAVITY: f32 = 1100.0;
const RUN_SPEED: f32 = 150.0;
const JUMP_SPEED: f32 = 360.0;
/// Seconds after leaving a ledge that a jump still works.
const COYOTE: f32 = 0.08;
const HERO_SIZE: Vec2 = Vec2::new(10.0, 15.0);

const FADE: f32 = 0.25;
const BACKGROUND: Color = Color::rgb(0.09, 0.1, 0.16);
const TEXT: Color = Color::rgb(0.92, 0.94, 1.0);

/// The room, one character per tile: `#` ground, `=` brick, `*` grass on
/// a layer of its own, `o` a coin, `@` where the hero starts.
const LEVEL: [&str; 20] = [
    "============================================================",
    "=..........................................................=",
    "=..........................................................=",
    "=.............o.o.o.........................o..............=",
    "=...........=======...................o...=====............=",
    "=.....................o..............===...................=",
    "=....................===.......o...........................=",
    "=..o..........................===.............o.o.o........=",
    "=.====................................=====..=======.......=",
    "=.........o.o..............o...............................=",
    "=........=====...........=====...............o.............=",
    "=...................o.......................===............=",
    "=..................===...........o.o.......................=",
    "=..o........................=========..............o.......=",
    "=.===.......o........o..........................======.....=",
    "=.........=====...=======..................................=",
    "=.*..@.*.....*..................*....**........*...........=",
    "############################################################",
    "############################################################",
    "############################################################",
];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Left,
    Right,
    Up,
    Down,
    Jump,
    Confirm,
    Back,
    Click,
}

const NAV: UiActions<Act> = UiActions {
    up: Act::Up,
    down: Act::Down,
    left: Act::Left,
    right: Act::Right,
    confirm: Act::Confirm,
    back: Act::Back,
};

fn controls() -> Actions<Act> {
    let mut actions = Actions::new();
    actions
        .bind(
            Act::Left,
            [
                Binding::Key(Key::A),
                Binding::Key(Key::Left),
                Binding::Button(GamepadButton::DpadLeft),
                Binding::stick_negative(GamepadAxis::LeftX),
            ],
        )
        .bind(
            Act::Right,
            [
                Binding::Key(Key::D),
                Binding::Key(Key::Right),
                Binding::Button(GamepadButton::DpadRight),
                Binding::stick_positive(GamepadAxis::LeftX),
            ],
        )
        .bind(
            Act::Up,
            [
                Binding::Key(Key::W),
                Binding::Key(Key::Up),
                Binding::Button(GamepadButton::DpadUp),
                Binding::stick_negative(GamepadAxis::LeftY),
            ],
        )
        .bind(
            Act::Down,
            [
                Binding::Key(Key::S),
                Binding::Key(Key::Down),
                Binding::Button(GamepadButton::DpadDown),
                Binding::stick_positive(GamepadAxis::LeftY),
            ],
        )
        .bind(
            Act::Jump,
            [
                Binding::Key(Key::Space),
                Binding::Key(Key::W),
                Binding::Key(Key::Up),
                Binding::Button(GamepadButton::A),
            ],
        )
        .bind(
            Act::Confirm,
            [
                Binding::Key(Key::Return),
                Binding::Key(Key::Space),
                Binding::Button(GamepadButton::A),
            ],
        )
        .bind(
            Act::Back,
            [
                Binding::Key(Key::Escape),
                Binding::Button(GamepadButton::B),
                Binding::Button(GamepadButton::Start),
            ],
        )
        .bind(Act::Click, [Binding::Mouse(MouseButton::Left)]);
    actions
}

/// What is kept between runs.
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Record {
    zoom: f32,
    show_debug: bool,
    best_seconds: Option<f32>,
}

impl Default for Record {
    fn default() -> Self {
        Record {
            zoom: 2.0,
            show_debug: false,
            best_seconds: None,
        }
    }
}

/// What every scene shares.
struct Shared {
    actions: Actions<Act>,
    debug: DebugOverlay,
    slots: Option<SaveSlots>,
    record: Record,
    level: TiledMap,
    hero: SpriteSheet,
}

impl Shared {
    fn menu_input(&self) -> UiInput {
        UiInput::from_actions(&self.actions, &NAV).with_pointer(
            mouse::position(),
            self.actions.is_pressed(Act::Click),
            self.actions.is_down(Act::Click),
        )
    }

    fn save(&self) {
        if let Some(slots) = &self.slots
            && let Err(error) = slots.save("record", &self.record)
        {
            eprintln!("could not save: {error}");
        }
    }
}

impl scene::Context for Shared {
    fn event(&mut self, event: &Event) {
        if self.debug.handle(event) {
            self.record.show_debug = self.debug.is_visible();
        }
        self.actions.handle(event);
    }

    fn update(&mut self, dt: f32) {
        self.actions.update(dt);
        self.debug.update(dt);
        self.debug.set_visible(self.record.show_debug);
    }

    fn draw(&mut self) {
        self.debug.draw();
    }

    fn background(&self) -> Color {
        BACKGROUND
    }
}

fn menu_area(rows: f32) -> Rect {
    let size = Vec2::new(320.0, rows * 38.0);
    Rect::from_center_size((WIDTH / 2.0, HEIGHT / 2.0 + 60.0), size)
}

fn title_text(text: &str, y: f32) {
    graphics::set_color(TEXT);
    graphics::printf(text, (0.0, y), WIDTH, Align::Center);
}

// The title screen.

struct Title {
    ui: Ui,
}

impl Scene<Shared> for Title {
    fn name(&self) -> &str {
        "title"
    }

    fn update(&mut self, ctx: &mut Shared, _dt: f32) -> Transition<Shared> {
        self.ui.begin(menu_area(3.0), ctx.menu_input());
        let play = self.ui.button("Play");
        let settings = self.ui.button("Settings");
        let quit = self.ui.button("Quit") || self.ui.back();
        self.ui.end();
        if play {
            Transition::switch(Play::new(ctx)).fade(FADE, BACKGROUND)
        } else if settings {
            Transition::push(Settings { ui: Ui::new() })
        } else if quit {
            Transition::Quit
        } else {
            Transition::None
        }
    }

    fn draw(&mut self, ctx: &mut Shared) {
        title_text("COIN DASH", 120.0);
        let best = match ctx.record.best_seconds {
            Some(seconds) => format!("Best time {seconds:.2} s"),
            None => "Collect every coin as fast as you can".to_owned(),
        };
        title_text(&best, 160.0);
        self.ui.draw();
    }
}

// Settings, over whatever opened them.

struct Settings {
    ui: Ui,
}

impl Scene<Shared> for Settings {
    fn name(&self) -> &str {
        "settings"
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn update(&mut self, ctx: &mut Shared, _dt: f32) -> Transition<Shared> {
        self.ui.begin(menu_area(4.0), ctx.menu_input());
        self.ui.label("Settings");
        self.ui
            .slider("Zoom", &mut ctx.record.zoom, 1.0..=3.0, 0.25);
        self.ui.toggle("Debug overlay", &mut ctx.record.show_debug);
        let back = self.ui.button("Back") || self.ui.back();
        self.ui.end();
        if back {
            ctx.save();
            Transition::Pop
        } else {
            Transition::None
        }
    }

    fn draw(&mut self, _ctx: &mut Shared) {
        graphics::set_color(Color::rgba(0.03, 0.035, 0.06, 0.94));
        graphics::rectangle(DrawMode::Fill, (0.0, 0.0), (WIDTH, HEIGHT));
        self.ui.draw();
    }
}

// The pause menu, over the level.

struct Paused {
    ui: Ui,
}

impl Scene<Shared> for Paused {
    fn name(&self) -> &str {
        "paused"
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn update(&mut self, ctx: &mut Shared, _dt: f32) -> Transition<Shared> {
        self.ui.begin(menu_area(3.0), ctx.menu_input());
        let resume = self.ui.button("Resume") || self.ui.back();
        let settings = self.ui.button("Settings");
        let title = self.ui.button("Back to title");
        self.ui.end();
        if resume {
            Transition::Pop
        } else if settings {
            Transition::push(Settings { ui: Ui::new() })
        } else if title {
            Transition::reset(Title { ui: Ui::new() }).fade(FADE, BACKGROUND)
        } else {
            Transition::None
        }
    }

    fn draw(&mut self, _ctx: &mut Shared) {
        graphics::set_color(Color::rgba(0.0, 0.0, 0.0, 0.55));
        graphics::rectangle(DrawMode::Fill, (0.0, 0.0), (WIDTH, HEIGHT));
        title_text("PAUSED", 150.0);
        self.ui.draw();
    }
}

// The level.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Pose {
    Idle,
    Run,
    Jump,
}

enum Happened {
    Coin(Vec2),
}

struct Popup {
    at: Vec2,
    rise: Tween<f32>,
}

struct Play {
    camera: Camera,
    bounds: Rect,
    velocity: Vec2,
    since_ground: f32,
    facing_left: bool,
    pose: Animator<Pose>,
    coins: Pool<Vec2>,
    total: usize,
    happened: Events<Happened>,
    popups: Vec<Popup>,
    banner: Sequence<&'static str>,
    message: Option<&'static str>,
    seconds: f32,
}

impl Play {
    fn new(ctx: &Shared) -> Play {
        let map = ctx.level.map();
        let spawn = map
            .objects()
            .find(|object| object.class == "spawn")
            .map_or(Vec2::splat(40.0), |object| object.position);
        let coins: Pool<Vec2> = map
            .layer("coins")
            .map(|layer| {
                layer
                    .objects()
                    .iter()
                    .filter(|object| object.class == "coin")
                    .map(|coin| coin.position)
                    .collect()
            })
            .unwrap_or_default();
        let world = Rect::from_size(map.pixel_size());
        let mut camera = Camera::new((WIDTH, HEIGHT))
            .with_bounds(world)
            .with_follow(Follow {
                dead_zone: Vec2::new(24.0, 16.0),
                ..Follow::default()
            })
            .with_zoom_smoothing(0.15);
        camera.zoom = ctx.record.zoom;
        camera.zoom_to(ctx.record.zoom);
        camera.look_at(spawn);

        let mut pose = Animator::new(Pose::Idle, Animation::new([0, 1], 2.0));
        pose.add(Pose::Run, Animation::new([2, 3, 4, 3], 12.0))
            .add(Pose::Jump, Animation::new([5], 1.0));
        Play {
            camera,
            bounds: Rect::from_center_size(spawn, HERO_SIZE),
            velocity: Vec2::ZERO,
            since_ground: 0.0,
            facing_left: false,
            pose,
            total: coins.len(),
            coins,
            happened: Events::new(),
            popups: Vec::new(),
            banner: Sequence::new()
                .emit("Collect every coin!")
                .wait(2.0)
                .emit(""),
            message: None,
            seconds: 0.0,
        }
    }

    fn step(&mut self, ctx: &mut Shared, dt: f32) {
        let actions = &ctx.actions;
        let run = actions.axis(Act::Left, Act::Right);
        self.velocity.x = run * RUN_SPEED;
        if run != 0.0 {
            self.facing_left = run < 0.0;
        }
        self.velocity.y += GRAVITY * dt;
        self.since_ground += dt;
        if actions.is_pressed(Act::Jump) && self.since_ground < COYOTE {
            self.velocity.y = -JUMP_SPEED;
            self.since_ground = COYOTE;
        }
        // A short hop when jump is let go early.
        if actions.is_released(Act::Jump) && self.velocity.y < 0.0 {
            self.velocity.y *= 0.45;
        }

        let map = ctx.level.map();
        let slide = move_and_slide(self.bounds, self.velocity * dt, TILE, |cell| {
            map.is_solid(cell)
        });
        self.bounds = slide.bounds;
        if slide.contacts.bottom {
            self.since_ground = 0.0;
        }
        if slide.contacts.bottom || slide.contacts.top {
            self.velocity.y = 0.0;
        }

        let on_ground = self.since_ground == 0.0;
        self.pose.set(match (on_ground, run != 0.0) {
            (false, _) => Pose::Jump,
            (true, true) => Pose::Run,
            (true, false) => Pose::Idle,
        });
        self.pose.update(dt);

        let reach = Circle::new(self.bounds.center(), 9.0);
        let happened = &mut self.happened;
        self.coins.retain(|_, coin| {
            let taken = reach.contains(*coin);
            if taken {
                happened.send(Happened::Coin(*coin));
            }
            !taken
        });
    }
}

impl Scene<Shared> for Play {
    fn name(&self) -> &str {
        "play"
    }

    fn resume(&mut self, ctx: &mut Shared) {
        // Settings may have changed the zoom.
        self.camera.zoom_to(ctx.record.zoom);
    }

    fn update(&mut self, ctx: &mut Shared, dt: f32) -> Transition<Shared> {
        // Long frames (a window drag) are cut into short steps.
        let mut left = dt.min(0.1);
        while left > 0.0 {
            let step = left.min(1.0 / 120.0);
            self.step(ctx, step);
            left -= step;
        }
        self.seconds += dt;

        for happened in self.happened.drain() {
            match happened {
                Happened::Coin(at) => {
                    self.camera.add_trauma(0.25);
                    self.popups.push(Popup {
                        at,
                        rise: Tween::new(0.0, 18.0, 0.6, Ease::OutCubic),
                    });
                }
            }
        }
        self.popups.retain_mut(|popup| !popup.rise.update(dt));
        if let Some(message) = self.banner.update(dt).last() {
            self.message = (!message.is_empty()).then_some(message);
        }

        self.camera.follow(self.bounds.center());
        self.camera.update(dt);
        ctx.debug
            .watch("coins", format_args!("{} left", self.coins.len()));
        ctx.debug
            .watch("hero", format_args!("{:.0}", self.bounds.center()));
        ctx.debug
            .watch("pose", format_args!("{:?}", self.pose.state()));

        if self.coins.is_empty() {
            let best = ctx
                .record
                .best_seconds
                .map_or(self.seconds, |b| b.min(self.seconds));
            ctx.record.best_seconds = Some(best);
            ctx.save();
            return Transition::switch(Title { ui: Ui::new() }).fade(FADE, BACKGROUND);
        }
        if ctx.actions.is_pressed(Act::Back) {
            return Transition::push(Paused { ui: Ui::new() });
        }
        Transition::None
    }

    fn draw(&mut self, ctx: &mut Shared) {
        let view = self.camera.visible_rect();
        self.camera.draw(|| {
            graphics::set_color(Color::WHITE);
            ctx.level.draw_layer("ground", view);
            ctx.level.draw_layer("grass", view);
            for coin in self.coins.values() {
                graphics::set_color(Color::rgb(1.0, 0.82, 0.25));
                graphics::circle(DrawMode::Fill, *coin, 4.0);
                graphics::set_color(Color::rgb(1.0, 0.95, 0.7));
                graphics::circle(DrawMode::Fill, *coin - Vec2::new(1.0, 1.0), 1.5);
            }
            graphics::set_color(Color::WHITE);
            ctx.hero.draw_at(
                self.pose.frame(),
                self.bounds.center() - Vec2::new(0.0, 0.5),
                Flip::facing_left(self.facing_left),
            );
            for popup in &self.popups {
                let fade = 1.0 - popup.rise.progress();
                graphics::set_color(TEXT.with_alpha(fade));
                graphics::print("+1", popup.at - Vec2::new(5.0, 12.0 + popup.rise.value()));
            }
        });
        graphics::set_color(TEXT);
        let got = self.total - self.coins.len();
        graphics::print(
            format!("Coins {got}/{}   {:.1} s", self.total, self.seconds).as_str(),
            (20.0, 16.0),
        );
        if let Some(message) = self.message {
            title_text(message, 90.0);
        }
    }
}

// The level and its art, made in code.

/// The level as Tiled would save it: a ground layer, a grass layer, an
/// object layer of coins and a spawn point, and one embedded tileset whose
/// ground and brick tiles have `solid` set.
fn level_json() -> String {
    let (width, height) = (LEVEL[0].len(), LEVEL.len());
    let mut ground = Vec::new();
    let mut grass = Vec::new();
    let mut objects = Vec::new();
    for (y, row) in LEVEL.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            let below_ground = y > 0 && LEVEL[y - 1].as_bytes()[x] == b'#';
            ground.push(match c {
                '#' if below_ground => 4,
                '#' => 1,
                '=' => 2,
                _ => 0,
            });
            grass.push(if c == '*' { 3 } else { 0 });
            let center = ((x as f32 + 0.5) * TILE, (y as f32 + 0.5) * TILE);
            match c {
                'o' => objects.push(format!(
                    r#"{{"id":{},"name":"coin","type":"coin","x":{},"y":{},"point":true}}"#,
                    objects.len() + 2,
                    center.0,
                    center.1
                )),
                '@' => objects.push(format!(
                    r#"{{"id":1,"name":"hero","type":"spawn","x":{},"y":{},"point":true}}"#,
                    center.0, center.1
                )),
                _ => {}
            }
        }
    }
    let list = |cells: &[u32]| {
        cells
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    let solid = r#"[{"name":"solid","type":"bool","value":true}]"#;
    format!(
        r#"{{
  "width": {width}, "height": {height}, "tilewidth": 16, "tileheight": 16,
  "orientation": "orthogonal", "infinite": false,
  "layers": [
    {{ "type": "tilelayer", "name": "ground", "width": {width}, "height": {height}, "data": [{}] }},
    {{ "type": "tilelayer", "name": "grass", "width": {width}, "height": {height}, "data": [{}] }},
    {{ "type": "objectgroup", "name": "coins", "objects": [{}] }}
  ],
  "tilesets": [{{
    "firstgid": 1, "name": "tiles", "image": "tiles.png",
    "imagewidth": 64, "imageheight": 16, "tilewidth": 16, "tileheight": 16,
    "columns": 4, "tilecount": 4,
    "tiles": [{{ "id": 0, "properties": {solid} }}, {{ "id": 1, "properties": {solid} }},
              {{ "id": 3, "properties": {solid} }}]
  }}]
}}"#,
        list(&ground),
        list(&grass),
        objects.join(",")
    )
}

/// Paints an image from a function of each pixel.
fn paint(width: u32, height: u32, color: impl Fn(u32, u32) -> Option<Color>) -> Image {
    let mut data = ImageData::new(width, height);
    data.map_pixels(|x, y, _| color(x, y).unwrap_or(Color::rgba(0.0, 0.0, 0.0, 0.0)));
    let image = Image::from_image_data(&data).expect("an image made in memory loads");
    image.set_filter(smitten::Filter::Nearest);
    image
}

/// Grassy ground, brick, a tuft of grass and plain earth, 16 pixels each.
fn tileset_image() -> Image {
    paint(64, 16, |x, y| {
        let (tile, x) = (x / 16, x % 16);
        match tile {
            0 | 3 => Some(if y < 3 && tile == 0 {
                Color::rgb(0.35, 0.75, 0.35)
            } else if (x * 7 + y * 3) % 11 == 0 {
                Color::rgb(0.35, 0.24, 0.18)
            } else {
                Color::rgb(0.45, 0.31, 0.22)
            }),
            1 => {
                let offset = if (y / 4) % 2 == 0 { 0 } else { 4 };
                let mortar = y % 4 == 3 || (x + offset) % 8 == 7;
                Some(if mortar {
                    Color::rgb(0.25, 0.25, 0.32)
                } else {
                    Color::rgb(0.55, 0.42, 0.5)
                })
            }
            2 => {
                let blade = (x == 4 && y > 9) || (x == 7 && y > 7) || (x == 10 && y > 10);
                blade.then(|| Color::rgb(0.4, 0.85, 0.4))
            }
            _ => None,
        }
    })
}

/// The hero: 6 frames of 12 by 16 (two idle, three running, one jumping).
fn hero_sheet() -> SpriteSheet {
    let body = Color::rgb(0.95, 0.55, 0.3);
    let dark = Color::rgb(0.2, 0.15, 0.25);
    let image = paint(72, 16, |x, y| {
        let (frame, x) = (x / 12, x % 12);
        let breathe = u32::from(frame == 1);
        // Head and body.
        if (3..9).contains(&x) && (1 + breathe..11).contains(&y) {
            return Some(if y == 4 && (x == 7 || x == 5) {
                dark
            } else {
                body
            });
        }
        // Legs, moving apart while running and tucked while jumping.
        let (left, right) = match frame {
            2 => (3, 8),
            3 => (4, 7),
            4 => (5, 6),
            5 => (2, 9),
            _ => (4, 7),
        };
        let leg_end = if frame == 5 { 14 } else { 16 };
        ((x == left || x == right) && (11..leg_end).contains(&y)).then_some(dark)
    });
    SpriteSheet::grid(image, 12, 16)
}

fn main() {
    let config = Config {
        title: "smitten-kit tour".to_string(),
        width: WIDTH as u32,
        height: HEIGHT as u32,
        ..Config::default()
    };
    scene::run(config, || {
        let map = TileMap::from_json(&level_json()).expect("the level is a valid map");
        let tiles = tileset_image();
        let level = TiledMap::with_images(map, |_| Ok::<_, ()>(Some(tiles.clone())))
            .expect("the images are made in memory");
        let slots = SaveSlots::new("kit-tour", 1).ok();
        let record = slots
            .as_ref()
            .and_then(|slots| slots.load("record"))
            .unwrap_or_default();
        let shared = Shared {
            actions: controls(),
            debug: DebugOverlay::new(),
            slots,
            record,
            level,
            hero: hero_sheet(),
        };
        Ok((shared, Title { ui: Ui::new() }))
    });
}
