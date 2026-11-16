//! Pong, for one or two players.
//!
//! - On the title screen, press 1 to play the computer or 2 for two players.
//! - The left paddle moves with W and S, the right one with the Up and Down
//!   arrows. Gamepads work too: the first one plays left, the second right.
//! - Space, Return or a gamepad's A button serves. First to 11 wins.
//! - Escape goes back to the title screen, and quits from there.

use smitten::graphics::{Align, DrawMode, Font};
use smitten::{
    Color, Config, Game, GamepadAxis, GamepadButton, Joystick, Key, event, graphics, joystick,
    keyboard,
};

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
const STICK_DEADZONE: f32 = 0.25;

const BACKGROUND: Color = Color::rgb(0.07, 0.08, 0.1);
const FOREGROUND: Color = Color::rgb(0.92, 0.93, 0.95);
const DIM: Color = Color::rgba(0.92, 0.93, 0.95, 0.25);

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Title,
    /// Waiting for `server` to serve.
    Serve {
        server: Side,
    },
    Playing,
    Won {
        winner: Side,
    },
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
            self.x,
            self.top(),
            PADDLE_WIDTH,
            PADDLE_HEIGHT,
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
            self.x - half,
            self.y - half,
            BALL_SIZE,
            BALL_SIZE,
        );
    }
}

struct Pong {
    state: State,
    two_players: bool,
    left: Paddle,
    right: Paddle,
    ball: Ball,
    /// How far from its paddle's center the computer means to hit the
    /// ball, so its returns are not all flat.
    ai_aim: f32,
    seed: u32,
    score_font: Font,
    title_font: Font,
    text_font: Font,
}

impl Pong {
    /// A small deterministic random number from 0.0 to 1.0, enough for a
    /// demo until `smitten::math` exists.
    fn random(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    fn start_match(&mut self, two_players: bool) {
        self.two_players = two_players;
        self.left = Paddle::new(PADDLE_MARGIN);
        self.right = Paddle::new(WIDTH - PADDLE_MARGIN - PADDLE_WIDTH);
        self.ball = Ball::centered();
        let server = if self.random() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        self.state = State::Serve { server };
    }

    fn serve(&mut self, server: Side) {
        // Aim somewhere within 30 degrees of straight across.
        let angle = (self.random() - 0.5) * std::f32::consts::FRAC_PI_3;
        self.ball = Ball::centered();
        self.ball.vx = server.direction() * SERVE_SPEED * angle.cos();
        self.ball.vy = SERVE_SPEED * angle.sin();
        self.choose_ai_aim();
        self.state = State::Playing;
    }

    /// Whatever confirms on the current screen: serving, or starting a new
    /// match after one is won.
    fn confirm(&mut self) {
        match self.state {
            State::Title => {}
            State::Serve { server } => self.serve(server),
            State::Playing => {}
            State::Won { .. } => self.start_match(self.two_players),
        }
    }

    fn back(&mut self) {
        if self.state == State::Title {
            event::quit();
        } else {
            self.state = State::Title;
        }
    }

    fn update_paddles(&mut self, dt: f32) {
        let pads = joystick::joysticks();
        let left = control(&[Key::W], &[Key::S], pads.first());
        self.left.move_by(left * PADDLE_SPEED * dt);

        if self.two_players {
            let right = control(&[Key::Up], &[Key::Down], pads.get(1));
            self.right.move_by(right * PADDLE_SPEED * dt);
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
        if self.state == State::Playing && self.ball.vx > 0.0 && self.ball.x > WIDTH * AI_REACTION {
            self.ball.y - self.ai_aim
        } else {
            HEIGHT / 2.0
        }
    }

    /// Picks where on its paddle the computer will try to take the next
    /// ball.
    fn choose_ai_aim(&mut self) {
        self.ai_aim = (self.random() * 2.0 - 1.0) * PADDLE_HEIGHT * 0.4;
    }

    fn update_ball(&mut self, dt: f32) {
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
                self.choose_ai_aim();
            } else if self.ball.vx > 0.0 && self.ball.touches(&self.right) {
                self.ball.bounce_off(&self.right, -1.0);
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
        self.state = if paddle.score >= WINNING_SCORE {
            State::Won { winner: scorer }
        } else {
            // The player who lost the point serves the next one.
            State::Serve {
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

    fn draw_court(&self) {
        graphics::set_color(DIM);
        let dash = 18.0;
        let mut y = 6.0;
        while y < HEIGHT {
            graphics::rectangle(DrawMode::Fill, WIDTH / 2.0 - 2.0, y, 4.0, dash);
            y += dash * 2.0;
        }

        graphics::set_color(FOREGROUND);
        graphics::set_font(&self.score_font);
        let column = WIDTH / 2.0 - 60.0;
        graphics::printf(
            &self.left.score.to_string(),
            0.0,
            30.0,
            column,
            Align::Right,
        );
        graphics::printf(
            &self.right.score.to_string(),
            WIDTH / 2.0 + 60.0,
            30.0,
            column,
            Align::Left,
        );

        self.left.draw();
        self.right.draw();
    }

    fn possessive(&self, side: Side) -> &'static str {
        match (side, self.two_players) {
            (Side::Left, false) => "Your",
            (Side::Right, false) => "The computer's",
            (Side::Left, true) => "Left player's",
            (Side::Right, true) => "Right player's",
        }
    }

    fn draw_message(&self, headline: &str, detail: &str) {
        // Blank out the center line behind the text.
        graphics::set_color(BACKGROUND);
        graphics::rectangle(DrawMode::Fill, WIDTH / 2.0 - 4.0, HEIGHT - 160.0, 8.0, 80.0);
        graphics::set_color(FOREGROUND);
        graphics::set_font(&self.text_font);
        graphics::printf(headline, 0.0, HEIGHT - 150.0, WIDTH, Align::Center);
        graphics::set_color(DIM);
        graphics::printf(detail, 0.0, HEIGHT - 115.0, WIDTH, Align::Center);
    }

    fn draw_title(&self) {
        graphics::set_color(FOREGROUND);
        graphics::set_font(&self.title_font);
        graphics::printf("PONG", 0.0, 150.0, WIDTH, Align::Center);

        graphics::set_font(&self.text_font);
        graphics::printf("1   Play the computer", 0.0, 320.0, WIDTH, Align::Center);
        graphics::printf("2   Two players", 0.0, 360.0, WIDTH, Align::Center);

        graphics::set_color(DIM);
        graphics::printf(
            "Left: W / S     Right: Up / Down",
            0.0,
            480.0,
            WIDTH,
            Align::Center,
        );
        graphics::printf(
            "Gamepad: A plays the computer, X starts two players",
            0.0,
            510.0,
            WIDTH,
            Align::Center,
        );
        graphics::printf("Escape quits", 0.0, 540.0, WIDTH, Align::Center);
    }
}

/// How the player wants to move a paddle, from -1.0 (up) to 1.0 (down).
fn control(up: &[Key], down: &[Key], pad: Option<&Joystick>) -> f32 {
    let mut dy = 0.0;
    if keyboard::is_any_down(up) {
        dy -= 1.0;
    }
    if keyboard::is_any_down(down) {
        dy += 1.0;
    }
    if let Some(pad) = pad {
        if pad.is_gamepad_down(GamepadButton::DpadUp) {
            dy -= 1.0;
        }
        if pad.is_gamepad_down(GamepadButton::DpadDown) {
            dy += 1.0;
        }
        let stick = pad.gamepad_axis(GamepadAxis::LeftY);
        if stick.abs() > STICK_DEADZONE {
            dy += stick;
        }
    }
    dy.clamp(-1.0, 1.0)
}

impl Game for Pong {
    fn load() -> Self {
        graphics::set_background_color(BACKGROUND);
        Pong {
            state: State::Title,
            two_players: false,
            left: Paddle::new(PADDLE_MARGIN),
            right: Paddle::new(WIDTH - PADDLE_MARGIN - PADDLE_WIDTH),
            ball: Ball::centered(),
            ai_aim: 0.0,
            seed: 2024,
            score_font: graphics::new_default_font(64.0),
            title_font: graphics::new_default_font(96.0),
            text_font: graphics::new_default_font(22.0),
        }
    }

    fn update(&mut self, dt: f32) {
        // A long stall, such as dragging the window, should not fling the
        // ball across the court in one frame.
        let dt = dt.min(1.0 / 20.0);
        match self.state {
            State::Title => {}
            State::Serve { .. } | State::Won { .. } => self.update_paddles(dt),
            State::Playing => {
                self.update_paddles(dt);
                self.update_ball(dt);
            }
        }
    }

    fn draw(&mut self) {
        match self.state {
            State::Title => self.draw_title(),
            State::Serve { server } => {
                self.draw_court();
                self.ball.draw();
                self.draw_message(
                    &format!("{} serve", self.possessive(server)),
                    "Press Space or A",
                );
            }
            State::Playing => {
                self.draw_court();
                self.ball.draw();
            }
            State::Won { winner } => {
                self.draw_court();
                let verb = if !self.two_players && winner == Side::Left {
                    "win"
                } else {
                    "wins"
                };
                self.draw_message(
                    &format!("{} {verb}!", self.side_name(winner)),
                    "Press Space to play again, or Escape for the title screen",
                );
            }
        }
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        if is_repeat {
            return;
        }
        match key {
            Key::Num1 if self.state == State::Title => self.start_match(false),
            Key::Num2 if self.state == State::Title => self.start_match(true),
            Key::Space | Key::Return => self.confirm(),
            Key::Escape => self.back(),
            _ => {}
        }
    }

    fn gamepad_pressed(&mut self, _joystick: Joystick, button: GamepadButton) {
        match button {
            // A gamepad on its own can still start a match: A plays the
            // computer, X starts two players.
            GamepadButton::A if self.state == State::Title => self.start_match(false),
            GamepadButton::X if self.state == State::Title => self.start_match(true),
            GamepadButton::A | GamepadButton::Start => self.confirm(),
            GamepadButton::B | GamepadButton::Back => self.back(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Pong>(Config {
        title: "Pong".to_string(),
        width: WIDTH as u32,
        height: HEIGHT as u32,
        ..Config::default()
    });
}
