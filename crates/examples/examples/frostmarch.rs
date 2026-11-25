//! Frostmarch: a proof of concept for fast melee combat.
//!
//! One fighter on a snowfield against waves of wights, brutes and casters.
//! The fighter is not a sprite. It is a small skeleton solved every frame,
//! with a three-point spine, legs and arms bent by two-bone inverse
//! kinematics, and a cape that trails as a chain. Press B to see the bones.
//!
//! There are three attacks, light, medium and heavy, and four weapons that
//! each turn them into different moves. Up to four attacks chain into a
//! string, a heavy attack ends it, and each weapon has four named strings
//! that finish harder. The list at the bottom left shows them.
//!
//! Enemies paint where they are about to hit on the ground before they
//! strike. A dash passes through anything, and pressing dash again just as
//! one ends chains another for free. An attack out of a dash hits harder,
//! and a swing that meets a caster's bolt sends it back.
//!
//! With a keyboard and mouse, W, A, S and D move and the mouse aims. The
//! left button is the light attack, the right button the medium, and the
//! middle button, F, or Shift with the left button the heavy. J, K and L
//! attack too. Space dashes, and Q, E or 1 to 4 change weapon.
//!
//! On a gamepad, the left stick moves and the right stick aims. X, Y and B
//! are the light, medium and heavy attacks, A dashes, and the shoulder
//! buttons change weapon.
//!
//! Tab, or Back on a gamepad, lets the game play itself. Run it with `demo`
//! to start that way:
//!
//! ```text
//! cargo run --example frostmarch
//! cargo run --example frostmarch -- demo
//! ```

use std::f32::consts::{PI, TAU};

use smitten::audio::Source;
use smitten::graphics::{Align, DrawMode, DrawParams, Font, Image, ImageData, LineStyle};
use smitten::math::{self, RandomGenerator};
use smitten::sound::SoundData;
use smitten::{
    Color, Config, Filter, Game, GamepadButton, Joystick, Key, MouseButton, MouseClick, Scaling,
    Vec2, event, graphics, joystick, keyboard, mouse, window,
};

const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 270.0;
/// The part of the snowfield the fighters can stand on.
const ARENA_MIN: Vec2 = Vec2::new(14.0, 50.0);
const ARENA_MAX: Vec2 = Vec2::new(466.0, 256.0);

const WALK_SPEED: f32 = 96.0;
const DASH_SPEED: f32 = 360.0;
const DASH_TIME: f32 = 0.16;
/// How long a press is remembered, so a button pressed a little early
/// still counts.
const BUFFER_TIME: f32 = 0.2;
const MAX_HEALTH: i32 = 6;
const MAX_STAMINA: f32 = 3.0;
/// How high above its feet a fighter's chest is, where swings are drawn.
const CHEST: Vec2 = Vec2::new(0.0, -11.0);

fn hex(value: u32) -> Color {
    Color::from_rgb8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

fn ease(value: f32) -> f32 {
    1.0 - (1.0 - value.clamp(0.0, 1.0)).powi(3)
}

/// Brings an angle into the range from -PI to PI.
fn wrap(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

// The three attacks and the four weapons.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Light,
    Medium,
    Heavy,
}

impl Kind {
    fn letter(self) -> &'static str {
        match self {
            Kind::Light => "L",
            Kind::Medium => "M",
            Kind::Heavy => "H",
        }
    }

    fn color(self) -> Color {
        match self {
            Kind::Light => hex(0xffffff),
            Kind::Medium => hex(0xff5c8d),
            Kind::Heavy => hex(0xffb86c),
        }
    }

    /// How long the whole game freezes when this attack lands.
    fn hit_stop(self) -> f32 {
        match self {
            Kind::Light => 0.035,
            Kind::Medium => 0.06,
            Kind::Heavy => 0.11,
        }
    }

    fn recovery(self) -> f32 {
        match self {
            Kind::Light => 0.09,
            Kind::Medium => 0.13,
            Kind::Heavy => 0.2,
        }
    }

    fn shake(self) -> f32 {
        match self {
            Kind::Light => 0.16,
            Kind::Medium => 0.28,
            Kind::Heavy => 0.5,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// A sweep from one angle to another.
    Arc,
    /// Both hands sweep in from the sides at once.
    Cross,
    /// A full turn that hits all around.
    Spin,
    /// A straight line ahead.
    Thrust,
}

/// One attack of one weapon. Angles are measured from where the fighter
/// aims, and positive is clockwise on screen.
#[derive(Clone, Copy)]
struct Move {
    name: &'static str,
    shape: Shape,
    from: f32,
    to: f32,
    radius: f32,
    /// How long the swing itself takes.
    active: f32,
    /// How long the fighter winds up first.
    charge: f32,
    /// How far the fighter steps forward during the swing.
    lunge: f32,
    knock: f32,
    damage: i32,
    /// The radius of the shock ring where the swing lands, or 0 for none.
    ring: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Sword,
    Great,
    Axe,
    Spear,
    Staff,
}

struct Weapon {
    name: &'static str,
    look: Look,
    length: f32,
    /// Scales the wind-up and recovery. Above 1 is slower.
    speed: f32,
    dual: bool,
    /// The light, medium and heavy moves.
    moves: [Move; 3],
    /// Strings that end in a harder hit.
    combos: [(&'static str, &'static [Kind]); 4],
}

const L: Kind = Kind::Light;
const M: Kind = Kind::Medium;
const H: Kind = Kind::Heavy;

#[allow(clippy::too_many_arguments)]
const fn attack(
    name: &'static str,
    shape: Shape,
    (from, to): (f32, f32),
    radius: f32,
    active: f32,
    charge: f32,
    lunge: f32,
    knock: f32,
    damage: i32,
    ring: f32,
) -> Move {
    Move {
        name,
        shape,
        from,
        to,
        radius,
        active,
        charge,
        lunge,
        knock,
        damage,
        ring,
    }
}

static WEAPONS: [Weapon; 4] = [
    Weapon {
        name: "LONGSWORD",
        look: Look::Sword,
        length: 14.0,
        speed: 1.0,
        dual: false,
        moves: [
            attack(
                "CUT",
                Shape::Arc,
                (-1.2, 1.0),
                28.0,
                0.09,
                0.0,
                7.0,
                3.0,
                1,
                0.0,
            ),
            attack(
                "SWEEP",
                Shape::Arc,
                (1.7, -2.3),
                32.0,
                0.13,
                0.0,
                4.0,
                5.0,
                2,
                0.0,
            ),
            attack(
                "CLEAVE",
                Shape::Arc,
                (-2.5, 1.2),
                35.0,
                0.16,
                0.12,
                10.0,
                12.0,
                4,
                30.0,
            ),
        ],
        combos: [
            ("THREE CUTS", &[L, L, L]),
            ("TURNABOUT", &[L, L, M]),
            ("LADDER", &[L, M, H]),
            ("BREAKER", &[M, M, H]),
        ],
    },
    Weapon {
        name: "HAND AXES",
        look: Look::Axe,
        length: 9.0,
        speed: 0.8,
        dual: true,
        moves: [
            attack(
                "CHOP",
                Shape::Arc,
                (-0.9, 0.8),
                22.0,
                0.06,
                0.0,
                9.0,
                2.0,
                1,
                0.0,
            ),
            attack(
                "SCISSOR",
                Shape::Cross,
                (-1.5, 0.2),
                25.0,
                0.09,
                0.0,
                7.0,
                4.0,
                2,
                0.0,
            ),
            attack(
                "WHIRL",
                Shape::Spin,
                (0.0, 3.0 * PI),
                28.0,
                0.3,
                0.06,
                6.0,
                8.0,
                3,
                26.0,
            ),
        ],
        combos: [
            ("FLURRY", &[L, L, L, L]),
            ("SCISSOR", &[L, L, M]),
            ("WHIRLWIND", &[L, L, M, H]),
            ("SPLIT", &[M, H]),
        ],
    },
    Weapon {
        name: "GREATSWORD",
        look: Look::Great,
        length: 20.0,
        speed: 1.35,
        dual: false,
        moves: [
            attack(
                "SWING",
                Shape::Arc,
                (-1.6, 1.4),
                40.0,
                0.15,
                0.0,
                4.0,
                6.0,
                2,
                0.0,
            ),
            attack(
                "UPROOT",
                Shape::Arc,
                (2.0, -1.8),
                42.0,
                0.17,
                0.0,
                3.0,
                9.0,
                3,
                0.0,
            ),
            attack(
                "QUAKE",
                Shape::Arc,
                (-2.6, 0.3),
                44.0,
                0.18,
                0.28,
                6.0,
                16.0,
                6,
                54.0,
            ),
        ],
        combos: [
            ("PENDULUM", &[L, L]),
            ("UPROOT", &[L, M]),
            ("QUAKE", &[M, H]),
            ("AVALANCHE", &[L, M, H]),
        ],
    },
    Weapon {
        name: "SPEAR",
        look: Look::Spear,
        length: 22.0,
        speed: 1.0,
        dual: false,
        moves: [
            attack(
                "JAB",
                Shape::Thrust,
                (0.0, 0.0),
                40.0,
                0.07,
                0.0,
                5.0,
                3.0,
                1,
                0.0,
            ),
            attack(
                "REAP",
                Shape::Arc,
                (-1.5, 1.5),
                44.0,
                0.15,
                0.0,
                2.0,
                6.0,
                2,
                0.0,
            ),
            attack(
                "SKEWER",
                Shape::Thrust,
                (0.0, 0.0),
                58.0,
                0.12,
                0.16,
                26.0,
                26.0,
                4,
                0.0,
            ),
        ],
        combos: [
            ("NEEDLEWORK", &[L, L, L]),
            ("REAP", &[L, M]),
            ("SKEWER", &[L, L, H]),
            ("THRESHING", &[M, M, H]),
        ],
    },
];

// The skeleton and how it is drawn.

/// The colors of one fighter.
#[derive(Clone, Copy)]
struct Palette {
    outline: Color,
    hood: Color,
    hood_light: Color,
    face: Color,
    eye: Color,
    fur: Color,
    fur_dark: Color,
    cape: Color,
    cape_dark: Color,
    cape_light: Color,
    armor: Color,
    armor_light: Color,
    armor_dark: Color,
    belt: Color,
    gold: Color,
    legs: Color,
    legs_dark: Color,
    boot: Color,
    boot_light: Color,
    blade: Color,
    blade_shade: Color,
}

impl Palette {
    fn outlander() -> Palette {
        Palette {
            outline: hex(0x1b1836),
            hood: hex(0x2b2550),
            hood_light: hex(0x4b3f86),
            face: hex(0xf2c9b0),
            eye: hex(0x1b1836),
            fur: hex(0xf6ecf1),
            fur_dark: hex(0xc6bbe4),
            cape: hex(0xff5c8d),
            cape_dark: hex(0xc93a70),
            cape_light: hex(0xff9ab8),
            armor: hex(0x3d4a82),
            armor_light: hex(0x6f7fc0),
            armor_dark: hex(0x2b3566),
            belt: hex(0x5a3a2a),
            gold: hex(0xffd0a8),
            legs: hex(0x2b2550),
            legs_dark: hex(0x221d42),
            boot: hex(0x5a3a2a),
            boot_light: hex(0x8a5a40),
            blade: hex(0xf6ecf1),
            blade_shade: hex(0xa99fd6),
        }
    }

    fn wight() -> Palette {
        Palette {
            outline: hex(0x0c1020),
            hood: hex(0x1d2a44),
            hood_light: hex(0x33507a),
            face: hex(0x0c1020),
            eye: hex(0x49f0e2),
            fur: hex(0x56708f),
            fur_dark: hex(0x33507a),
            cape: hex(0x1d2a44),
            cape_dark: hex(0x141c30),
            cape_light: hex(0x33507a),
            armor: hex(0x2a3b5c),
            armor_light: hex(0x3f5a86),
            armor_dark: hex(0x1b2740),
            belt: hex(0x141c30),
            gold: hex(0x49f0e2),
            legs: hex(0x1d2a44),
            legs_dark: hex(0x141c30),
            boot: hex(0x141c30),
            boot_light: hex(0x2a3b5c),
            blade: hex(0xc9f4ff),
            blade_shade: hex(0x6fb6d6),
        }
    }

    fn brute() -> Palette {
        Palette {
            outline: hex(0x180e1c),
            hood: hex(0x3a2430),
            hood_light: hex(0x5a3a48),
            face: hex(0x120a12),
            eye: hex(0xffb86c),
            fur: hex(0x7a5a52),
            fur_dark: hex(0x4d342f),
            cape: hex(0x3a2430),
            cape_dark: hex(0x24161d),
            cape_light: hex(0x5a3a48),
            armor: hex(0x4a2f3a),
            armor_light: hex(0x6d4654),
            armor_dark: hex(0x33202a),
            belt: hex(0x24161d),
            gold: hex(0xffb86c),
            legs: hex(0x33202a),
            legs_dark: hex(0x24161d),
            boot: hex(0x24161d),
            boot_light: hex(0x4a2f3a),
            blade: hex(0xe8dcd0),
            blade_shade: hex(0xa08c84),
        }
    }

    fn caster() -> Palette {
        Palette {
            outline: hex(0x160f2e),
            hood: hex(0x3a1f5c),
            hood_light: hex(0x5e349a),
            face: hex(0x0e0820),
            eye: hex(0xff5cf0),
            fur: hex(0x5e349a),
            fur_dark: hex(0x3a1f5c),
            cape: hex(0x5e349a),
            cape_dark: hex(0x3a1f5c),
            cape_light: hex(0x8a5cd0),
            armor: hex(0x2c1a4a),
            armor_light: hex(0x47307a),
            armor_dark: hex(0x1c1034),
            belt: hex(0x1c1034),
            gold: hex(0xff5cf0),
            legs: hex(0x1c1034),
            legs_dark: hex(0x140b28),
            boot: hex(0x140b28),
            boot_light: hex(0x2c1a4a),
            blade: hex(0xff9af5),
            blade_shade: hex(0xc050d0),
        }
    }
}

/// Everything that places a fighter's skeleton for one frame.
#[derive(Clone, Copy)]
struct Pose {
    /// Where the feet stand.
    feet: Vec2,
    /// 1.0 facing right and -1.0 facing left.
    facing: f32,
    scale: f32,
    crouch: f32,
    lean: f32,
    bob: f32,
    /// How far through a walk cycle the legs are, as an angle.
    leg: f32,
    stride: f32,
    lift: f32,
    /// Where the weapon hand reaches, as an angle from the shoulder.
    arm: f32,
    /// Where the weapon points, as an angle from the hand.
    weapon: f32,
    length: f32,
    /// The other hand's reach, weapon angle and weapon length, if it is
    /// doing something other than swinging at the fighter's side.
    off_hand: Option<(f32, f32, f32)>,
    look: Look,
    horns: bool,
    cape: Option<[Vec2; 6]>,
    /// Draws the fighter as a flat shape in one color.
    flat: Option<Color>,
    bones: bool,
}

impl Pose {
    fn standing(feet: Vec2, facing: f32, look: Look, length: f32) -> Pose {
        Pose {
            feet,
            facing,
            scale: 1.0,
            crouch: 0.0,
            lean: 0.0,
            bob: 0.0,
            leg: 0.0,
            stride: 0.0,
            lift: 0.0,
            arm: mirror(1.35, facing),
            weapon: mirror(1.2, facing),
            length,
            off_hand: None,
            look,
            horns: false,
            cape: None,
            flat: None,
            bones: false,
        }
    }
}

/// Flips an angle that was written facing right to face left when `facing`
/// is negative.
fn mirror(angle: f32, facing: f32) -> f32 {
    if facing >= 0.0 { angle } else { PI - angle }
}

/// Two-bone inverse kinematics. Given a limb's root and where its end
/// should be, returns the middle joint and the end, pulled in if the limb
/// cannot reach that far. `bend` is 1.0 or -1.0 and picks the side the
/// joint bends to.
fn solve_limb(root: Vec2, target: Vec2, upper: f32, lower: f32, bend: f32) -> (Vec2, Vec2) {
    let mut reach = target - root;
    let mut length = reach.length().max(0.001);
    let longest = upper + lower - 0.01;
    if length > longest {
        reach *= longest / length;
        length = longest;
    }
    let along = (upper * upper - lower * lower + length * length) / (2.0 * length);
    let out = (upper * upper - along * along).max(0.0).sqrt();
    let unit = reach / length;
    let joint = root + unit * along + Vec2::new(-unit.y, unit.x) * out * bend;
    (joint, root + reach)
}

fn line(from: Vec2, to: Vec2, color: Color) {
    graphics::set_color(color);
    graphics::line((from.x + 0.5, from.y + 0.5), (to.x + 0.5, to.y + 0.5));
}

fn rect(x: f32, y: f32, width: f32, height: f32, color: Color) {
    graphics::set_color(color);
    graphics::rectangle(
        DrawMode::Fill,
        (x.round(), y.round()),
        (width.round().max(1.0), height.round().max(1.0)),
    );
}

fn oval(center: Vec2, rx: f32, ry: f32, color: Color) {
    graphics::set_color(color);
    graphics::ellipse(DrawMode::Fill, center, (rx, ry));
}

fn fill(points: &[Vec2], color: Color) {
    graphics::set_color(color);
    graphics::polygon(DrawMode::Fill, points);
}

/// Fills the band between two lines of points, which need not be convex.
fn band(left: &[Vec2], right: &[Vec2], color: Color) {
    graphics::set_color(color);
    for i in 1..left.len().min(right.len()) {
        graphics::polygon(
            DrawMode::Fill,
            &[left[i - 1], left[i], right[i], right[i - 1]],
        );
    }
}

/// Builds the skeleton for `pose` and draws the fighter over it.
#[allow(clippy::too_many_lines)]
fn draw_fighter(pose: &Pose, colors: &Palette) {
    let k = pose.scale;
    let f = pose.facing;
    let feet = pose.feet;
    let v = Vec2::new;

    // The spine, from the hips up to the head.
    let hip = v(feet.x, feet.y - 9.0 * k + pose.crouch * k);
    let waist = v(feet.x + pose.lean * k * 0.25, hip.y - 2.0 * k);
    let chest = v(feet.x + pose.lean * k * 0.7, waist.y - 5.0 * k);
    let neck = v(feet.x + pose.lean * k, chest.y - 3.5 * k + pose.bob * k);
    let head = v(neck.x + f * 0.6 * k, neck.y - 4.2 * k);
    let front_shoulder = v(neck.x + f * 2.5 * k, neck.y + 1.5 * k);
    let back_shoulder = v(neck.x - f * 3.0 * k, neck.y + 1.5 * k);

    // Each leg plants its foot along the walk cycle, then the knee is
    // solved from the hip and the ankle.
    let legs = [PI, 0.0].map(|phase| {
        let side = if phase == 0.0 { 1.5 } else { -1.5 };
        let cycle = pose.leg + phase;
        let root = v(hip.x + side * k * f, hip.y);
        let foot = v(
            root.x + cycle.sin() * pose.stride * k,
            feet.y - cycle.cos().max(0.0) * pose.lift * k - 1.5 * k,
        );
        let (knee, ankle) = solve_limb(root, foot, 4.6 * k, 4.6 * k, -f);
        (root, knee, ankle)
    });

    // The weapon arm reaches toward its angle, and the elbow is solved.
    let reach = front_shoulder + v(pose.arm.cos() * 7.6, pose.arm.sin() * 6.4) * k;
    let (elbow, hand) = solve_limb(front_shoulder, reach, 4.0 * k, 4.0 * k, f);
    let swing = if pose.stride > 0.0 && pose.lift > 0.0 {
        pose.leg.sin() * pose.stride * 0.7 * k
    } else {
        0.0
    };
    let off_reach = match pose.off_hand {
        Some((angle, _, _)) => back_shoulder + v(angle.cos() * 7.0, angle.sin() * 6.0) * k,
        None => v(
            back_shoulder.x - f * 1.5 * k - pose.arm.cos() * 2.5 * k + swing,
            back_shoulder.y + 6.4 * k,
        ),
    };
    let (off_elbow, off_hand) = solve_limb(back_shoulder, off_reach, 4.0 * k, 4.0 * k, -f);
    let aim = v(pose.weapon.cos(), pose.weapon.sin() * 0.85);
    let tip = hand + aim * pose.length * k;

    if pose.bones {
        let bone = hex(0x49f0e2);
        if let Some(cape) = pose.cape {
            for pair in cape.windows(2) {
                line(pair[0], pair[1], hex(0xffd0a8));
            }
        }
        for (root, knee, ankle) in legs {
            line(root, knee, bone);
            line(knee, ankle, bone);
            line(ankle, ankle + v(f * 2.5 * k, 1.5 * k), bone);
        }
        let bones = [
            (legs[0].0, legs[1].0),
            (hip, waist),
            (waist, chest),
            (chest, neck),
            (neck, head),
            (back_shoulder, front_shoulder),
            (front_shoulder, elbow),
            (elbow, hand),
            (back_shoulder, off_elbow),
            (off_elbow, off_hand),
        ];
        for (from, to) in bones {
            line(from, to, bone);
        }
        line(hand, tip, Color::WHITE);
        let mut joints = vec![
            waist,
            chest,
            neck,
            head,
            front_shoulder,
            back_shoulder,
            elbow,
            off_elbow,
            hand,
            off_hand,
            tip,
        ];
        joints.extend(legs.iter().flat_map(|leg| [leg.0, leg.1, leg.2]));
        joints.extend(pose.cape.iter().flatten());
        for joint in joints {
            rect(joint.x - 0.5, joint.y - 0.5, 2.0, 2.0, hex(0xff5c8d));
        }
        return;
    }

    // One pass draws the whole fighter, moved by `offset`. With `flat` it
    // is a single color, which draws the outline and the flashes.
    let pass = |offset: Vec2, flat: Option<Color>| {
        let pick = |color: Color| flat.unwrap_or(color);
        let detail = flat.is_none();
        let o = offset;
        let limb = |from: Vec2, to: Vec2, color: Color| {
            line(from + o, to + o, pick(color));
            line(from + o + v(1.0, 0.0), to + o + v(1.0, 0.0), pick(color));
        };

        if let Some(cape) = pose.cape {
            let mut left = [Vec2::ZERO; 6];
            let mut right = [Vec2::ZERO; 6];
            for i in 0..6 {
                let along = (cape[(i + 1).min(5)] - cape[i.saturating_sub(1)]).normalize_or_zero();
                let width = if i == 5 { 2.4 } else { 3.4 + i as f32 * 0.55 } * k;
                let across = v(-along.y, along.x) * width;
                left[i] = cape[i] + across + o;
                right[i] = cape[i] - across + o;
            }
            band(&left, &right, pick(colors.cape));
            if detail {
                for i in 1..6 {
                    line(cape[i - 1], cape[i], colors.cape_dark);
                    line(left[i - 1], left[i], colors.cape_light);
                }
                line(left[5], right[5], colors.cape_dark);
            }
        }

        // The arm and leg on the far side go behind the body.
        limb(back_shoulder, off_elbow, colors.armor_dark);
        limb(off_elbow, off_hand, colors.armor);
        rect(
            off_hand.x + o.x,
            off_hand.y + o.y,
            2.0,
            2.0,
            pick(colors.face),
        );
        for (index, (root, knee, ankle)) in legs.into_iter().enumerate() {
            let color = if index == 0 {
                colors.legs_dark
            } else {
                colors.legs
            };
            limb(root, knee, color);
            limb(knee, ankle, color);
            let boot = ankle.x + if f > 0.0 { -0.5 } else { -1.5 } * k;
            let size = ((4.0 * k).max(3.0), (2.6 * k).max(2.0));
            rect(
                boot + o.x,
                ankle.y - k + o.y,
                size.0,
                size.1,
                pick(colors.boot),
            );
            if detail {
                rect(boot, ankle.y - k, size.0, 1.0, colors.boot_light);
            }
        }

        // The skirt of the coat, then the torso over it.
        let sway = -pose.lean * k * 0.4;
        fill(
            &[
                v(waist.x - 3.0 * k, waist.y) + o,
                v(waist.x + 3.0 * k + 1.0, waist.y) + o,
                v(hip.x + 4.6 * k + 1.0 + sway, hip.y + 3.4 * k) + o,
                v(hip.x - 4.6 * k + sway, hip.y + 3.4 * k) + o,
            ],
            pick(colors.armor_dark),
        );
        fill(
            &[
                v(waist.x - 3.0 * k, waist.y + 1.0) + o,
                v(waist.x + 3.0 * k + 1.0, waist.y + 1.0) + o,
                v(chest.x + 4.0 * k + 1.0, chest.y) + o,
                v(neck.x + 3.6 * k + 1.0, neck.y + k) + o,
                v(neck.x - 3.6 * k, neck.y + k) + o,
                v(chest.x - 4.0 * k, chest.y) + o,
            ],
            pick(colors.armor),
        );
        if detail {
            let front = if f > 0.0 { 2.6 } else { -3.0 } * k;
            let back = if f > 0.0 { -3.4 } else { 3.0 } * k;
            line(
                v(neck.x + front, neck.y + 2.0 * k),
                v(waist.x + front * 0.8, waist.y),
                colors.armor_light,
            );
            line(
                v(neck.x + back, neck.y + 2.0 * k),
                v(waist.x + back * 0.8, waist.y),
                colors.armor_dark,
            );
            rect(
                waist.x - 3.0 * k,
                waist.y - 1.0,
                6.0 * k + 2.0,
                1.4 * k,
                colors.belt,
            );
            rect(
                waist.x + f * k,
                waist.y - 1.0,
                1.6 * k,
                1.4 * k,
                colors.gold,
            );
            rect(
                chest.x + f * 0.5 * k,
                chest.y + 0.5 * k,
                1.0,
                2.0,
                colors.gold,
            );
        }

        // The fur collar, the hood with its tail, and the face.
        oval(
            v(neck.x + 0.5, neck.y + 0.8 * k) + o,
            5.0 * k,
            2.0 * k,
            pick(colors.fur),
        );
        if detail {
            line(
                v(neck.x - 4.0 * k, neck.y + 2.2 * k),
                v(neck.x + 4.5 * k, neck.y + 2.2 * k),
                colors.fur_dark,
            );
        }
        fill(
            &[
                v(head.x - f * 1.5 * k, head.y - 4.0 * k) + o,
                v(head.x - f * 6.2 * k, head.y - 0.2 * k) + o,
                v(head.x - f * 2.0 * k, head.y + 3.0 * k) + o,
            ],
            pick(colors.hood),
        );
        oval(
            v(head.x + 0.5, head.y) + o,
            3.9 * k,
            4.1 * k,
            pick(colors.hood),
        );
        if pose.horns {
            for side in [-1.0, 1.0] {
                line(
                    v(head.x + side * 3.5 * k, head.y - 3.0 * k) + o,
                    v(head.x + side * 5.0 * k, head.y - 7.0 * k) + o,
                    pick(colors.blade),
                );
            }
        }
        if detail {
            let width = (3.2 * k).round().max(2.0);
            let left = if f > 0.0 {
                head.x + 0.6 * k
            } else {
                head.x - 0.6 * k - width + 1.0
            };
            rect(
                left,
                head.y - 1.4 * k,
                width,
                (3.6 * k).max(3.0),
                colors.face,
            );
            rect(left, head.y - 1.4 * k, width, 1.0, colors.hood_light);
            rect(
                head.x + f * 2.1 * k - 0.5,
                head.y + 0.2 * k,
                1.0,
                1.0,
                colors.eye,
            );
            line(
                v(head.x - f * 2.0 * k, head.y - 3.6 * k),
                v(head.x + f * 1.5 * k, head.y - 4.1 * k),
                colors.hood_light,
            );
        }

        // The near arm, its shoulder plate, and the weapons on top.
        limb(front_shoulder, elbow, colors.armor);
        limb(elbow, hand, colors.armor_light);
        oval(
            v(front_shoulder.x + 0.5, front_shoulder.y - 0.4 * k) + o,
            2.6 * k,
            2.1 * k,
            pick(colors.armor_light),
        );
        let weapon = |grip: Vec2, aim: Vec2, length: f32| {
            let across = v(-aim.y, aim.x);
            let tip = grip + aim * length * k;
            let base = grip + aim * 1.5 * k;
            match pose.look {
                Look::Spear => {
                    line(grip - aim * 8.0 * k + o, tip + o, pick(colors.belt));
                    let neck = tip - aim * 6.0 * k;
                    line(neck + o, tip + o, pick(colors.blade));
                    line(neck + across + o, tip - aim * 2.0 + o, pick(colors.blade));
                    line(
                        neck - across + o,
                        tip - aim * 2.0 + o,
                        pick(colors.blade_shade),
                    );
                }
                Look::Axe => {
                    line(base + o, tip + o, pick(colors.belt));
                    let size = (length * 0.3).min(4.0) * k;
                    oval(tip + o, size, size, pick(colors.blade));
                    if detail {
                        oval(
                            tip + v(1.0, 1.0),
                            size * 0.55,
                            size * 0.55,
                            colors.blade_shade,
                        );
                    }
                }
                Look::Staff => {
                    line(grip - aim * 5.0 * k + o, tip + o, pick(colors.belt));
                    oval(tip + o, 2.2 * k, 2.2 * k, pick(colors.blade));
                }
                Look::Sword | Look::Great => {
                    line(base + o, tip + o, pick(colors.blade));
                    line(
                        base + across + o,
                        tip - aim * 2.0 * k + across + o,
                        pick(colors.blade_shade),
                    );
                    let wide = pose.look == Look::Great;
                    if wide {
                        line(
                            base - across + o,
                            tip - aim * 2.0 * k - across + o,
                            pick(colors.blade),
                        );
                    }
                    if detail {
                        let guard = across * if wide { 3.6 } else { 2.6 } * k;
                        line(base + guard, base - guard, colors.gold);
                    }
                }
            }
        };
        if let Some((_, angle, length)) = pose.off_hand
            && length > 0.0
        {
            weapon(off_hand, v(angle.cos(), angle.sin() * 0.85), length);
        }
        if pose.length > 0.0 {
            weapon(hand, aim, pose.length);
        }
        rect(
            hand.x - 1.0 + o.x,
            hand.y - 1.0 + o.y,
            2.0,
            2.0,
            pick(colors.face),
        );
    };

    if let Some(color) = pose.flat {
        pass(Vec2::ZERO, Some(color));
    } else {
        for offset in [v(1.0, 0.0), v(-1.0, 0.0), v(0.0, 1.0), v(0.0, -1.0)] {
            pass(offset, Some(colors.outline));
        }
        pass(Vec2::ZERO, None);
    }
}

/// A cape hanging still from a fighter's neck, for fighters whose cape is
/// not simulated.
fn hanging_cape(pose: &Pose, time: f32) -> [Vec2; 6] {
    let k = pose.scale;
    let top = Vec2::new(
        pose.feet.x + pose.lean * k,
        pose.feet.y - 19.5 * k + pose.crouch * k,
    );
    std::array::from_fn(|i| {
        let i = i as f32;
        Vec2::new(
            top.x - pose.facing * (2.0 + i * 0.9) * k + (time * 2.0 + i * 0.8).sin() * 0.16 * i,
            top.y + i * 3.3 * k,
        )
    })
}

// The player's fighter.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Input {
    Attack(Kind),
    Dash,
}

/// One attack while it plays out.
struct Swing {
    kind: Kind,
    stats: Move,
    /// The angle the fighter aimed along when it began.
    aim: f32,
    /// 1.0, or -1.0 to sweep the other way round.
    flip: f32,
    time: f32,
    windup: f32,
    active: f32,
    recovery: f32,
    damage: i32,
    finisher: bool,
    /// The enemies this swing has already hit.
    hit: Vec<u32>,
    started: bool,
    finished: bool,
}

impl Swing {
    /// How far through its sweep the swing is, from 0.0 to 1.0.
    fn progress(&self) -> f32 {
        ease((self.time - self.windup) / self.active)
    }

    fn is_active(&self) -> bool {
        self.time >= self.windup && self.time < self.windup + self.active
    }

    fn in_recovery(&self) -> bool {
        self.time >= self.windup + self.active
    }

    /// The angle the weapon points along right now.
    fn angle(&self) -> f32 {
        let stats = &self.stats;
        self.aim + self.flip * (stats.from + (stats.to - stats.from) * self.progress())
    }

    /// Whether the swing has reached a circle of `radius` at `offset` from
    /// the fighter.
    fn reaches(&self, offset: Vec2, radius: f32) -> bool {
        let stats = &self.stats;
        let distance = offset.length();
        match stats.shape {
            Shape::Spin => distance <= stats.radius + radius,
            Shape::Thrust => {
                let aim = Vec2::from_angle(self.aim);
                let along = offset.dot(aim);
                let across = offset.dot(aim.perp()).abs();
                along >= -radius
                    && along <= stats.radius * self.progress().max(0.4) + radius
                    && across <= 7.0 + radius
            }
            Shape::Arc | Shape::Cross => {
                if distance > stats.radius + radius {
                    return false;
                }
                // Close enough that the angle hardly matters.
                if distance < radius + 4.0 {
                    return true;
                }
                let angle = wrap(offset.to_angle() - self.aim) * self.flip;
                let (low, high) = if stats.shape == Shape::Cross {
                    (-stats.from.abs(), stats.from.abs())
                } else {
                    let now = stats.from + (stats.to - stats.from) * self.progress();
                    (stats.from.min(now), stats.from.max(now))
                };
                angle >= low - 0.35 && angle <= high + 0.35
            }
        }
    }
}

enum Action {
    Free,
    Dash {
        time: f32,
        direction: Vec2,
        /// Another dash was asked for at the right moment and follows this
        /// one for free.
        chained: bool,
        /// Dash was pressed too early, so this dash cannot chain.
        fumbled: bool,
    },
    Attack(Swing),
    Dead,
}

struct Hero {
    position: Vec2,
    velocity: Vec2,
    aim: Vec2,
    facing: f32,
    action: Action,
    health: i32,
    stamina: f32,
    /// Seconds left during which nothing can hurt the fighter.
    safe: f32,
    hurt: f32,
    weapon: usize,
    /// The attacks of the string so far.
    string: Vec<Kind>,
    /// Seconds left to continue the string.
    string_timer: f32,
    buffer: Option<(Input, f32)>,
    /// How many dashes have chained without a pause.
    dashes: u32,
    walk: f32,
    cape: [Vec2; 6],
    ghost_timer: f32,
}

impl Hero {
    fn new() -> Hero {
        let position = Vec2::new(WIDTH / 2.0, 170.0);
        Hero {
            position,
            velocity: Vec2::ZERO,
            aim: Vec2::X,
            facing: 1.0,
            action: Action::Free,
            health: MAX_HEALTH,
            stamina: MAX_STAMINA,
            safe: 0.0,
            hurt: 0.0,
            weapon: 0,
            string: Vec::new(),
            string_timer: 0.0,
            buffer: None,
            dashes: 0,
            walk: 0.0,
            cape: std::array::from_fn(|i| position + Vec2::new(-2.0, -19.5 + i as f32 * 3.3)),
            ghost_timer: 0.0,
        }
    }

    fn weapon(&self) -> &'static Weapon {
        &WEAPONS[self.weapon]
    }

    fn is_dashing(&self) -> bool {
        matches!(self.action, Action::Dash { .. })
    }

    fn is_dead(&self) -> bool {
        matches!(self.action, Action::Dead)
    }

    /// The skeleton's pose for what the fighter is doing.
    fn pose(&self, time: f32) -> Pose {
        let weapon = self.weapon();
        let f = self.facing;
        let mut pose = Pose::standing(self.position, f, weapon.look, weapon.length);
        pose.cape = Some(self.cape);
        if weapon.dual {
            pose.off_hand = Some((mirror(1.9, f), mirror(1.5, f), weapon.length));
        }
        match &self.action {
            Action::Free => {
                let speed = self.velocity.length();
                if speed > 12.0 {
                    pose.leg = self.walk;
                    pose.stride = 4.0;
                    pose.lift = 2.5;
                    pose.lean = self.velocity.x / WALK_SPEED * 1.6;
                    if weapon.dual {
                        pose.off_hand = None;
                    }
                } else {
                    pose.bob = if (time * 2.4).sin() > 0.2 { 1.0 } else { 0.0 };
                }
                if self.hurt > 0.0 {
                    pose.lean = -f * 3.0;
                    pose.crouch = 1.0;
                }
            }
            Action::Dash { direction, .. } => {
                let back = (-*direction).to_angle();
                pose.lean = direction.x * 4.0;
                pose.crouch = 2.0;
                pose.leg = 1.1;
                pose.stride = 5.0;
                pose.arm = back + 0.5 * f;
                pose.weapon = back;
                if weapon.dual {
                    pose.off_hand = Some((back - 0.5 * f, back, weapon.length));
                }
            }
            Action::Attack(swing) => {
                let aim = Vec2::from_angle(swing.aim);
                let stats = &swing.stats;
                let angle = swing.angle();
                pose.lean = aim.x * if swing.is_active() { 3.0 } else { 1.5 };
                pose.leg = 0.9;
                pose.stride = 3.5;
                pose.arm = angle;
                pose.weapon = angle;
                if swing.time < swing.windup {
                    // Winding up: the weapon is drawn back, and shakes
                    // while a heavy attack charges.
                    let shake = if stats.charge > 0.0 {
                        (time * 70.0).sin() * 0.08
                    } else {
                        0.0
                    };
                    pose.crouch = 1.5;
                    pose.lean = -aim.x * 1.5;
                    if stats.shape == Shape::Thrust {
                        pose.arm = swing.aim + PI;
                        pose.weapon = swing.aim + shake;
                    } else {
                        pose.arm += shake;
                        pose.weapon += shake - 0.3 * swing.flip;
                    }
                } else if stats.shape == Shape::Spin {
                    pose.facing = if angle.cos() >= 0.0 { 1.0 } else { -1.0 };
                }
                if weapon.dual {
                    let other = match stats.shape {
                        // The off hand mirrors the main hand.
                        Shape::Cross => 2.0 * swing.aim - angle,
                        Shape::Spin => angle + PI,
                        _ => angle - 0.9 * swing.flip,
                    };
                    pose.off_hand = Some((other, other, weapon.length));
                }
            }
            Action::Dead => {
                pose.crouch = 5.0;
                pose.lean = -f * 4.0;
                pose.arm = mirror(1.5, f);
                pose.weapon = mirror(0.1, f);
            }
        }
        pose
    }
}

// Enemies.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Foe {
    /// Closes in and lunges along a line.
    Wight,
    /// Slow and armored, and slams a circle. Only a heavy attack breaks
    /// its guard.
    Brute,
    /// Keeps away and fires bolts.
    Caster,
}

impl Foe {
    fn health(self) -> i32 {
        match self {
            Foe::Wight => 5,
            Foe::Brute => 14,
            Foe::Caster => 3,
        }
    }

    fn radius(self) -> f32 {
        match self {
            Foe::Brute => 8.0,
            _ => 6.0,
        }
    }

    fn speed(self) -> f32 {
        match self {
            Foe::Wight => 54.0,
            Foe::Brute => 32.0,
            Foe::Caster => 42.0,
        }
    }

    fn windup(self) -> f32 {
        match self {
            Foe::Wight => 0.5,
            Foe::Brute => 0.85,
            Foe::Caster => 0.8,
        }
    }

    fn glow(self) -> Color {
        match self {
            Foe::Wight => hex(0x49f0e2),
            Foe::Brute => hex(0xffb86c),
            Foe::Caster => hex(0xff5cf0),
        }
    }
}

const LUNGE_LENGTH: f32 = 50.0;
const LUNGE_WIDTH: f32 = 9.0;
const SLAM_RADIUS: f32 = 32.0;
const SLAM_REACH: f32 = 16.0;

#[derive(Clone, Copy)]
enum Mind {
    /// Rising out of the snow.
    Spawn(f32),
    Chase,
    /// Showing where the attack will land.
    Windup {
        time: f32,
        direction: Vec2,
    },
    Strike {
        time: f32,
        direction: Vec2,
    },
    Recover(f32),
    Stagger(f32),
}

struct Enemy {
    id: u32,
    foe: Foe,
    position: Vec2,
    knock: Vec2,
    health: i32,
    mind: Mind,
    facing: f32,
    walk: f32,
    /// Seconds until it may attack again.
    cooldown: f32,
    /// Seconds left with its guard broken.
    broken: f32,
    flash: f32,
    /// Which way a caster circles.
    orbit: f32,
}

impl Enemy {
    fn pose(&self, time: f32) -> Pose {
        let f = self.facing;
        let (look, length, scale) = match self.foe {
            Foe::Wight => (Look::Sword, 12.0, 1.0),
            Foe::Brute => (Look::Axe, 14.0, 1.35),
            Foe::Caster => (Look::Staff, 13.0, 0.95),
        };
        let mut pose = Pose::standing(self.position, f, look, length);
        pose.scale = scale;
        pose.horns = self.foe == Foe::Wight;
        match self.mind {
            Mind::Spawn(left) => pose.crouch = left * 14.0,
            Mind::Chase => {
                pose.leg = self.walk;
                pose.stride = 3.5;
                pose.lift = 2.0;
                pose.lean = f;
            }
            Mind::Windup { time, direction } => {
                let progress = time / self.foe.windup();
                let shake = (time * 60.0).sin() * 0.06;
                pose.crouch = 1.5;
                match self.foe {
                    Foe::Wight => {
                        pose.lean = -direction.x * 2.0;
                        pose.arm = direction.to_angle() + PI + shake;
                        pose.weapon = direction.to_angle() + shake;
                    }
                    Foe::Brute => {
                        pose.arm = -PI / 2.0 - f * 0.5 * progress + shake;
                        pose.weapon = -PI / 2.0 - f * 0.9 * progress + shake;
                        pose.lean = -f * 2.0 * progress;
                    }
                    Foe::Caster => {
                        pose.arm = direction.to_angle();
                        pose.weapon = -PI / 2.0 + shake;
                    }
                }
            }
            Mind::Strike { direction, .. } => {
                pose.lean = direction.x * 3.0;
                pose.leg = 1.0;
                pose.stride = 4.0;
                let down = mirror(0.9, f);
                let ahead = direction.to_angle();
                (pose.arm, pose.weapon) = match self.foe {
                    Foe::Brute => (down, down),
                    _ => (ahead, ahead),
                };
            }
            Mind::Recover(_) => {
                pose.crouch = 1.0;
                pose.arm = mirror(1.1, f);
                pose.weapon = mirror(0.7, f);
            }
            Mind::Stagger(_) => {
                pose.lean = -f * 3.0;
                pose.crouch = 1.5;
                pose.arm = mirror(2.2, f);
                pose.weapon = mirror(2.6, f);
            }
        }
        if self.foe == Foe::Caster {
            pose.cape = Some(hanging_cape(&pose, time + self.id as f32));
        }
        pose
    }
}

/// A caster's bolt.
struct Bolt {
    position: Vec2,
    velocity: Vec2,
    /// Sent back by a swing, so it now hurts enemies.
    deflected: bool,
    life: f32,
}

// Effects.

struct Particle {
    position: Vec2,
    velocity: Vec2,
    life: f32,
    span: f32,
    color: Color,
    /// Drawn as a streak along its path rather than a square.
    streak: bool,
}

/// The mark a swing leaves in the air, fading out.
struct Trail {
    center: Vec2,
    shape: Shape,
    from: f32,
    to: f32,
    radius: f32,
    color: Color,
    life: f32,
}

/// A ring that grows along the ground.
struct Ring {
    center: Vec2,
    radius: f32,
    color: Color,
    life: f32,
}

struct Popup {
    text: String,
    position: Vec2,
    color: Color,
    life: f32,
    scale: f32,
}

/// A faded copy of the fighter left behind by a dash.
struct Ghost {
    pose: Pose,
    life: f32,
}

struct Pine {
    position: Vec2,
    height: f32,
}

// Sound, all of it made when the game loads.

const RATE: u32 = 44_100;

/// Makes `seconds` of sound from `wave`, which is given the time in
/// seconds and how far through the sound it is, from 0.0 to 1.0.
fn synth(seconds: f32, mut wave: impl FnMut(f32, f32) -> f32) -> Source {
    let count = (seconds * RATE as f32) as usize;
    let mut data = SoundData::new(count, RATE, 1);
    for i in 0..count {
        let t = i as f32 / RATE as f32;
        data.set_sample(i, 0, wave(t, i as f32 / count as f32).clamp(-1.0, 1.0));
    }
    Source::from_sound_data(&data)
}

/// White noise through a low-pass filter. Lower `cutoff` is duller.
struct Noise {
    rng: RandomGenerator,
    last: f32,
}

impl Noise {
    fn new(seed: u64) -> Noise {
        Noise {
            rng: RandomGenerator::new(seed),
            last: 0.0,
        }
    }

    fn next(&mut self, cutoff: f32) -> f32 {
        self.last += (self.rng.random_range(-1.0, 1.0) - self.last) * cutoff;
        self.last
    }
}

/// A sine wave whose pitch can slide.
#[derive(Default)]
struct Tone {
    phase: f32,
}

impl Tone {
    fn next(&mut self, frequency: f32) -> f32 {
        self.phase = (self.phase + frequency / RATE as f32).fract();
        (self.phase * TAU).sin()
    }
}

struct Sounds {
    swing: Source,
    hit: Source,
    heavy: Source,
    dash: Source,
    deflect: Source,
    hurt: Source,
    fall: Source,
    warn: Source,
    slam: Source,
    bolt: Source,
}

impl Sounds {
    fn new() -> Sounds {
        let mut noise = Noise::new(3);
        let swing = synth(0.16, |_, progress| {
            noise.next(0.12 + progress * 0.5) * (progress * PI).sin() * 1.6
        });

        let mut noise = Noise::new(5);
        let mut tone = Tone::default();
        let hit = synth(0.12, |t, _| {
            let thump = tone.next(190.0 - t * 900.0) * 0.7;
            (noise.next(0.6) + thump) * (-t * 34.0).exp() * 0.8
        });

        let mut noise = Noise::new(7);
        let mut tone = Tone::default();
        let heavy = synth(0.4, |t, _| {
            let thump = tone.next(110.0 - t * 160.0);
            (noise.next(0.3) * 1.4 + thump) * (-t * 11.0).exp() * 0.9
        });

        let mut noise = Noise::new(11);
        let dash = synth(0.2, |_, progress| {
            noise.next(0.06 + progress * 0.3) * 2.2 * (progress * PI).sin()
        });

        let mut tone = Tone::default();
        let mut high = Tone::default();
        let deflect = synth(0.35, |t, _| {
            (tone.next(1320.0) + high.next(1980.0) * 0.6) * (-t * 12.0).exp() * 0.35
        });

        let mut noise = Noise::new(13);
        let mut tone = Tone::default();
        let hurt = synth(0.4, |t, _| {
            let buzz = (tone.next(150.0 - t * 150.0) * 4.0).tanh();
            (buzz * 0.6 + noise.next(0.3)) * (-t * 8.0).exp() * 0.6
        });

        let mut noise = Noise::new(17);
        let mut tone = Tone::default();
        let fall = synth(0.45, |t, progress| {
            let sigh = tone.next(420.0 - progress * 300.0) * 0.4;
            (noise.next(0.2) * 1.5 + sigh) * (-t * 8.0).exp() * 0.7
        });

        let mut tone = Tone::default();
        let warn = synth(0.09, |t, _| {
            tone.next(620.0 + t * 2400.0) * (-t * 30.0).exp() * 0.25
        });

        let mut noise = Noise::new(19);
        let mut tone = Tone::default();
        let slam = synth(0.7, |t, _| {
            let thump = tone.next(80.0 - t * 40.0) * (-t * 7.0).exp();
            (noise.next(0.18) * 1.6 + thump) * (-t * 5.0).exp() * 0.9
        });

        let mut tone = Tone::default();
        let bolt = synth(0.25, |t, _| {
            tone.next(380.0 + t * 1600.0) * (-t * 12.0).exp() * 0.3
        });

        Sounds {
            swing,
            hit,
            heavy,
            dash,
            deflect,
            hurt,
            fall,
            warn,
            slam,
            bolt,
        }
    }
}

// The game.

struct Frostmarch {
    hero: Hero,
    enemies: Vec<Enemy>,
    bolts: Vec<Bolt>,
    particles: Vec<Particle>,
    trails: Vec<Trail>,
    rings: Vec<Ring>,
    popups: Vec<Popup>,
    ghosts: Vec<Ghost>,
    flakes: Vec<Vec2>,
    pines: Vec<Pine>,
    rng: RandomGenerator,
    sounds: Sounds,
    ground: Image,
    font: Font,
    hero_colors: Palette,
    foe_colors: [Palette; 3],
    time: f32,
    wave: u32,
    /// Seconds until the next wave rises, while the field is clear.
    wave_timer: f32,
    /// Seconds the game stays frozen after a hit lands.
    hit_stop: f32,
    /// Screen shake, from 0.0 to 1.0.
    trauma: f32,
    /// A red flash when the fighter is hurt.
    flash: f32,
    /// Hits landed in a row without being hit.
    chain: u32,
    chain_timer: f32,
    best_chain: u32,
    next_id: u32,
    pad: Option<Joystick>,
    using_pad: bool,
    bones: bool,
    /// The game plays itself.
    demo: bool,
    demo_move: Vec2,
    demo_plan: Vec<Kind>,
    demo_wait: f32,
    dead_time: f32,
}

impl Frostmarch {
    fn restart(&mut self) {
        self.hero = Hero::new();
        self.enemies.clear();
        self.bolts.clear();
        self.trails.clear();
        self.rings.clear();
        self.ghosts.clear();
        self.wave = 0;
        self.wave_timer = 1.0;
        self.chain = 0;
        self.dead_time = 0.0;
    }

    /// Takes a button press. Most are remembered for a moment and acted on
    /// as soon as the fighter is able to.
    fn press(&mut self, input: Input) {
        if self.hero.is_dead() {
            return;
        }
        if input == Input::Dash
            && let Action::Dash {
                time,
                chained,
                fumbled,
                ..
            } = &mut self.hero.action
        {
            // The window to chain a dash gets tighter with each one.
            let window = (0.11 - self.hero.dashes as f32 * 0.012).max(0.05);
            if !*fumbled && DASH_TIME - *time <= window {
                *chained = true;
            } else if !*chained {
                *fumbled = true;
            }
            return;
        }
        self.hero.buffer = Some((input, BUFFER_TIME));
    }

    fn change_weapon(&mut self, weapon: usize) {
        if self.hero.is_dead() || weapon == self.hero.weapon {
            return;
        }
        self.hero.weapon = weapon;
        self.hero.string.clear();
        if matches!(self.hero.action, Action::Attack(_)) {
            self.hero.action = Action::Free;
        }
        let position = self.hero.position + Vec2::new(0.0, -34.0);
        self.popup(WEAPONS[weapon].name, position, hex(0xffd0a8), 1.0);
    }

    fn popup(&mut self, text: &str, position: Vec2, color: Color, scale: f32) {
        self.popups.push(Popup {
            text: text.to_string(),
            position,
            color,
            life: 0.9,
            scale,
        });
    }

    fn sparks(&mut self, at: Vec2, toward: Vec2, count: u32, color: Color) {
        for _ in 0..count {
            let angle = toward.to_angle() + self.rng.random_range(-0.9, 0.9);
            let speed = self.rng.random_range(60.0, 240.0);
            let span = self.rng.random_range(0.12, 0.3);
            self.particles.push(Particle {
                position: at,
                velocity: Vec2::from_angle(angle) * speed,
                life: span,
                span,
                color,
                streak: true,
            });
        }
    }

    fn puff(&mut self, at: Vec2, count: u32, color: Color, speed: f32) {
        for _ in 0..count {
            let angle = self.rng.random_range(0.0, TAU);
            let speed = self.rng.random_range(0.2, 1.0) * speed;
            let span = self.rng.random_range(0.25, 0.6);
            self.particles.push(Particle {
                position: at,
                velocity: Vec2::from_angle(angle) * speed,
                life: span,
                span,
                color,
                streak: false,
            });
        }
    }

    fn rumble(&self, strength: f32, seconds: f32) {
        if self.using_pad
            && let Some(pad) = &self.pad
        {
            pad.set_vibration(strength, strength * 0.6, seconds);
        }
    }

    /// Where the player wants to walk and where they aim.
    fn steering(&self) -> (Vec2, Vec2) {
        let hero = &self.hero;
        if self.demo {
            return (self.demo_move, hero.aim);
        }
        if self.using_pad
            && let Some(pad) = &self.pad
        {
            let walk = pad.left_stick(0.25);
            let look = pad.right_stick(0.35);
            let aim = if look != Vec2::ZERO { look } else { walk };
            return (walk, aim.try_normalize().unwrap_or(hero.aim));
        }
        let aim = mouse::position() - (hero.position + CHEST);
        (
            keyboard::direction(),
            aim.try_normalize().unwrap_or(hero.aim),
        )
    }

    // The fighter.

    fn begin_dash(&mut self, direction: Vec2, free: bool) -> Option<Action> {
        if !free {
            if self.hero.stamina < 1.0 {
                return None;
            }
            self.hero.stamina -= 1.0;
            self.hero.dashes = 0;
        } else {
            self.hero.dashes += 1;
            let position = self.hero.position + Vec2::new(0.0, -34.0);
            let text = format!("CHAIN {}", self.hero.dashes + 1);
            self.popup(&text, position, hex(0x49f0e2), 1.0);
        }
        self.sounds
            .dash
            .play_once_with(0.5, 1.0 + self.hero.dashes as f32 * 0.08);
        let position = self.hero.position;
        self.puff(position, 6, hex(0xffffff), 40.0);
        Some(Action::Dash {
            time: 0.0,
            direction,
            chained: false,
            fumbled: false,
        })
    }

    fn begin_attack(&mut self, kind: Kind, from_dash: bool) -> Action {
        let hero = &mut self.hero;
        let weapon = hero.weapon();
        if hero.string.len() >= 4 || hero.string.last() == Some(&Kind::Heavy) {
            hero.string.clear();
        }
        hero.string.push(kind);
        hero.string_timer = 0.5;
        let stats = weapon.moves[kind as usize];
        let finisher = weapon
            .combos
            .iter()
            .find(|(_, string)| *string == hero.string.as_slice())
            .map(|(name, _)| *name);
        if hero.aim.x.abs() > 0.2 {
            hero.facing = hero.aim.x.signum();
        }
        // Light attacks swing one way and then back the other.
        let alternate = kind == Kind::Light && hero.string.len().is_multiple_of(2);
        let flip = if alternate { -hero.facing } else { hero.facing };
        let windup = if stats.charge > 0.0 {
            stats.charge
        } else if kind == Kind::Light {
            0.035 * weapon.speed
        } else {
            0.07 * weapon.speed
        };
        let swing = Swing {
            kind,
            stats,
            aim: hero.aim.to_angle(),
            flip,
            time: 0.0,
            windup,
            active: stats.active,
            recovery: kind.recovery() * weapon.speed,
            damage: stats.damage + i32::from(from_dash) + if finisher.is_some() { 2 } else { 0 },
            finisher: finisher.is_some(),
            hit: Vec::new(),
            started: false,
            finished: false,
        };
        let above = hero.position + Vec2::new(0.0, -36.0);
        if let Some(name) = finisher {
            self.popup(name, above, kind.color(), 2.0);
        } else if from_dash {
            self.popup("DASH CUT", above, hex(0x49f0e2), 1.0);
        }
        Action::Attack(swing)
    }

    fn update_hero(&mut self, dt: f32) {
        let (walk, aim) = self.steering();
        let hero = &mut self.hero;
        hero.safe = (hero.safe - dt).max(0.0);
        hero.hurt = (hero.hurt - dt).max(0.0);
        if let Some((_, left)) = &mut hero.buffer {
            *left -= dt;
            if *left <= 0.0 {
                hero.buffer = None;
            }
        }
        if !hero.is_dashing() {
            hero.stamina = (hero.stamina + dt / 0.8).min(MAX_STAMINA);
        }
        if !matches!(hero.action, Action::Attack(_)) {
            hero.aim = aim;
            hero.string_timer -= dt;
            if hero.string_timer <= 0.0 {
                hero.string.clear();
            }
        }
        let heading = walk.try_normalize().unwrap_or(hero.aim);

        let action = std::mem::replace(&mut self.hero.action, Action::Free);
        self.hero.action = match action {
            Action::Dead => Action::Dead,
            Action::Free => {
                let hero = &mut self.hero;
                let blend = 1.0 - (-dt * 18.0).exp();
                hero.velocity = hero.velocity.lerp(walk * WALK_SPEED, blend);
                hero.position += hero.velocity * dt;
                hero.walk += hero.velocity.length() * dt * 0.16;
                if hero.aim.x.abs() > 0.2 {
                    hero.facing = hero.aim.x.signum();
                }
                match hero.buffer.take() {
                    Some((Input::Attack(kind), _)) => self.begin_attack(kind, false),
                    Some((Input::Dash, left)) => {
                        self.begin_dash(heading, false).unwrap_or_else(|| {
                            // Out of stamina. Keep the press in case some
                            // comes back in time.
                            self.hero.buffer = Some((Input::Dash, left));
                            Action::Free
                        })
                    }
                    None => Action::Free,
                }
            }
            Action::Dash {
                time,
                direction,
                chained,
                fumbled,
            } => {
                let hero = &mut self.hero;
                let time = time + dt;
                let fade = 1.0 - (time / DASH_TIME).powi(2) * 0.7;
                hero.position += direction * DASH_SPEED * fade * dt;
                hero.velocity = direction * 70.0;
                if direction.x.abs() > 0.2 {
                    hero.facing = direction.x.signum();
                }
                hero.ghost_timer -= dt;
                if hero.ghost_timer <= 0.0 {
                    hero.ghost_timer = 0.025;
                    let mut pose = hero.pose(self.time);
                    pose.flat = Some(hex(0xff5c8d));
                    self.ghosts.push(Ghost { pose, life: 0.22 });
                }
                let attack = match self.hero.buffer {
                    Some((Input::Attack(kind), _)) if time > DASH_TIME * 0.55 => Some(kind),
                    _ => None,
                };
                if let Some(kind) = attack {
                    self.hero.buffer = None;
                    self.begin_attack(kind, true)
                } else if time < DASH_TIME {
                    Action::Dash {
                        time,
                        direction,
                        chained,
                        fumbled,
                    }
                } else if chained {
                    self.begin_dash(heading, true).unwrap_or(Action::Free)
                } else {
                    Action::Free
                }
            }
            Action::Attack(mut swing) => {
                swing.time += dt;
                let aim = Vec2::from_angle(swing.aim);
                let hero = &mut self.hero;
                hero.velocity = Vec2::ZERO;
                if swing.is_active() {
                    hero.position += aim * swing.stats.lunge / swing.active * dt;
                }
                if swing.time >= swing.windup && !swing.started {
                    swing.started = true;
                    let pitch = match swing.kind {
                        Kind::Light => 1.5,
                        Kind::Medium => 1.1,
                        Kind::Heavy => 0.75,
                    };
                    self.sounds.swing.play_once_with(0.45, pitch);
                }
                if swing.is_active() {
                    self.land_swing(&mut swing);
                }
                if swing.in_recovery() && !swing.finished {
                    swing.finished = true;
                    self.finish_swing(&mut swing);
                }
                // A light attack can be dashed out of at any point, the
                // others only once the swing is over.
                let can_dash = swing.kind == Kind::Light || swing.in_recovery();
                let buffered = self.hero.buffer.map(|(input, _)| input);
                match buffered {
                    Some(Input::Dash) if can_dash && self.hero.stamina >= 1.0 => {
                        self.hero.buffer = None;
                        self.begin_dash(heading, false).unwrap_or(Action::Free)
                    }
                    Some(Input::Attack(kind)) if swing.in_recovery() => {
                        self.hero.buffer = None;
                        self.hero.aim = aim.lerp(self.steering().1, 0.6).normalize_or(aim);
                        self.begin_attack(kind, false)
                    }
                    _ if swing.time >= swing.windup + swing.active + swing.recovery => Action::Free,
                    _ => Action::Attack(swing),
                }
            }
        };

        let hero = &mut self.hero;
        hero.position = hero.position.clamp(ARENA_MIN, ARENA_MAX);

        // The cape is a chain. Its first link is pinned to the neck, and
        // each of the others falls and is pulled along by the one above.
        let pose = hero.pose(self.time);
        hero.cape[0] = hero.position
            + Vec2::new(
                pose.lean - pose.facing * 2.0,
                -19.5 + pose.crouch + pose.bob,
            );
        for i in 1..6 {
            let rest = hero.cape[i - 1] + Vec2::new(-pose.facing * 0.9, 3.3);
            let mut link = hero.cape[i].lerp(rest, 1.0 - (-dt * 9.0).exp());
            link.x += (self.time * 2.0 + i as f32 * 0.8).sin() * 0.03 * i as f32;
            let pull = link - hero.cape[i - 1];
            hero.cape[i] = hero.cape[i - 1] + pull.normalize_or(Vec2::Y) * pull.length().min(3.5);
        }
    }

    /// Hits whatever the swing has reached and not hit yet.
    fn land_swing(&mut self, swing: &mut Swing) {
        let origin = self.hero.position;
        let aim = Vec2::from_angle(swing.aim);
        let targets: Vec<usize> = (0..self.enemies.len())
            .filter(|&i| {
                let enemy = &self.enemies[i];
                !swing.hit.contains(&enemy.id)
                    && !matches!(enemy.mind, Mind::Spawn(_))
                    && swing.reaches(enemy.position - origin, enemy.foe.radius())
            })
            .collect();
        for index in targets {
            swing.hit.push(self.enemies[index].id);
            let push = (self.enemies[index].position - origin).normalize_or(aim);
            self.hurt_enemy(
                index,
                swing.damage,
                swing.kind,
                push * swing.stats.knock,
                swing.finisher,
            );
        }

        let mut deflected = false;
        for bolt in &mut self.bolts {
            if !bolt.deflected && swing.reaches(bolt.position - origin, 5.0) {
                bolt.deflected = true;
                bolt.velocity = aim * 300.0;
                bolt.life = 2.0;
                deflected = true;
            }
        }
        if deflected {
            let at = origin + CHEST + aim * 14.0;
            self.sparks(at, aim, 10, hex(0xff9af5));
            self.popup(
                "DEFLECT",
                origin + Vec2::new(0.0, -36.0),
                hex(0xff9af5),
                1.0,
            );
            self.sounds.deflect.play_once_with(0.6, 1.0);
            self.hit_stop = self.hit_stop.max(0.06);
            self.trauma = (self.trauma + 0.2).min(1.0);
        }
    }

    /// Runs once when a swing ends: leaves its trail and its shock ring.
    fn finish_swing(&mut self, swing: &mut Swing) {
        let stats = swing.stats;
        let origin = self.hero.position;
        self.trails.push(Trail {
            center: origin + CHEST,
            shape: stats.shape,
            from: swing.aim + swing.flip * stats.from,
            to: swing.aim + swing.flip * stats.to,
            radius: stats.radius,
            color: swing.kind.color(),
            life: 0.14,
        });
        if stats.ring <= 0.0 {
            return;
        }
        let center = if stats.shape == Shape::Spin {
            origin
        } else {
            origin + Vec2::from_angle(swing.angle()) * stats.radius * 0.6
        };
        self.rings.push(Ring {
            center,
            radius: stats.ring,
            color: swing.kind.color(),
            life: 0.3,
        });
        self.puff(center, 12, hex(0xffffff), 90.0);
        self.sounds.slam.play_once_with(0.5, 1.3);
        self.trauma = (self.trauma + 0.3).min(1.0);
        let caught: Vec<usize> = (0..self.enemies.len())
            .filter(|&i| {
                let enemy = &self.enemies[i];
                !swing.hit.contains(&enemy.id)
                    && !matches!(enemy.mind, Mind::Spawn(_))
                    && enemy.position.distance(center) <= stats.ring + enemy.foe.radius()
            })
            .collect();
        for index in caught {
            swing.hit.push(self.enemies[index].id);
            let push = (self.enemies[index].position - center).normalize_or(Vec2::X);
            self.hurt_enemy(
                index,
                (swing.damage / 2).max(1),
                Kind::Medium,
                push * stats.knock * 0.7,
                false,
            );
        }
    }

    fn hurt_enemy(&mut self, index: usize, damage: i32, kind: Kind, knock: Vec2, finisher: bool) {
        let enemy = &mut self.enemies[index];
        let armored = enemy.foe == Foe::Brute && enemy.broken <= 0.0;
        let mut broke = false;
        let mut damage = damage;
        if armored {
            if kind == Kind::Heavy {
                enemy.broken = 2.0;
                enemy.mind = Mind::Stagger(2.0);
                broke = true;
            } else {
                // Its guard soaks up most of a lighter hit.
                damage = (damage / 2).max(1);
            }
            enemy.knock = knock * 3.0;
        } else {
            if enemy.broken > 0.0 {
                damage += 1;
            }
            enemy.knock = knock * 14.0;
            let attacking = matches!(enemy.mind, Mind::Windup { .. } | Mind::Strike { .. });
            match kind {
                // A light hit makes it flinch but does not stop an attack
                // it has already begun.
                Kind::Light if !attacking => enemy.mind = Mind::Stagger(0.16),
                Kind::Light => {}
                Kind::Medium => enemy.mind = Mind::Stagger(0.5),
                Kind::Heavy => enemy.mind = Mind::Stagger(0.75),
            }
        }
        enemy.health -= damage;
        enemy.flash = 0.1;
        let at = enemy.position + CHEST;
        let glow = enemy.foe.glow();
        let fell = enemy.health <= 0;

        let count = match kind {
            Kind::Light => 5,
            Kind::Medium => 9,
            Kind::Heavy => 16,
        };
        let toward = knock.normalize_or(Vec2::X);
        self.sparks(at, toward, count, kind.color());
        self.hit_stop = self
            .hit_stop
            .max(kind.hit_stop() + if finisher { 0.05 } else { 0.0 });
        self.trauma = (self.trauma + kind.shake() + if finisher { 0.15 } else { 0.0 }).min(1.0);
        self.chain += 1;
        self.chain_timer = 2.5;
        self.best_chain = self.best_chain.max(self.chain);
        let pitch = self.rng.random_range(0.9, 1.1);
        if kind == Kind::Heavy || finisher {
            self.sounds.heavy.play_once_with(0.7, pitch);
            self.rumble(0.8, 0.18);
        } else {
            self.sounds.hit.play_once_with(0.6, pitch);
            self.rumble(0.35, 0.08);
        }
        if broke {
            self.popup(
                "GUARD BROKEN",
                at + Vec2::new(0.0, -26.0),
                hex(0xffb86c),
                1.0,
            );
        }
        if fell {
            self.puff(at, 18, glow, 110.0);
            self.puff(at, 10, hex(0x1d2a44), 60.0);
            self.sounds.fall.play_once_with(0.6, pitch);
        }
    }

    fn hurt_hero(&mut self, damage: i32, from: Vec2) {
        let hero = &mut self.hero;
        if hero.safe > 0.0 || hero.is_dashing() || hero.is_dead() {
            return;
        }
        hero.health -= damage;
        hero.safe = 0.9;
        hero.hurt = 0.3;
        hero.velocity = (hero.position - from).normalize_or(Vec2::X) * 190.0;
        hero.string.clear();
        hero.buffer = None;
        hero.action = if hero.health <= 0 {
            Action::Dead
        } else {
            Action::Free
        };
        let at = hero.position + CHEST;
        self.chain = 0;
        self.flash = 1.0;
        self.hit_stop = self.hit_stop.max(0.12);
        self.trauma = (self.trauma + 0.6).min(1.0);
        self.sparks(at, at - (from + CHEST), 12, hex(0xff5c8d));
        self.sounds.hurt.play_once_with(0.7, 1.0);
        self.rumble(1.0, 0.25);
        if self.hero.is_dead() {
            self.puff(at, 24, hex(0xff5c8d), 120.0);
        }
    }

    // Enemies.

    fn spawn_wave(&mut self) {
        self.wave += 1;
        let wave = self.wave;
        let wights = (2 + wave.div_ceil(2)).min(6);
        let casters = (wave / 2).min(3);
        let brutes = (wave / 3).min(2);
        let foes = [
            (Foe::Wight, wights),
            (Foe::Caster, casters),
            (Foe::Brute, brutes),
        ];
        for (foe, count) in foes {
            for _ in 0..count {
                // Somewhere on the field that is not on top of the player.
                let mut position = self.hero.position;
                for _ in 0..20 {
                    position = Vec2::new(
                        self.rng
                            .random_range(ARENA_MIN.x + 20.0, ARENA_MAX.x - 20.0),
                        self.rng.random_range(ARENA_MIN.y + 12.0, ARENA_MAX.y - 8.0),
                    );
                    if position.distance(self.hero.position) > 90.0 {
                        break;
                    }
                }
                self.next_id += 1;
                self.enemies.push(Enemy {
                    id: self.next_id,
                    foe,
                    position,
                    knock: Vec2::ZERO,
                    health: foe.health(),
                    mind: Mind::Spawn(0.7),
                    facing: 1.0,
                    walk: self.rng.random_range(0.0, TAU),
                    cooldown: self.rng.random_range(0.3, 1.4),
                    broken: 0.0,
                    flash: 0.0,
                    orbit: if self.rng.chance(0.5) { 1.0 } else { -1.0 },
                });
                self.rings.push(Ring {
                    center: position,
                    radius: 16.0,
                    color: foe.glow(),
                    life: 0.5,
                });
            }
        }
        if self.demo && wave > 1 {
            // Show off the next weapon.
            self.change_weapon((self.hero.weapon + 1) % WEAPONS.len());
        }
        if wave > 1 && self.hero.health < MAX_HEALTH {
            self.hero.health += 1;
        }
        let text = format!("WAVE {wave}");
        self.popup(&text, Vec2::new(WIDTH / 2.0, 90.0), hex(0x1b1836), 2.0);
    }

    #[allow(clippy::too_many_lines)]
    fn update_enemies(&mut self, dt: f32) {
        let target = self.hero.position;
        let alive = !self.hero.is_dead();
        let mut blows: Vec<(i32, Vec2)> = Vec::new();
        let places: Vec<Vec2> = self.enemies.iter().map(|enemy| enemy.position).collect();

        for index in 0..self.enemies.len() {
            let enemy = &mut self.enemies[index];
            let foe = enemy.foe;
            let offset = target - enemy.position;
            let distance = offset.length();
            let toward = offset.normalize_or(Vec2::X);
            enemy.flash = (enemy.flash - dt).max(0.0);
            enemy.broken = (enemy.broken - dt).max(0.0);
            enemy.cooldown -= dt;
            enemy.position += enemy.knock * dt;
            enemy.knock *= (-dt * 10.0).exp();

            enemy.mind = match enemy.mind {
                Mind::Spawn(left) if left > dt => Mind::Spawn(left - dt),
                Mind::Spawn(_) => Mind::Chase,
                Mind::Stagger(left) if left > dt => Mind::Stagger(left - dt),
                Mind::Stagger(_) => Mind::Chase,
                Mind::Recover(left) if left > dt => Mind::Recover(left - dt),
                Mind::Recover(_) => Mind::Chase,
                Mind::Chase => {
                    // Keep a little apart from the others.
                    let mut walk = Vec2::ZERO;
                    for (other, place) in places.iter().enumerate() {
                        let apart = enemy.position - *place;
                        if other != index && apart.length() < 16.0 {
                            walk += apart.normalize_or(Vec2::X) * 0.8;
                        }
                    }
                    let reach = match foe {
                        Foe::Wight => 40.0,
                        Foe::Brute => 36.0,
                        Foe::Caster => 190.0,
                    };
                    match foe {
                        Foe::Caster if distance < 95.0 => walk -= toward,
                        Foe::Caster if distance > 150.0 => walk += toward,
                        Foe::Caster => walk += toward.perp() * enemy.orbit * 0.5,
                        // Stop short of the player rather than stand on them.
                        _ if distance > 20.0 => walk += toward,
                        _ => walk -= toward * 0.5,
                    }
                    enemy.position += walk.clamp_length_max(1.0) * foe.speed() * dt;
                    enemy.walk += dt * 11.0;
                    enemy.facing = if toward.x >= 0.0 { 1.0 } else { -1.0 };
                    if alive && distance < reach && enemy.cooldown <= 0.0 {
                        self.sounds.warn.play_once_with(0.5, 1.0);
                        Mind::Windup {
                            time: 0.0,
                            direction: toward,
                        }
                    } else {
                        Mind::Chase
                    }
                }
                Mind::Windup { time, direction } => {
                    let time = time + dt;
                    // A caster follows its target for a while before its
                    // aim locks.
                    let direction = if foe == Foe::Caster && time < foe.windup() * 0.6 {
                        toward
                    } else {
                        direction
                    };
                    enemy.facing = if direction.x >= 0.0 { 1.0 } else { -1.0 };
                    if time < foe.windup() {
                        Mind::Windup { time, direction }
                    } else {
                        match foe {
                            Foe::Wight => {}
                            Foe::Brute => {
                                let center = enemy.position + direction * SLAM_REACH;
                                if target.distance(center) <= SLAM_RADIUS + 3.0 {
                                    blows.push((2, center));
                                }
                                self.rings.push(Ring {
                                    center,
                                    radius: SLAM_RADIUS,
                                    color: hex(0xff9a3c),
                                    life: 0.3,
                                });
                                self.sounds.slam.play_once_with(0.8, 1.0);
                                self.trauma = (self.trauma + 0.45).min(1.0);
                            }
                            Foe::Caster => {
                                self.bolts.push(Bolt {
                                    position: enemy.position + direction * 10.0,
                                    velocity: direction * 150.0,
                                    deflected: false,
                                    life: 4.0,
                                });
                                self.sounds.bolt.play_once_with(0.5, 1.0);
                            }
                        }
                        Mind::Strike {
                            time: 0.0,
                            direction,
                        }
                    }
                }
                Mind::Strike { time, direction } => {
                    let time = time + dt;
                    if foe == Foe::Wight {
                        enemy.position += direction * 230.0 * dt;
                        if target.distance(enemy.position + direction * 8.0) < 13.0 {
                            blows.push((1, enemy.position));
                        }
                    }
                    if time < 0.14 {
                        Mind::Strike { time, direction }
                    } else {
                        enemy.cooldown = match foe {
                            Foe::Wight => self.rng.random_range(0.6, 1.3),
                            Foe::Brute => 1.2,
                            Foe::Caster => self.rng.random_range(1.6, 2.6),
                        };
                        Mind::Recover(match foe {
                            Foe::Wight => 0.55,
                            Foe::Brute => 1.0,
                            Foe::Caster => 0.4,
                        })
                    }
                }
            };
            enemy.position = enemy.position.clamp(ARENA_MIN, ARENA_MAX);
        }

        for (damage, from) in blows {
            self.hurt_hero(damage, from);
        }
        self.enemies.retain(|enemy| enemy.health > 0);
    }

    fn update_bolts(&mut self, dt: f32) {
        let target = self.hero.position;
        let mut blow = None;
        let mut struck: Vec<(usize, Vec2)> = Vec::new();
        for bolt in &mut self.bolts {
            bolt.position += bolt.velocity * dt;
            bolt.life -= dt;
            let inside = bolt.position.clamp(ARENA_MIN - 20.0, ARENA_MAX + 20.0);
            if inside != bolt.position {
                bolt.life = 0.0;
            }
            if bolt.deflected {
                let hit = self.enemies.iter().position(|enemy| {
                    !matches!(enemy.mind, Mind::Spawn(_))
                        && enemy.position.distance(bolt.position) < enemy.foe.radius() + 5.0
                });
                if let Some(index) = hit {
                    struck.push((index, bolt.velocity.normalize_or(Vec2::X)));
                    bolt.life = 0.0;
                }
            } else if target.distance(bolt.position) < 7.0 && !self.hero.is_dashing() {
                blow = Some(bolt.position);
                bolt.life = 0.0;
            }
        }
        for (index, push) in struck {
            if self.enemies[index].health > 0 {
                self.hurt_enemy(index, 3, Kind::Medium, push * 8.0, false);
            }
        }
        if let Some(from) = blow {
            self.hurt_hero(1, from);
        }
        self.enemies.retain(|enemy| enemy.health > 0);
        let mut burst = Vec::new();
        self.bolts.retain(|bolt| {
            if bolt.life <= 0.0 {
                burst.push(bolt.position + CHEST);
            }
            bolt.life > 0.0
        });
        for at in burst {
            self.puff(at, 6, hex(0xff9af5), 50.0);
        }
    }

    fn update_effects(&mut self, dt: f32) {
        for particle in &mut self.particles {
            particle.position += particle.velocity * dt;
            particle.velocity *= (-dt * 5.0).exp();
            particle.life -= dt;
        }
        self.particles.retain(|particle| particle.life > 0.0);
        for trail in &mut self.trails {
            trail.life -= dt;
        }
        self.trails.retain(|trail| trail.life > 0.0);
        for ring in &mut self.rings {
            ring.life -= dt;
        }
        self.rings.retain(|ring| ring.life > 0.0);
        for ghost in &mut self.ghosts {
            ghost.life -= dt;
        }
        self.ghosts.retain(|ghost| ghost.life > 0.0);
        for (index, flake) in self.flakes.iter_mut().enumerate() {
            let drift = (self.time * 0.7 + index as f32).sin() * 6.0 - 10.0;
            *flake += Vec2::new(drift, 14.0 + (index % 5) as f32 * 4.0) * dt;
            flake.x = flake.x.rem_euclid(WIDTH);
            flake.y = flake.y.rem_euclid(HEIGHT);
        }
        self.chain_timer -= dt;
        if self.chain_timer <= 0.0 {
            self.chain = 0;
        }
    }

    /// Plays the fighter when nobody else is: closes in on the nearest
    /// enemy, works through the weapon's strings, and dashes away from
    /// attacks that are about to land.
    fn play_demo(&mut self, dt: f32) {
        self.demo_wait -= dt;
        self.demo_move = Vec2::ZERO;
        if self.hero.is_dead() {
            if self.dead_time > 2.5 {
                self.restart();
            }
            return;
        }
        let hero = self.hero.position;
        let weapon = self.hero.weapon();

        // Something is about to land here: get out, or cut a bolt down.
        let mut danger = None;
        for enemy in &self.enemies {
            if let Mind::Windup { time, direction } = enemy.mind {
                let late = time > enemy.foe.windup() - 0.2;
                let offset = hero - enemy.position;
                let caught = match enemy.foe {
                    Foe::Wight => {
                        let along = offset.dot(direction);
                        (0.0..LUNGE_LENGTH + 8.0).contains(&along)
                            && offset.dot(direction.perp()).abs() < LUNGE_WIDTH + 6.0
                    }
                    Foe::Brute => offset.distance(direction * SLAM_REACH) < SLAM_RADIUS + 8.0,
                    Foe::Caster => false,
                };
                if late && caught {
                    danger = Some(direction.perp());
                }
            }
        }
        let bolt = self
            .bolts
            .iter()
            .find(|bolt| !bolt.deflected && bolt.position.distance(hero) < 34.0)
            .map(|bolt| bolt.position);
        if let Some(away) = danger
            && self.hero.stamina >= 1.0
        {
            self.demo_move = away;
            if !self.hero.is_dashing() {
                self.press(Input::Dash);
            }
            return;
        }
        if let Some(bolt) = bolt
            && matches!(self.hero.action, Action::Free)
        {
            self.hero.aim = (bolt - hero).normalize_or(Vec2::X);
            self.press(Input::Attack(Kind::Light));
            return;
        }

        let nearest = self.enemies.iter().min_by(|a, b| {
            let a = a.position.distance(hero);
            let b = b.position.distance(hero);
            a.total_cmp(&b)
        });
        let Some(enemy) = nearest else {
            return;
        };
        let offset = enemy.position - hero;
        let reach = weapon.moves[0].radius * 0.75;
        if matches!(self.hero.action, Action::Free) {
            self.hero.aim = offset.normalize_or(Vec2::X);
        }
        if offset.length() > reach {
            self.demo_move = offset.normalize_or(Vec2::X);
            if offset.length() > 110.0 && self.hero.stamina >= 2.5 && self.demo_wait <= 0.0 {
                self.press(Input::Dash);
                self.demo_wait = 0.5;
            }
            return;
        }
        let ready = match &self.hero.action {
            Action::Free => true,
            Action::Attack(swing) => swing.in_recovery(),
            _ => false,
        };
        if ready && self.hero.buffer.is_none() && self.demo_wait <= 0.0 {
            if self.demo_plan.is_empty() {
                let pick = self.rng.random_int(0, 3) as usize;
                self.demo_plan = weapon.combos[pick].1.iter().rev().copied().collect();
                self.demo_wait = 0.35;
                return;
            }
            if let Some(kind) = self.demo_plan.pop() {
                self.press(Input::Attack(kind));
            }
        }
    }

    // Drawing.

    fn draw_telegraphs(&self) {
        let warn = hex(0xff9a3c);
        for enemy in &self.enemies {
            let Mind::Windup { time, direction } = enemy.mind else {
                continue;
            };
            let progress = (time / enemy.foe.windup()).min(1.0);
            // The last moment before it lands flashes white.
            let color = if progress > 0.85 { Color::WHITE } else { warn };
            match enemy.foe {
                Foe::Wight => {
                    let side = direction.perp() * LUNGE_WIDTH;
                    let from = enemy.position;
                    let lane = |length: f32| {
                        let to = from + direction * length;
                        [from + side, to + side, to - side, from - side]
                    };
                    fill(&lane(LUNGE_LENGTH), warn.with_alpha(0.22));
                    fill(&lane(LUNGE_LENGTH * progress), color.with_alpha(0.5));
                    graphics::set_color(color.with_alpha(0.9));
                    graphics::polygon(DrawMode::Line, &lane(LUNGE_LENGTH));
                }
                Foe::Brute => {
                    let center = enemy.position + direction * SLAM_REACH;
                    graphics::set_color(warn.with_alpha(0.22));
                    graphics::circle(DrawMode::Fill, center, SLAM_RADIUS);
                    graphics::set_color(color.with_alpha(0.5));
                    graphics::circle(DrawMode::Fill, center, SLAM_RADIUS * progress);
                    graphics::set_color(color.with_alpha(0.9));
                    graphics::circle(DrawMode::Line, center, SLAM_RADIUS);
                }
                Foe::Caster => {
                    let from = enemy.position + direction * 8.0;
                    let side = direction.perp() * (0.5 + progress * 1.5);
                    let to = from + direction * 520.0;
                    fill(
                        &[from + side, to + side, to - side, from - side],
                        color.with_alpha(0.25 + progress * 0.5),
                    );
                }
            }
        }
    }

    fn draw_trail(center: Vec2, shape: Shape, from: f32, to: f32, radius: f32, color: Color) {
        let at = |angle: f32, radius: f32| {
            center + Vec2::new(angle.cos() * radius, angle.sin() * radius * 0.8)
        };
        match shape {
            Shape::Thrust => {
                let aim = Vec2::from_angle(from);
                let aim = Vec2::new(aim.x, aim.y * 0.8);
                let side = aim.perp().normalize_or_zero() * 2.5;
                let tip = center + aim * radius;
                let base = center + aim * 6.0;
                fill(&[base + side, tip, base - side], color);
            }
            Shape::Cross => {
                // Two blades closing on the middle from either side.
                let middle = (from + to) / 2.0;
                let half = (to - from).abs() / 2.0;
                let aim = middle + (from - middle).signum() * (half - (to - from).abs());
                for side in [-1.0, 1.0] {
                    let start = aim + side * (from - aim).abs();
                    Self::draw_trail(center, Shape::Arc, start, aim, radius, color);
                }
            }
            Shape::Arc | Shape::Spin => {
                // Only the last turn of a spin shows.
                let from = if (to - from).abs() > TAU {
                    to - TAU * (to - from).signum()
                } else {
                    from
                };
                let steps = 16;
                let mut outer = Vec::with_capacity(steps + 1);
                let mut inner = Vec::with_capacity(steps + 1);
                for step in 0..=steps {
                    let along = step as f32 / steps as f32;
                    let angle = from + (to - from) * along;
                    outer.push(at(angle, radius));
                    // Thin where the swing began and wide at its head.
                    inner.push(at(angle, radius * (1.0 - 0.45 * along)));
                }
                band(&outer, &inner, color);
            }
        }
    }

    fn draw_world(&self) {
        graphics::set_color(Color::WHITE);
        graphics::draw(&self.ground, (-8.0, -8.0));
        self.draw_telegraphs();

        for ring in &self.rings {
            let age = 1.0 - ring.life / 0.5;
            graphics::set_color(ring.color.with_alpha(ring.life * 2.5));
            graphics::set_line_width(2.0);
            graphics::ellipse(
                DrawMode::Line,
                ring.center,
                (
                    ring.radius * (0.6 + age * 0.5),
                    ring.radius * (0.6 + age * 0.5) * 0.8,
                ),
            );
            graphics::set_line_width(1.0);
        }

        // Shadows first, then everything standing on the field from the
        // back to the front.
        let shadow = hex(0x1b1836).with_alpha(0.3);
        oval(self.hero.position + Vec2::new(1.0, 1.0), 8.0, 2.4, shadow);
        for enemy in &self.enemies {
            let size = enemy.foe.radius() / 6.0;
            oval(
                enemy.position + Vec2::new(1.0, 1.0),
                8.0 * size,
                2.4 * size,
                shadow,
            );
        }
        for bolt in &self.bolts {
            oval(bolt.position, 3.0, 1.2, shadow);
        }

        enum Thing {
            Hero,
            Enemy(usize),
            Pine(usize),
        }
        let mut things = vec![(self.hero.position.y, Thing::Hero)];
        things
            .extend((0..self.enemies.len()).map(|i| (self.enemies[i].position.y, Thing::Enemy(i))));
        things.extend((0..self.pines.len()).map(|i| (self.pines[i].position.y, Thing::Pine(i))));
        things.sort_by(|a, b| a.0.total_cmp(&b.0));

        for ghost in &self.ghosts {
            let mut pose = ghost.pose;
            pose.flat = pose.flat.map(|color| color.with_alpha(ghost.life * 2.2));
            draw_fighter(&pose, &self.hero_colors);
        }
        for (_, thing) in things {
            match thing {
                Thing::Hero => {
                    let hero = &self.hero;
                    let mut pose = hero.pose(self.time);
                    pose.bones = self.bones;
                    // Blink while nothing can hurt the fighter.
                    let blink =
                        hero.safe > 0.0 && !hero.is_dead() && (self.time * 30.0).sin() > 0.3;
                    if hero.hurt > 0.2 {
                        pose.flat = Some(Color::WHITE);
                    }
                    if !blink {
                        draw_fighter(&pose, &self.hero_colors);
                    }
                }
                Thing::Enemy(index) => {
                    let enemy = &self.enemies[index];
                    let mut pose = enemy.pose(self.time);
                    pose.bones = self.bones;
                    if enemy.flash > 0.0 {
                        pose.flat = Some(Color::WHITE);
                    } else if let Mind::Spawn(left) = enemy.mind {
                        pose.flat = Some(enemy.foe.glow().with_alpha(1.0 - left));
                    }
                    draw_fighter(&pose, &self.foe_colors[enemy.foe as usize]);
                    if enemy.broken > 0.0 {
                        // Stars circle a brute whose guard is broken.
                        for star in 0..3 {
                            let angle = self.time * 6.0 + star as f32 * TAU / 3.0;
                            let at = enemy.position
                                + Vec2::new(angle.cos() * 8.0, -34.0 + angle.sin() * 2.5);
                            rect(at.x, at.y, 2.0, 2.0, hex(0xffb86c));
                        }
                    }
                }
                Thing::Pine(index) => self.draw_pine(&self.pines[index]),
            }
        }

        // The swing in the air right now, and the ones still fading.
        if let Action::Attack(swing) = &self.hero.action
            && swing.is_active()
        {
            let stats = &swing.stats;
            let start = swing.aim + swing.flip * stats.from;
            let (from, to) = match stats.shape {
                Shape::Thrust => (swing.aim, swing.aim),
                Shape::Cross => (start, swing.aim + swing.flip * stats.to),
                _ => (start, swing.angle()),
            };
            let radius = if stats.shape == Shape::Thrust {
                stats.radius * swing.progress().max(0.3)
            } else {
                stats.radius
            };
            let center = self.hero.position + CHEST;
            Self::draw_trail(center, stats.shape, from, to, radius, swing.kind.color());
        }
        for trail in &self.trails {
            let color = trail.color.with_alpha(trail.life / 0.14 * 0.8);
            Self::draw_trail(
                trail.center,
                trail.shape,
                trail.from,
                trail.to,
                trail.radius,
                color,
            );
        }

        for bolt in &self.bolts {
            let at = bolt.position + CHEST;
            let tail = bolt.velocity.normalize_or_zero() * -7.0;
            let (core, glow) = if bolt.deflected {
                (Color::WHITE, hex(0x49f0e2))
            } else {
                (hex(0xffd6fb), hex(0xff5cf0))
            };
            line(at, at + tail, glow.with_alpha(0.7));
            oval(at, 3.5, 3.5, glow);
            oval(at, 2.0, 2.0, core);
        }
        for particle in &self.particles {
            let color = particle
                .color
                .with_alpha((particle.life / particle.span).min(1.0));
            if particle.streak {
                let tail = particle.velocity * 0.03;
                line(particle.position, particle.position - tail, color);
            } else {
                rect(particle.position.x, particle.position.y, 2.0, 2.0, color);
            }
        }
        for flake in &self.flakes {
            rect(flake.x, flake.y, 1.0, 1.0, Color::WHITE.with_alpha(0.8));
        }
    }

    fn draw_pine(&self, pine: &Pine) {
        let base = pine.position;
        let height = pine.height;
        oval(
            base + Vec2::new(2.0, 0.0),
            height * 0.36,
            3.0,
            hex(0x1b1836).with_alpha(0.3),
        );
        rect(base.x - 1.0, base.y - 5.0, 3.0, 5.0, hex(0x3a2a3a));
        for tier in 0..3 {
            let tier = tier as f32;
            let bottom = base.y - 3.0 - tier * height * 0.26;
            let half = height * (0.34 - tier * 0.08);
            let top = bottom - height * 0.42;
            let apex = Vec2::new(base.x + 0.5, top);
            let outline = hex(0x1b1836);
            fill(
                &[
                    Vec2::new(base.x - half - 1.0, bottom + 1.0),
                    Vec2::new(base.x + half + 2.0, bottom + 1.0),
                    apex - Vec2::new(0.0, 1.5),
                ],
                outline,
            );
            fill(
                &[
                    Vec2::new(base.x - half, bottom),
                    Vec2::new(base.x + half + 1.0, bottom),
                    apex,
                ],
                hex(0x2b3566),
            );
            // Snow lies on the side the light comes from.
            fill(
                &[
                    Vec2::new(base.x - half, bottom),
                    Vec2::new(base.x, bottom),
                    apex,
                ],
                hex(0x4b5a9a),
            );
            let cap = |along: f32| apex + (Vec2::new(base.x - half, bottom) - apex) * along;
            fill(
                &[cap(0.55), Vec2::new(apex.x, cap(0.45).y), apex],
                hex(0xf6ecf1),
            );
            fill(
                &[
                    Vec2::new(base.x + half * 0.3, bottom),
                    Vec2::new(base.x + half + 1.0, bottom),
                    apex,
                ],
                hex(0x3d4a82),
            );
        }
    }

    fn text(&self, text: &str, position: Vec2, color: Color, scale: f32) {
        let ink = hex(0x1b1836).with_alpha(color.a);
        for (offset, color) in [(Vec2::new(scale, scale), ink), (Vec2::ZERO, color)] {
            graphics::set_color(color);
            graphics::print(text, DrawParams::at(position + offset).scale(scale));
        }
    }

    fn centered(&self, text: &str, position: Vec2, color: Color, scale: f32) {
        let width = 240.0;
        let ink = hex(0x1b1836).with_alpha(color.a);
        for (offset, color) in [(Vec2::new(scale, scale), ink), (Vec2::ZERO, color)] {
            graphics::set_color(color);
            let corner = position + offset - Vec2::new(width * scale / 2.0, 0.0);
            graphics::printf(
                text,
                DrawParams::at(corner.round()).scale(scale),
                width,
                Align::Center,
            );
        }
    }

    fn draw_hud(&self) {
        let hero = &self.hero;
        let ink = hex(0x1b1836);

        // Health as diamonds, and stamina as bars under them.
        for pip in 0..MAX_HEALTH {
            let center = Vec2::new(13.0 + pip as f32 * 11.0, 13.0);
            let diamond = |size: f32| {
                [
                    center + Vec2::new(0.0, -size),
                    center + Vec2::new(size, 0.0),
                    center + Vec2::new(0.0, size),
                    center + Vec2::new(-size, 0.0),
                ]
            };
            fill(&diamond(5.5), ink);
            let color = if pip < hero.health {
                hex(0xff5c8d)
            } else {
                hex(0x4b3f86)
            };
            fill(&diamond(4.0), color);
        }
        for pip in 0..MAX_STAMINA as i32 {
            let x = 8.0 + pip as f32 * 15.0;
            let level = (hero.stamina - pip as f32).clamp(0.0, 1.0);
            rect(x - 1.0, 21.0, 15.0, 5.0, ink);
            rect(x, 22.0, 13.0, 3.0, hex(0x4b3f86));
            if level > 0.0 {
                let color = if level >= 1.0 {
                    hex(0x49f0e2)
                } else {
                    hex(0x2f8f9a)
                };
                rect(x, 22.0, (13.0 * level).max(1.0), 3.0, color);
            }
        }

        // The weapon and its strings. The part of each string that the
        // attacks so far have followed is lit.
        let weapon = hero.weapon();
        let top = HEIGHT - 12.0 - weapon.combos.len() as f32 * 9.0;
        let panel = ink.with_alpha(0.72);
        rect(4.0, top - 24.0, 122.0, HEIGHT - top + 20.0, panel);
        rect(WIDTH - 58.0, 4.0, 54.0, 14.0, panel);
        self.text(weapon.name, Vec2::new(8.0, top - 10.0), hex(0xffd0a8), 1.0);
        for (row, (name, string)) in weapon.combos.iter().enumerate() {
            let y = top + row as f32 * 9.0;
            let followed = string.starts_with(&hero.string) && !hero.string.is_empty();
            for (column, kind) in string.iter().enumerate() {
                let lit = followed && column < hero.string.len();
                let color = if lit {
                    kind.color()
                } else {
                    kind.color().lerp(hex(0x4b3f86), 0.4)
                };
                self.text(
                    kind.letter(),
                    Vec2::new(8.0 + column as f32 * 7.0, y),
                    color,
                    1.0,
                );
            }
            let done = followed && hero.string.len() == string.len();
            let color = if done { Color::WHITE } else { hex(0xc6bbe4) };
            self.text(name, Vec2::new(40.0, y), color, 1.0);
        }

        // The name of the move being swung.
        if let Action::Attack(swing) = &hero.action {
            let at = Vec2::new(8.0, top - 20.0);
            self.text(swing.stats.name, at, swing.kind.color(), 1.0);
        }

        let wave = format!("WAVE {}", self.wave.max(1));
        self.text(&wave, Vec2::new(WIDTH - 52.0, 8.0), Color::WHITE, 1.0);
        if self.chain > 1 {
            let chain = format!("{} HITS", self.chain);
            let pulse = if self.chain_timer > 2.35 { 3.0 } else { 2.0 };
            let color = hex(0xffb86c);
            self.text(
                &chain,
                Vec2::new(WIDTH - 52.0 - pulse * 14.0, 20.0),
                color,
                pulse,
            );
        }

        for popup in &self.popups {
            let rise = (0.9 - popup.life) * 14.0;
            let color = popup.color.with_alpha((popup.life * 3.0).min(1.0));
            let position = popup.position - Vec2::new(0.0, rise);
            self.centered(&popup.text, position, color, popup.scale);
        }

        let hint = hex(0xf6ecf1);
        if self.demo || self.wave <= 1 {
            let width = if self.demo { 124.0 } else { 216.0 };
            let (x, y) = if self.demo {
                (WIDTH / 2.0 - 62.0, 4.0)
            } else {
                (WIDTH - 218.0, HEIGHT - 26.0)
            };
            rect(x, y, width, if self.demo { 14.0 } else { 22.0 }, panel);
        }
        if self.demo {
            self.centered("DEMO: TAB TO PLAY", Vec2::new(WIDTH / 2.0, 8.0), hint, 1.0);
        } else if self.wave <= 1 {
            let lines = if self.using_pad {
                ["X LIGHT   Y MEDIUM   B HEAVY", "A DASH   SHOULDERS: WEAPON"]
            } else {
                [
                    "MOUSE: LIGHT, MEDIUM   F: HEAVY",
                    "SPACE: DASH   Q, E: WEAPON",
                ]
            };
            for (row, text) in lines.iter().enumerate() {
                let y = HEIGHT - 22.0 + row as f32 * 9.0;
                self.centered(text, Vec2::new(WIDTH - 110.0, y), hint, 1.0);
            }
        }
        if hero.is_dead() {
            let middle = Vec2::new(WIDTH / 2.0, 104.0);
            self.centered("FALLEN", middle, hex(0xff5c8d), 4.0);
            let best = format!("WAVE {}   BEST CHAIN {}", self.wave, self.best_chain);
            self.centered(&best, middle + Vec2::new(0.0, 34.0), Color::WHITE, 1.0);
            let again = if self.using_pad {
                "START TO RISE AGAIN"
            } else {
                "ENTER TO RISE AGAIN"
            };
            self.centered(again, middle + Vec2::new(0.0, 46.0), Color::WHITE, 1.0);
        }
    }
}

/// The letters of the game's font, each five pixels wide and seven tall,
/// one row of pixels per number.
const GLYPHS: [(char, [u8; 7]); 41] = [
    ('A', [14, 17, 17, 31, 17, 17, 17]),
    ('B', [30, 17, 17, 30, 17, 17, 30]),
    ('C', [14, 17, 16, 16, 16, 17, 14]),
    ('D', [30, 17, 17, 17, 17, 17, 30]),
    ('E', [31, 16, 16, 30, 16, 16, 31]),
    ('F', [31, 16, 16, 30, 16, 16, 16]),
    ('G', [14, 17, 16, 23, 17, 17, 15]),
    ('H', [17, 17, 17, 31, 17, 17, 17]),
    ('I', [14, 4, 4, 4, 4, 4, 14]),
    ('J', [7, 2, 2, 2, 2, 18, 12]),
    ('K', [17, 18, 20, 24, 20, 18, 17]),
    ('L', [16, 16, 16, 16, 16, 16, 31]),
    ('M', [17, 27, 21, 21, 17, 17, 17]),
    ('N', [17, 25, 21, 19, 17, 17, 17]),
    ('O', [14, 17, 17, 17, 17, 17, 14]),
    ('P', [30, 17, 17, 30, 16, 16, 16]),
    ('Q', [14, 17, 17, 17, 21, 18, 13]),
    ('R', [30, 17, 17, 30, 20, 18, 17]),
    ('S', [15, 16, 16, 14, 1, 1, 30]),
    ('T', [31, 4, 4, 4, 4, 4, 4]),
    ('U', [17, 17, 17, 17, 17, 17, 14]),
    ('V', [17, 17, 17, 17, 17, 10, 4]),
    ('W', [17, 17, 17, 21, 21, 27, 17]),
    ('X', [17, 17, 10, 4, 10, 17, 17]),
    ('Y', [17, 17, 10, 4, 4, 4, 4]),
    ('Z', [31, 1, 2, 4, 8, 16, 31]),
    ('0', [14, 17, 19, 21, 25, 17, 14]),
    ('1', [4, 12, 4, 4, 4, 4, 14]),
    ('2', [14, 17, 1, 2, 4, 8, 31]),
    ('3', [30, 1, 1, 14, 1, 1, 30]),
    ('4', [2, 6, 10, 18, 31, 2, 2]),
    ('5', [31, 16, 30, 1, 1, 17, 14]),
    ('6', [6, 8, 16, 30, 17, 17, 14]),
    ('7', [31, 1, 2, 4, 8, 8, 8]),
    ('8', [14, 17, 17, 14, 17, 17, 14]),
    ('9', [14, 17, 17, 15, 1, 2, 12]),
    (' ', [0, 0, 0, 0, 0, 0, 0]),
    (':', [0, 4, 4, 0, 4, 4, 0]),
    (',', [0, 0, 0, 0, 12, 4, 8]),
    ('.', [0, 0, 0, 0, 0, 12, 12]),
    ('-', [0, 0, 0, 31, 0, 0, 0]),
];

/// Builds the font from [`GLYPHS`] as a bitmap font: the letters in a row
/// with a column of another color between them.
fn make_font() -> smitten::Result<Font> {
    let between = Color::rgb(1.0, 0.0, 1.0);
    let mut sheet = ImageData::new(GLYPHS.len() as u32 * 6 + 1, 7);
    sheet.map_pixels(|x, y, _| {
        let (_, rows) = GLYPHS[(x.saturating_sub(1) / 6) as usize % GLYPHS.len()];
        let column = x.wrapping_sub(1) % 6;
        if x % 6 == 0 {
            between
        } else if rows[y as usize] >> (4 - column) & 1 == 1 {
            Color::WHITE
        } else {
            Color::rgba(0.0, 0.0, 0.0, 0.0)
        }
    });
    let letters: String = GLYPHS.iter().map(|(letter, _)| *letter).collect();
    let font = Font::from_image(&sheet.encode_png()?, &letters, 1.0)?;
    font.set_filter(Filter::Nearest);
    Ok(font)
}

/// Paints the snowfield: three tones of snow from smooth noise, with a few
/// bright specks.
fn paint_ground(rng: &mut RandomGenerator) -> Image {
    let (width, height) = (WIDTH as u32 + 16, HEIGHT as u32 + 16);
    let tones = [hex(0xf6ecf1), hex(0xe4d9ee), hex(0xcfc3e6)];
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32, y as f32);
            let drift =
                math::noise2(fx / 50.0, fy / 34.0) * 0.7 + math::noise2(fx / 13.0, fy / 9.0) * 0.3;
            let mut color = if drift > 0.56 {
                tones[0]
            } else if drift > 0.44 {
                tones[1]
            } else {
                tones[2]
            };
            if rng.chance(0.004) {
                color = Color::WHITE;
            } else if rng.chance(0.003) {
                color = hex(0xb9addf);
            }
            pixels.extend_from_slice(&color.to_rgba8());
        }
    }
    Image::from_rgba(width, height, &pixels).expect("the snowfield is a valid image")
}

impl Game for Frostmarch {
    fn load() -> smitten::Result<Self> {
        graphics::set_default_filter(Filter::Nearest);
        graphics::set_background_color(hex(0x1b1836));
        mouse::set_visible(true);
        let font = make_font()?;

        let mut rng = RandomGenerator::new(4);
        let ground = paint_ground(&mut rng);
        // A line of pines along the back of the field and a few at the
        // front, where they frame the fight without getting in its way.
        let mut pines = Vec::new();
        let mut x = -4.0;
        while x < WIDTH + 10.0 {
            pines.push(Pine {
                position: Vec2::new(x, rng.random_range(26.0, 44.0)),
                height: rng.random_range(26.0, 40.0),
            });
            x += rng.random_range(13.0, 30.0);
        }
        for x in [-2.0, 30.0, 452.0, 478.0] {
            pines.push(Pine {
                position: Vec2::new(x, HEIGHT + rng.random_range(2.0, 12.0)),
                height: rng.random_range(34.0, 44.0),
            });
        }
        let flakes = (0..60)
            .map(|_| Vec2::new(rng.random_range(0.0, WIDTH), rng.random_range(0.0, HEIGHT)))
            .collect();

        Ok(Frostmarch {
            hero: Hero::new(),
            enemies: Vec::new(),
            bolts: Vec::new(),
            particles: Vec::new(),
            trails: Vec::new(),
            rings: Vec::new(),
            popups: Vec::new(),
            ghosts: Vec::new(),
            flakes,
            pines,
            rng,
            sounds: Sounds::new(),
            ground,
            font,
            hero_colors: Palette::outlander(),
            foe_colors: [Palette::wight(), Palette::brute(), Palette::caster()],
            time: 0.0,
            wave: 0,
            wave_timer: 1.0,
            hit_stop: 0.0,
            trauma: 0.0,
            flash: 0.0,
            chain: 0,
            chain_timer: 0.0,
            best_chain: 0,
            next_id: 0,
            pad: joystick::gamepads().into_iter().next(),
            using_pad: false,
            bones: false,
            demo: std::env::args().any(|argument| argument == "demo"),
            demo_move: Vec2::ZERO,
            demo_plan: Vec::new(),
            demo_wait: 0.0,
            dead_time: 0.0,
        })
    }

    fn update(&mut self, dt: f32) {
        let dt = dt.min(1.0 / 30.0);
        self.trauma = (self.trauma - dt * 1.8).max(0.0);
        self.flash = (self.flash - dt * 4.0).max(0.0);
        for popup in &mut self.popups {
            popup.life -= dt;
        }
        self.popups.retain(|popup| popup.life > 0.0);
        if self.hero.is_dead() {
            self.dead_time += dt;
        }

        // A hit that lands freezes everything for a moment.
        if self.hit_stop > 0.0 {
            self.hit_stop -= dt;
            return;
        }
        self.time += dt;
        if self.demo {
            self.play_demo(dt);
        }
        self.update_hero(dt);
        self.update_enemies(dt);
        self.update_bolts(dt);
        self.update_effects(dt);

        if self.enemies.is_empty() && !self.hero.is_dead() {
            self.wave_timer -= dt;
            if self.wave_timer <= 0.0 {
                self.spawn_wave();
                self.wave_timer = 1.6;
            }
        }
    }

    fn draw(&mut self) {
        graphics::set_font(&self.font);
        graphics::set_line_style(LineStyle::Rough);
        let shake = self.trauma * self.trauma * 6.0;
        let jolt = Vec2::new(
            (math::noise(self.time * 40.0) - 0.5) * 2.0 * shake,
            (math::noise(self.time * 40.0 + 70.0) - 0.5) * 2.0 * shake,
        );
        graphics::scoped(|| {
            graphics::translate(jolt.round());
            self.draw_world();
        });
        if self.flash > 0.0 {
            graphics::set_color(hex(0xff5c8d).with_alpha(self.flash * 0.25));
            graphics::rectangle(DrawMode::Fill, (0.0, 0.0), (WIDTH, HEIGHT));
        }
        self.draw_hud();
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        if is_repeat {
            return;
        }
        if !matches!(key, Key::Tab | Key::F11 | Key::Escape | Key::B) {
            self.using_pad = false;
            self.demo = false;
        }
        let weapons = WEAPONS.len();
        match key {
            Key::Space => self.press(Input::Dash),
            Key::J => self.press(Input::Attack(Kind::Light)),
            Key::K => self.press(Input::Attack(Kind::Medium)),
            Key::L | Key::F => self.press(Input::Attack(Kind::Heavy)),
            Key::Q => self.change_weapon((self.hero.weapon + weapons - 1) % weapons),
            Key::E => self.change_weapon((self.hero.weapon + 1) % weapons),
            Key::Num1 => self.change_weapon(0),
            Key::Num2 => self.change_weapon(1),
            Key::Num3 => self.change_weapon(2),
            Key::Num4 => self.change_weapon(3),
            Key::Return | Key::KpEnter if self.hero.is_dead() => self.restart(),
            Key::B => self.bones = !self.bones,
            Key::Tab => self.demo = !self.demo,
            Key::F11 => window::set_fullscreen(!window::fullscreen()),
            Key::Escape => event::quit(),
            _ => {}
        }
    }

    fn mouse_pressed(&mut self, click: MouseClick) {
        self.using_pad = false;
        self.demo = false;
        let shift = keyboard::is_any_down(&[Key::LeftShift, Key::RightShift]);
        let kind = match click.button {
            MouseButton::Left if shift => Kind::Heavy,
            MouseButton::Left => Kind::Light,
            MouseButton::Right => Kind::Medium,
            MouseButton::Middle => Kind::Heavy,
            _ => return,
        };
        self.press(Input::Attack(kind));
    }

    fn gamepad_pressed(&mut self, pad: Joystick, button: GamepadButton) {
        self.pad = Some(pad);
        self.using_pad = true;
        if button != GamepadButton::Back {
            self.demo = false;
        }
        let weapons = WEAPONS.len();
        match button {
            GamepadButton::A => self.press(Input::Dash),
            GamepadButton::X => self.press(Input::Attack(Kind::Light)),
            GamepadButton::Y => self.press(Input::Attack(Kind::Medium)),
            GamepadButton::B => self.press(Input::Attack(Kind::Heavy)),
            GamepadButton::LeftShoulder => {
                self.change_weapon((self.hero.weapon + weapons - 1) % weapons);
            }
            GamepadButton::RightShoulder => self.change_weapon((self.hero.weapon + 1) % weapons),
            GamepadButton::Start if self.hero.is_dead() => self.restart(),
            GamepadButton::Back => self.demo = !self.demo,
            _ => {}
        }
    }

    fn joystick_added(&mut self, pad: Joystick) {
        if self.pad.is_none() && pad.is_gamepad() {
            self.pad = Some(pad);
        }
    }

    fn joystick_removed(&mut self, pad: Joystick) {
        if self.pad.as_ref().is_some_and(|mine| mine.id() == pad.id()) {
            self.pad = None;
            self.using_pad = false;
        }
    }
}

fn main() {
    smitten::run::<Frostmarch>(Config {
        title: "Frostmarch".to_string(),
        width: 1440,
        height: 810,
        resizable: true,
        design_size: Some((WIDTH as u32, HEIGHT as u32)),
        design_scaling: Scaling::PixelPerfect,
        ..Config::default()
    });
}
