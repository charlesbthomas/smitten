//! Pong, for one or two players, built on smitten-kit.
//!
//! - On the title screen, press 1 to play the computer or 2 for two players.
//! - The left paddle moves with W and S, the right one with the Up and Down
//!   arrows. Gamepads work too: the first one plays left, the second right.
//! - Space, Return or a gamepad's A button serves. First to 11 wins.
//! - Escape pauses a match; Escape again goes back to the title screen, and
//!   quits from there.
//!
//! The kit shapes the game: the title, the match and the pause menu are
//! scenes on a [`SceneStack`], each player's controls are named
//! [`Actions`], and a [`Camera`] shakes the court on hits.

use smitten::graphics::{Align, DrawMode, Font};
use smitten::{Color, Config, GamepadAxis, GamepadButton, Key, joystick};
use smitten_kit::prelude::*;

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;

const PADDLE_WIDTH: f32 = 12.0;
const PADDLE_HEIGHT: f32 = 90.0;
const PADDLE_MARGIN: f32 = 30.0;
const PADDLE_SPEED: f32 = 480.0;
const AI_SPEED: f32 = 330.0;
/// How far across the court, as a fraction of its width, the ball must come
/// before the computer starts chasing it.
const AI_REACTION: f32 = 0.45;

const BALL_SIZE: f32 = 12.0;
const SERVE_SPEED: f32 = 360.0;
const MAX_BALL_SPEED: f32 = 950.0;
const SPEEDUP: f32 = 1.06;
/// The steepest a returned ball can leave a paddle, hit on its very edge.
const MAX_BOUNCE_ANGLE: f32 = std::f32::consts::FRAC_PI_3;
/// The longest a single physics step moves the ball, so a fast ball cannot
/// pass through a paddle between frames.
const MAX_STEP: f32 = 4.0;

const WINNING_SCORE: u32 = 11;
/// Seconds for each half of the fade between the title and a match.
const FADE: f32 = 0.2;

const BACKGROUND: Color = Color::rgb(0.07, 0.08, 0.1);
const FOREGROUND: Color = Color::rgb(0.92, 0.93, 0.95);
const DIM: Color = Color::rgba(0.92, 0.93, 0.95, 0.25);

/// What a player can do with a paddle.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Move {
    Up,
    Down,
}

/// What any player can do on the menus.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Menu {
    OnePlayer,
    TwoPlayers,
    Confirm,
    Back,
}

fn paddle_controls(up: Key, down: Key) -> Actions<Move> {
    let mut actions = Actions::new();
    // No gamepad until `Shared::assign_gamepads` hands one over.
    actions.set_gamepad(None);
    actions
        .bind(
            Move::Up,
            [
                Binding::Key(up),
                Binding::Button(GamepadButton::DpadUp),
                Binding::stick_negative(GamepadAxis::LeftY),
            ],
        )
        .bind(
            Move::Down,
            [
                Binding::Key(down),
                Binding::Button(GamepadButton::DpadDown),
                Binding::stick_positive(GamepadAxis::LeftY),
            ],
        );
    actions
}

fn menu_controls() -> Actions<Menu> {
    let mut actions = Actions::new();
    actions
        // A gamepad on its own can still start a match: A plays the
        // computer, X starts two players.
        .bind(
            Menu::OnePlayer,
            [Binding::Key(Key::Num1), GamepadButton::A.into()],
        )
        .bind(
            Menu::TwoPlayers,
            [Binding::Key(Key::Num2), GamepadButton::X.into()],
        )
        .bind(
            Menu::Confirm,
            [
                Binding::Key(Key::Space),
                Key::Return.into(),
                GamepadButton::A.into(),
                GamepadButton::Start.into(),
            ],
        )
        .bind(
            Menu::Back,
            [
                Binding::Key(Key::Escape),
                GamepadButton::B.into(),
                GamepadButton::Back.into(),
            ],
        );
    actions
}

/// What lasts for the whole game, shared by every scene.
struct Shared {
    menu: Actions<Menu>,
    /// The left and right players' paddles.
    paddles: [Actions<Move>; 2],
    seed: u32,
    score_font: Font,
    title_font: Font,
    text_font: Font,
}

impl Shared {
    /// A small deterministic random number from 0.0 to 1.0.
    fn random(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// The first gamepad plays left and the second right. Hands each its
    /// pad when the set of connected pads changes.
    fn assign_gamepads(&mut self) {
        let pads = joystick::gamepads();
        for (side, actions) in self.paddles.iter_mut().enumerate() {
            let pad = pads.get(side).copied();
            if actions.gamepad().copied() != pad {
                actions.set_gamepad(pad);
            }
        }
    }
}

impl scene::Context for Shared {
    fn event(&mut self, event: &Event) {
        self.menu.handle(event);
        for actions in &mut self.paddles {
            actions.handle(event);
        }
    }

    fn update(&mut self, dt: f32) {
        self.assign_gamepads();
        self.menu.update(dt);
        for actions in &mut self.paddles {
            actions.update(dt);
        }
    }

    fn background(&self) -> Color {
        BACKGROUND
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

impl Side {
    fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// Which way the ball travels along x when this side serves.
    fn direction(self) -> f32 {
        match self {
            Side::Left => 1.0,
            Side::Right => -1.0,
        }
    }
}

struct Paddle {
    /// The paddle's left edge.
    x: f32,
    /// The paddle's center.
    y: f32,
    score: u32,
}

impl Paddle {
    fn new(x: f32) -> Paddle {
        Paddle {
            x,
            y: HEIGHT / 2.0,
            score: 0,
        }
    }

    fn top(&self) -> f32 {
        self.y - PADDLE_HEIGHT / 2.0
    }

    fn move_by(&mut self, dy: f32) {
        let half = PADDLE_HEIGHT / 2.0;
        self.y = (self.y + dy).clamp(half, HEIGHT - half);
    }

    fn draw(&self) {
        graphics::rectangle(
            DrawMode::Fill,
            (self.x, self.top()),
            (PADDLE_WIDTH, PADDLE_HEIGHT),
        );
    }
}

struct Ball {
    /// The ball's center.
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

impl Ball {
    fn centered() -> Ball {
        Ball {
            x: WIDTH / 2.0,
            y: HEIGHT / 2.0,
            vx: 0.0,
            vy: 0.0,
        }
    }

    fn speed(&self) -> f32 {
        self.vx.hypot(self.vy)
    }

    /// Whether the ball overlaps the paddle.
    fn touches(&self, paddle: &Paddle) -> bool {
        let half = BALL_SIZE / 2.0;
        self.x + half > paddle.x
            && self.x - half < paddle.x + PADDLE_WIDTH
            && self.y + half > paddle.top()
            && self.y - half < paddle.top() + PADDLE_HEIGHT
    }

    /// Sends the ball back off `paddle`, steeper the further from the
    /// paddle's center it hit, and a little faster each time.
    fn bounce_off(&mut self, paddle: &Paddle, direction: f32) {
        let offset = ((self.y - paddle.y) / (PADDLE_HEIGHT / 2.0)).clamp(-1.0, 1.0);
        let angle = offset * MAX_BOUNCE_ANGLE;
        let speed = (self.speed() * SPEEDUP).min(MAX_BALL_SPEED);
        self.vx = direction * speed * angle.cos();
        self.vy = speed * angle.sin();
        // Move clear of the paddle so it is not hit twice.
        self.x = if direction > 0.0 {
            paddle.x + PADDLE_WIDTH + BALL_SIZE / 2.0
        } else {
            paddle.x - BALL_SIZE / 2.0
        };
    }

    fn draw(&self) {
        let half = BALL_SIZE / 2.0;
        graphics::rectangle(
            DrawMode::Fill,
            (self.x - half, self.y - half),
            (BALL_SIZE, BALL_SIZE),
        );
    }
}

/// The title screen.
struct Title;

impl Scene<Shared> for Title {
    fn update(&mut self, ctx: &mut Shared, _dt: f32) -> Transition<Shared> {
        if ctx.menu.is_pressed(Menu::OnePlayer) {
            Transition::switch(Match::new(ctx, false)).fade(FADE, BACKGROUND)
        } else if ctx.menu.is_pressed(Menu::TwoPlayers) {
            Transition::switch(Match::new(ctx, true)).fade(FADE, BACKGROUND)
        } else if ctx.menu.is_pressed(Menu::Back) {
            Transition::Quit
        } else {
            Transition::None
        }
    }

    fn draw(&mut self, ctx: &mut Shared) {
        graphics::set_color(FOREGROUND);
        graphics::set_font(&ctx.title_font);
        graphics::printf("PONG", (0.0, 150.0), WIDTH, Align::Center);

        graphics::set_font(&ctx.text_font);
        graphics::printf("1   Play the computer", (0.0, 320.0), WIDTH, Align::Center);
        graphics::printf("2   Two players", (0.0, 360.0), WIDTH, Align::Center);

        graphics::set_color(DIM);
        graphics::printf(
            "Left: W / S     Right: Up / Down",
            (0.0, 480.0),
            WIDTH,
            Align::Center,
        );
        graphics::printf(
            "Gamepad: A plays the computer, X starts two players",
            (0.0, 510.0),
            WIDTH,
            Align::Center,
        );
        graphics::printf("Escape quits", (0.0, 540.0), WIDTH, Align::Center);
    }
}

/// Where a match is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for `server` to serve.
    Serve {
        server: Side,
    },
    Playing,
    Won {
        winner: Side,
    },
}

/// A match in play, from the first serve to the winning point.
struct Match {
    phase: Phase,
    two_players: bool,
    left: Paddle,
    right: Paddle,
    ball: Ball,
    /// How far from its paddle's center the computer means to hit the
    /// ball, so its returns are not all flat.
    ai_aim: f32,
    /// Shakes the court a little on each hit and more on each point.
    camera: Camera,
}

impl Match {
    fn new(ctx: &mut Shared, two_players: bool) -> Match {
        let server = if ctx.random() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        Match {
            phase: Phase::Serve { server },
            two_players,
            left: Paddle::new(PADDLE_MARGIN),
            right: Paddle::new(WIDTH - PADDLE_MARGIN - PADDLE_WIDTH),
            ball: Ball::centered(),
            ai_aim: 0.0,
            camera: Camera::new((WIDTH, HEIGHT)).with_shake(Shake {
                max_offset: 10.0,
                max_angle: 0.01,
                drain: 2.5,
                frequency: 30.0,
            }),
        }
    }

    fn serve(&mut self, ctx: &mut Shared, server: Side) {
        // Aim somewhere within 30 degrees of straight across.
        let angle = (ctx.random() - 0.5) * std::f32::consts::FRAC_PI_3;
        self.ball = Ball::centered();
        self.ball.vx = server.direction() * SERVE_SPEED * angle.cos();
        self.ball.vy = SERVE_SPEED * angle.sin();
        self.choose_ai_aim(ctx);
        self.phase = Phase::Playing;
    }

    fn update_paddles(&mut self, ctx: &Shared, dt: f32) {
        let [left, right] = &ctx.paddles;
        self.left
            .move_by(left.axis(Move::Up, Move::Down) * PADDLE_SPEED * dt);

        if self.two_players {
            self.right
                .move_by(right.axis(Move::Up, Move::Down) * PADDLE_SPEED * dt);
        } else {
            let target = self.ai_target();
            let step = AI_SPEED * dt;
            let dy = (target - self.right.y).clamp(-step, step);
            self.right.move_by(dy);
        }
    }

    /// Where the computer's paddle wants its center to be. Once the ball
    /// is coming and has crossed into its half, it lines up so the ball
    /// lands `ai_aim` off center, which angles the return. Otherwise it
    /// drifts back toward the middle.
    fn ai_target(&self) -> f32 {
        if self.phase == Phase::Playing && self.ball.vx > 0.0 && self.ball.x > WIDTH * AI_REACTION {
            self.ball.y - self.ai_aim
        } else {
            HEIGHT / 2.0
        }
    }

    /// Picks where on its paddle the computer will try to take the next
    /// ball.
    fn choose_ai_aim(&mut self, ctx: &mut Shared) {
        self.ai_aim = (ctx.random() * 2.0 - 1.0) * PADDLE_HEIGHT * 0.4;
    }

    fn update_ball(&mut self, ctx: &mut Shared, dt: f32) {
        let distance = self.ball.speed() * dt;
        let steps = (distance / MAX_STEP).ceil().max(1.0) as u32;
        let step_dt = dt / steps as f32;

        for _ in 0..steps {
            self.ball.x += self.ball.vx * step_dt;
            self.ball.y += self.ball.vy * step_dt;

            let half = BALL_SIZE / 2.0;
            if self.ball.y < half {
                self.ball.y = half;
                self.ball.vy = self.ball.vy.abs();
            } else if self.ball.y > HEIGHT - half {
                self.ball.y = HEIGHT - half;
                self.ball.vy = -self.ball.vy.abs();
            }

            if self.ball.vx < 0.0 && self.ball.touches(&self.left) {
                self.ball.bounce_off(&self.left, 1.0);
                self.camera.add_trauma(0.3);
                self.choose_ai_aim(ctx);
            } else if self.ball.vx > 0.0 && self.ball.touches(&self.right) {
                self.ball.bounce_off(&self.right, -1.0);
                self.camera.add_trauma(0.3);
            }

            if self.ball.x < -BALL_SIZE {
                self.point_to(Side::Right);
                return;
            }
            if self.ball.x > WIDTH + BALL_SIZE {
                self.point_to(Side::Left);
                return;
            }
        }
    }

    fn point_to(&mut self, scorer: Side) {
        let paddle = match scorer {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        };
        paddle.score += 1;
        self.ball = Ball::centered();
        self.camera.add_trauma(0.6);
        self.phase = if paddle.score >= WINNING_SCORE {
            Phase::Won { winner: scorer }
        } else {
            // The player who lost the point serves the next one.
            Phase::Serve {
                server: scorer.other(),
            }
        };
    }

    fn side_name(&self, side: Side) -> &'static str {
        match (side, self.two_players) {
            (Side::Left, false) => "You",
            (Side::Right, false) => "The computer",
            (Side::Left, true) => "Left player",
            (Side::Right, true) => "Right player",
        }
    }

    fn possessive(&self, side: Side) -> &'static str {
        match (side, self.two_players) {
            (Side::Left, false) => "Your",
            (Side::Right, false) => "The computer's",
            (Side::Left, true) => "Left player's",
            (Side::Right, true) => "Right player's",
        }
    }

    fn draw_court(&self, ctx: &Shared) {
        graphics::set_color(DIM);
        let dash = 18.0;
        let mut y = 6.0;
        while y < HEIGHT {
            graphics::rectangle(DrawMode::Fill, (WIDTH / 2.0 - 2.0, y), (4.0, dash));
            y += dash * 2.0;
        }

        graphics::set_color(FOREGROUND);
        graphics::set_font(&ctx.score_font);
        let column = WIDTH / 2.0 - 60.0;
        graphics::printf(
            &self.left.score.to_string(),
            (0.0, 30.0),
            column,
            Align::Right,
        );
        graphics::printf(
            &self.right.score.to_string(),
            (WIDTH / 2.0 + 60.0, 30.0),
            column,
            Align::Left,
        );

        self.left.draw();
        self.right.draw();
    }
}

fn draw_message(ctx: &Shared, headline: &str, detail: &str) {
    // Blank out the center line behind the text.
    graphics::set_color(BACKGROUND);
    graphics::rectangle(
        DrawMode::Fill,
        (WIDTH / 2.0 - 4.0, HEIGHT - 160.0),
        (8.0, 80.0),
    );
    graphics::set_color(FOREGROUND);
    graphics::set_font(&ctx.text_font);
    graphics::printf(headline, (0.0, HEIGHT - 150.0), WIDTH, Align::Center);
    graphics::set_color(DIM);
    graphics::printf(detail, (0.0, HEIGHT - 115.0), WIDTH, Align::Center);
}

impl Scene<Shared> for Match {
    fn update(&mut self, ctx: &mut Shared, dt: f32) -> Transition<Shared> {
        // A long stall, such as dragging the window, should not fling the
        // ball across the court in one frame.
        let dt = dt.min(1.0 / 20.0);
        self.camera.update(dt);
        self.update_paddles(ctx, dt);
        match self.phase {
            Phase::Serve { server } if ctx.menu.is_pressed(Menu::Confirm) => {
                self.serve(ctx, server);
            }
            Phase::Playing => self.update_ball(ctx, dt),
            Phase::Won { .. } if ctx.menu.is_pressed(Menu::Confirm) => {
                return Transition::switch(Match::new(ctx, self.two_players));
            }
            _ => {}
        }
        if ctx.menu.is_pressed(Menu::Back) {
            return Transition::push(Paused);
        }
        Transition::None
    }

    fn draw(&mut self, ctx: &mut Shared) {
        self.camera.draw(|| {
            self.draw_court(ctx);
            if !matches!(self.phase, Phase::Won { .. }) {
                self.ball.draw();
            }
        });
        match self.phase {
            Phase::Serve { server } => draw_message(
                ctx,
                &format!("{} serve", self.possessive(server)),
                "Press Space or A",
            ),
            Phase::Playing => {}
            Phase::Won { winner } => {
                let verb = if !self.two_players && winner == Side::Left {
                    "win"
                } else {
                    "wins"
                };
                draw_message(
                    ctx,
                    &format!("{} {verb}!", self.side_name(winner)),
                    "Press Space to play again, or Escape for the title screen",
                );
            }
        }
    }
}

/// The pause menu, drawn over the frozen match.
struct Paused;

impl Scene<Shared> for Paused {
    fn is_overlay(&self) -> bool {
        true
    }

    fn update(&mut self, ctx: &mut Shared, _dt: f32) -> Transition<Shared> {
        if ctx.menu.is_pressed(Menu::Confirm) {
            Transition::Pop
        } else if ctx.menu.is_pressed(Menu::Back) {
            Transition::reset(Title).fade(FADE, BACKGROUND)
        } else {
            Transition::None
        }
    }

    fn draw(&mut self, ctx: &mut Shared) {
        graphics::set_color(BACKGROUND.with_alpha(0.75));
        graphics::rectangle(DrawMode::Fill, (0.0, 0.0), (WIDTH, HEIGHT));
        graphics::set_color(FOREGROUND);
        graphics::set_font(&ctx.text_font);
        graphics::printf("PAUSED", (0.0, HEIGHT / 2.0 - 40.0), WIDTH, Align::Center);
        graphics::set_color(DIM);
        graphics::printf(
            "Space to carry on, Escape for the title screen",
            (0.0, HEIGHT / 2.0),
            WIDTH,
            Align::Center,
        );
    }
}

fn main() {
    let config = Config {
        title: "Pong".to_string(),
        width: WIDTH as u32,
        height: HEIGHT as u32,
        // The court is laid out for exactly this size, so let Smitten
        // scale it to whatever size the window is given.
        resizable: true,
        design_size: Some((WIDTH as u32, HEIGHT as u32)),
        ..Config::default()
    };
    scene::run(config, || {
        let shared = Shared {
            menu: menu_controls(),
            paddles: [
                paddle_controls(Key::W, Key::S),
                paddle_controls(Key::Up, Key::Down),
            ],
            seed: 2024,
            score_font: Font::default_at(64.0),
            title_font: Font::default_at(96.0),
            text_font: Font::default_at(22.0),
        };
        Ok((shared, Title))
    });
}
