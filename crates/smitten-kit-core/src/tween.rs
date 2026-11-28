//! Easing curves, tweens, and frame-rate independent smoothing.
//!
//! - [`Tween`] animates a value from one place to another over a set
//!   time, along an [`Ease`] curve.
//! - [`approach`] chases a moving target, closing half the gap every
//!   `half_life` seconds, the same at 30 and 144 frames a second.
//! - [`Spring`] chases a target with a critically damped spring: it picks
//!   up the current speed when the target moves, and never overshoots.
//!
//! Time is `f32` seconds by default, as smitten's `dt` is. Everything here
//! also works in `f64` for a game that keeps its clock in doubles: the
//! value type picks which (`Tween<f64, f64>`, `Spring<DVec2, f64>`).
//!
//! ```
//! use smitten_kit_core::tween::{Ease, Tween};
//!
//! let mut fade = Tween::new(0.0_f32, 1.0, 0.5, Ease::OutQuad);
//! fade.update(0.25);
//! assert!(fade.value() > 0.5); // OutQuad is past halfway at half time
//! assert!(fade.update(0.25)); // true on the frame it finishes
//! assert_eq!(fade.value(), 1.0);
//!
//! // The same in f64.
//! let mut slide = Tween::new(0.0_f64, 100.0, 2.0, Ease::Linear);
//! slide.update(0.5);
//! assert_eq!(slide.value(), 25.0);
//! ```

use std::f64::consts::PI;
use std::ops::{Add, Mul, Sub};

use glam::{DVec2, DVec3, Vec2, Vec3};

use crate::float::Float;

/// A value that can be blended between two others, with `t` of type `F`.
pub trait Lerp<F: Float = f32>: Copy {
    /// `self` at 0.0, `to` at 1.0. Values of `t` outside that range carry
    /// on past the ends, so overshooting curves like [`Ease::OutBack`]
    /// work.
    fn lerp(self, to: Self, t: F) -> Self;
}

impl Lerp<f32> for f32 {
    fn lerp(self, to: f32, t: f32) -> f32 {
        self + (to - self) * t
    }
}

impl Lerp<f64> for f64 {
    fn lerp(self, to: f64, t: f64) -> f64 {
        self + (to - self) * t
    }
}

impl Lerp<f32> for Vec2 {
    fn lerp(self, to: Vec2, t: f32) -> Vec2 {
        self + (to - self) * t
    }
}

impl Lerp<f32> for Vec3 {
    fn lerp(self, to: Vec3, t: f32) -> Vec3 {
        self + (to - self) * t
    }
}

impl Lerp<f64> for DVec2 {
    fn lerp(self, to: DVec2, t: f64) -> DVec2 {
        self + (to - self) * t
    }
}

impl Lerp<f64> for DVec3 {
    fn lerp(self, to: DVec3, t: f64) -> DVec3 {
        self + (to - self) * t
    }
}

/// A point or size as a pair, as `(x, y)`.
impl<F: Float + Lerp<F>> Lerp<F> for (F, F) {
    fn lerp(self, to: (F, F), t: F) -> (F, F) {
        (self.0.lerp(to.0, t), self.1.lerp(to.1, t))
    }
}

/// Each element blended alone, as for `[r, g, b, a]` colors.
impl<F: Float + Lerp<F>, const N: usize> Lerp<F> for [F; N] {
    fn lerp(self, to: [F; N], t: F) -> [F; N] {
        std::array::from_fn(|i| self[i].lerp(to[i], t))
    }
}

#[cfg(feature = "color")]
impl Lerp<f32> for smitten_core::Color {
    fn lerp(self, to: smitten_core::Color, t: f32) -> smitten_core::Color {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        smitten_core::Color::rgba(
            mix(self.r, to.r),
            mix(self.g, to.g),
            mix(self.b, to.b),
            mix(self.a, to.a),
        )
    }
}

/// An easing curve: how a tween's progress maps to how far along its
/// value is. See <https://easings.net> for pictures of each.
#[derive(Clone, Copy, Debug)]
#[allow(missing_docs)] // The names are the standard ones.
pub enum Ease {
    Linear,
    InQuad,
    OutQuad,
    InOutQuad,
    InCubic,
    OutCubic,
    InOutCubic,
    InSine,
    OutSine,
    InOutSine,
    InExpo,
    OutExpo,
    InOutExpo,
    InBack,
    OutBack,
    InOutBack,
    OutElastic,
    OutBounce,
    /// A curve of the game's own. It should map 0.0 to 0.0 and 1.0 to 1.0.
    Custom(fn(f64) -> f64),
}

impl Ease {
    /// Where the curve is at `t`, from 0.0 to 1.0. `t` is cut to that
    /// range first; the result may go outside it for the Back and Elastic
    /// curves.
    pub fn apply<F: Float>(self, t: F) -> F {
        F::from_f64(self.apply_f64(t.to_f64()))
    }

    fn apply_f64(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        const BACK: f64 = 1.70158;
        match self {
            Ease::Linear => t,
            Ease::InQuad => t * t,
            Ease::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Ease::InOutQuad => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            Ease::InCubic => t * t * t,
            Ease::OutCubic => 1.0 - (1.0 - t).powi(3),
            Ease::InOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Ease::InSine => 1.0 - (t * PI / 2.0).cos(),
            Ease::OutSine => (t * PI / 2.0).sin(),
            Ease::InOutSine => -((PI * t).cos() - 1.0) / 2.0,
            Ease::InExpo => {
                if t == 0.0 {
                    0.0
                } else {
                    2f64.powf(10.0 * t - 10.0)
                }
            }
            Ease::OutExpo => {
                if t == 1.0 {
                    1.0
                } else {
                    1.0 - 2f64.powf(-10.0 * t)
                }
            }
            Ease::InOutExpo => {
                if t == 0.0 || t == 1.0 {
                    t
                } else if t < 0.5 {
                    2f64.powf(20.0 * t - 10.0) / 2.0
                } else {
                    (2.0 - 2f64.powf(-20.0 * t + 10.0)) / 2.0
                }
            }
            Ease::InBack => (BACK + 1.0) * t * t * t - BACK * t * t,
            Ease::OutBack => {
                let u = t - 1.0;
                1.0 + (BACK + 1.0) * u * u * u + BACK * u * u
            }
            Ease::InOutBack => {
                let c = BACK * 1.525;
                if t < 0.5 {
                    (2.0 * t).powi(2) * ((c + 1.0) * 2.0 * t - c) / 2.0
                } else {
                    ((2.0 * t - 2.0).powi(2) * ((c + 1.0) * (t * 2.0 - 2.0) + c) + 2.0) / 2.0
                }
            }
            Ease::OutElastic => {
                if t == 0.0 || t == 1.0 {
                    t
                } else {
                    2f64.powf(-10.0 * t) * ((t * 10.0 - 0.75) * (2.0 * PI / 3.0)).sin() + 1.0
                }
            }
            Ease::OutBounce => out_bounce(t),
            Ease::Custom(curve) => curve(t),
        }
    }
}

fn out_bounce(t: f64) -> f64 {
    const N: f64 = 7.5625;
    const D: f64 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let t = t - 1.5 / D;
        N * t * t + 0.75
    } else if t < 2.5 / D {
        let t = t - 2.25 / D;
        N * t * t + 0.9375
    } else {
        let t = t - 2.625 / D;
        N * t * t + 0.984375
    }
}

/// What a tween does when it reaches the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Repeat {
    /// Stop at the end.
    #[default]
    Once,
    /// Jump back to the start and play again, forever.
    Loop,
    /// Play backwards to the start, then forwards again, forever.
    PingPong,
}

/// A value animated from `from` to `to` over a set number of seconds.
///
/// A tween is a plain value: keep it in a field, call
/// [`update`](Tween::update) each frame, and read
/// [`value`](Tween::value) when drawing.
#[derive(Clone, Copy, Debug)]
pub struct Tween<T: Lerp<F>, F: Float = f32> {
    from: T,
    to: T,
    duration: F,
    elapsed: F,
    delay: F,
    ease: Ease,
    repeat: Repeat,
}

impl<T: Lerp<F>, F: Float> Tween<T, F> {
    /// A tween from `from` to `to` lasting `seconds`.
    pub fn new(from: T, to: T, seconds: F, ease: Ease) -> Self {
        Tween {
            from,
            to,
            duration: seconds.max(F::ZERO),
            elapsed: F::ZERO,
            delay: F::ZERO,
            ease,
            repeat: Repeat::Once,
        }
    }

    /// A tween that has already finished at `value`, for a field that
    /// needs a starting value before anything animates it.
    pub fn done(value: T) -> Self {
        Tween::new(value, value, F::ZERO, Ease::Linear)
    }

    /// The same tween, repeating as `repeat` says.
    pub fn repeat(mut self, repeat: Repeat) -> Self {
        self.repeat = repeat;
        self
    }

    /// The same tween, holding at `from` for `seconds` before it starts.
    pub fn delay(mut self, seconds: F) -> Self {
        self.delay = seconds.max(F::ZERO);
        self
    }

    /// Moves the tween on by `dt` seconds. Returns true on the frame a
    /// [`Repeat::Once`] tween finishes, and on each frame a repeating one
    /// wraps around.
    pub fn update(&mut self, dt: F) -> bool {
        let mut dt = dt.max(F::ZERO);
        if self.delay > F::ZERO {
            let used = dt.min(self.delay);
            self.delay -= used;
            dt -= used;
        }
        if self.repeat == Repeat::Once && self.is_finished() {
            return false;
        }
        self.elapsed += dt;
        match self.repeat {
            Repeat::Once => self.elapsed >= self.duration,
            Repeat::Loop | Repeat::PingPong => {
                let period = match self.repeat {
                    Repeat::PingPong => self.duration * F::TWO,
                    _ => self.duration,
                };
                if period <= F::ZERO {
                    return false;
                }
                let wrapped = self.elapsed >= period;
                self.elapsed = self.elapsed % period;
                wrapped
            }
        }
    }

    /// How far along the tween is, from 0.0 to 1.0, before easing.
    pub fn progress(&self) -> F {
        if self.duration <= F::ZERO {
            return F::ONE;
        }
        let t = self.elapsed / self.duration;
        match self.repeat {
            Repeat::PingPong if t > F::ONE => F::TWO - t,
            _ => t.min(F::ONE),
        }
    }

    /// The value now.
    pub fn value(&self) -> T {
        self.from.lerp(self.to, self.ease.apply(self.progress()))
    }

    /// Whether a [`Repeat::Once`] tween has reached the end. A repeating
    /// tween never finishes.
    pub fn is_finished(&self) -> bool {
        self.repeat == Repeat::Once && self.delay <= F::ZERO && self.elapsed >= self.duration
    }

    /// Where the tween is heading.
    pub fn target(&self) -> T {
        self.to
    }

    /// Starts over from the current value toward `to`, taking the full
    /// duration again, so a change of target never makes the value jump.
    pub fn retarget(&mut self, to: T) {
        self.from = self.value();
        self.to = to;
        self.elapsed = F::ZERO;
    }

    /// Goes back to the start.
    pub fn restart(&mut self) {
        self.elapsed = F::ZERO;
    }
}

/// Moves `current` toward `target`, closing half of the gap every
/// `half_life` seconds.
///
/// This is the frame-rate independent form of the familiar
/// `x += (target - x) * rate * dt`, which drifts differently at 30 and 144
/// frames a second. A `half_life` of 0.0 or less snaps to the target.
pub fn approach<T: Lerp<F>, F: Float>(current: T, target: T, half_life: F, dt: F) -> T {
    if half_life <= F::ZERO {
        return target;
    }
    let t = F::ONE - F::HALF.powf(dt.max(F::ZERO) / half_life);
    current.lerp(target, t)
}

/// A value a [`Spring`] can move: one that adds, subtracts and scales.
pub trait SpringValue<F: Float = f32>:
    Lerp<F> + Add<Output = Self> + Sub<Output = Self> + Mul<F, Output = Self> + PartialEq
{
    /// Zero.
    const ZERO: Self;
    /// The dot product, used to notice a step past the target.
    fn dot(self, other: Self) -> F;
}

impl SpringValue<f32> for f32 {
    const ZERO: f32 = 0.0;
    fn dot(self, other: f32) -> f32 {
        self * other
    }
}

impl SpringValue<f64> for f64 {
    const ZERO: f64 = 0.0;
    fn dot(self, other: f64) -> f64 {
        self * other
    }
}

impl SpringValue<f32> for Vec2 {
    const ZERO: Vec2 = Vec2::ZERO;
    fn dot(self, other: Vec2) -> f32 {
        Vec2::dot(self, other)
    }
}

impl SpringValue<f32> for Vec3 {
    const ZERO: Vec3 = Vec3::ZERO;
    fn dot(self, other: Vec3) -> f32 {
        Vec3::dot(self, other)
    }
}

impl SpringValue<f64> for DVec2 {
    const ZERO: DVec2 = DVec2::ZERO;
    fn dot(self, other: DVec2) -> f64 {
        DVec2::dot(self, other)
    }
}

impl SpringValue<f64> for DVec3 {
    const ZERO: DVec3 = DVec3::ZERO;
    fn dot(self, other: DVec3) -> f64 {
        DVec3::dot(self, other)
    }
}

/// A value that chases a target on a critically damped spring.
///
/// A move starts gently, lands softly and never overshoots, and a new
/// target mid-move carries on from the current speed instead of snapping.
/// `smooth_time` is roughly the seconds it takes to arrive.
///
/// ```
/// use smitten_kit_core::tween::Spring;
///
/// let mut x = Spring::new(0.0_f32, 0.3);
/// x.set_target(100.0);
/// for _ in 0..120 {
///     x.update(1.0 / 60.0);
/// }
/// assert!((x.value() - 100.0).abs() < 0.01);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Spring<T: SpringValue<F>, F: Float = f32> {
    value: T,
    target: T,
    velocity: T,
    smooth_time: F,
}

impl<T: SpringValue<F>, F: Float> Spring<T, F> {
    /// A spring resting at `value`.
    pub fn new(value: T, smooth_time: F) -> Self {
        Spring {
            value,
            target: value,
            velocity: T::ZERO,
            smooth_time,
        }
    }

    /// Where the spring is now.
    pub fn value(&self) -> T {
        self.value
    }

    /// Where the spring is heading.
    pub fn target(&self) -> T {
        self.target
    }

    /// How fast the value is moving, per second.
    pub fn velocity(&self) -> T {
        self.velocity
    }

    /// Roughly the seconds it takes to arrive.
    pub fn smooth_time(&self) -> F {
        self.smooth_time
    }

    /// Changes how quickly the spring arrives.
    pub fn set_smooth_time(&mut self, seconds: F) {
        self.smooth_time = seconds;
    }

    /// Heads for `target`, keeping the current speed.
    pub fn set_target(&mut self, target: T) {
        self.target = target;
    }

    /// Moves the value to `value` without changing the target or the
    /// speed, as when something else has moved what the spring drives.
    pub fn set_value(&mut self, value: T) {
        self.value = value;
    }

    /// Jumps straight to `value` and stops there.
    pub fn snap_to(&mut self, value: T) {
        self.value = value;
        self.target = value;
        self.velocity = T::ZERO;
    }

    /// Moves the value on by `dt` seconds and returns it.
    pub fn update(&mut self, dt: F) -> T {
        if self.smooth_time <= F::ZERO {
            self.snap_to(self.target);
            return self.value;
        }
        // Game Programming Gems 4, "Critically Damped Ease-In/Ease-Out
        // Smoothing", as Unity's SmoothDamp.
        let dt = dt.max(F::ZERO);
        let omega = F::TWO / self.smooth_time;
        let x = omega * dt;
        let decay =
            F::ONE / (F::ONE + x + F::from_f64(0.48) * x * x + F::from_f64(0.235) * x * x * x);
        let change = self.value - self.target;
        let temp = (self.velocity + change * omega) * dt;
        let before = self.value;
        self.velocity = (self.velocity - temp * omega) * decay;
        let out = self.target + (change + temp) * decay;
        // Never step past the target.
        if (self.target - before).dot(out - self.target) > F::ZERO {
            self.value = self.target;
            self.velocity = T::ZERO;
        } else {
            self.value = out;
        }
        self.value
    }

    /// Whether the value is within `tolerance` of the target and nearly
    /// still.
    pub fn is_settled(&self, tolerance: F) -> bool {
        let gap = self.value - self.target;
        gap.dot(gap) <= tolerance * tolerance
            && self.velocity.dot(self.velocity) <= tolerance * tolerance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Ease; 18] = [
        Ease::Linear,
        Ease::InQuad,
        Ease::OutQuad,
        Ease::InOutQuad,
        Ease::InCubic,
        Ease::OutCubic,
        Ease::InOutCubic,
        Ease::InSine,
        Ease::OutSine,
        Ease::InOutSine,
        Ease::InExpo,
        Ease::OutExpo,
        Ease::InOutExpo,
        Ease::InBack,
        Ease::OutBack,
        Ease::InOutBack,
        Ease::OutElastic,
        Ease::OutBounce,
    ];

    #[test]
    fn every_curve_starts_at_zero_and_ends_at_one() {
        for ease in ALL {
            assert!(ease.apply(0.0_f64).abs() < 1e-3, "{ease:?} at 0");
            assert!((ease.apply(1.0_f64) - 1.0).abs() < 1e-3, "{ease:?} at 1");
        }
    }

    #[test]
    fn in_out_curves_are_halfway_at_half_time() {
        for ease in [
            Ease::Linear,
            Ease::InOutQuad,
            Ease::InOutCubic,
            Ease::InOutSine,
        ] {
            assert!((ease.apply(0.5_f32) - 0.5).abs() < 1e-4, "{ease:?}");
        }
    }

    #[test]
    fn out_back_overshoots() {
        let peak = (0..100)
            .map(|i| Ease::OutBack.apply(i as f32 / 100.0))
            .fold(0.0, f32::max);
        assert!(peak > 1.05);
    }

    #[test]
    fn custom_curves_run() {
        let ease = Ease::Custom(|t| t * t * t * t);
        assert_eq!(ease.apply(0.5_f64), 0.0625);
    }

    #[test]
    fn tween_finishes_once() {
        let mut t = Tween::new(0.0_f32, 10.0, 1.0, Ease::Linear);
        assert!(!t.update(0.5));
        assert_eq!(t.value(), 5.0);
        assert!(t.update(0.6));
        assert_eq!(t.value(), 10.0);
        assert!(t.is_finished());
        assert!(!t.update(0.1), "reports finishing only once");
    }

    #[test]
    fn tween_waits_out_its_delay() {
        let mut t = Tween::new(0.0_f32, 1.0, 1.0, Ease::Linear).delay(0.5);
        t.update(0.5);
        assert_eq!(t.value(), 0.0);
        t.update(0.5);
        assert_eq!(t.value(), 0.5);
    }

    #[test]
    fn ping_pong_comes_back() {
        let mut t = Tween::new(0.0_f32, 1.0, 1.0, Ease::Linear).repeat(Repeat::PingPong);
        t.update(1.5);
        assert!((t.value() - 0.5).abs() < 1e-5);
        assert!(t.update(0.6), "wraps after a full there-and-back");
        assert!((t.value() - 0.1).abs() < 1e-4);
        assert!(!t.is_finished());
    }

    #[test]
    fn retarget_starts_from_where_it_is() {
        let mut t = Tween::new(0.0_f32, 10.0, 1.0, Ease::Linear);
        t.update(0.5);
        t.retarget(0.0);
        assert_eq!(t.value(), 5.0);
        t.update(0.5);
        assert_eq!(t.value(), 2.5);
    }

    #[test]
    fn tweens_pairs_and_arrays_in_f64() {
        let mut pos = Tween::new((0.0_f64, 0.0), (10.0, -20.0), 2.0, Ease::Linear);
        pos.update(1.0);
        assert_eq!(pos.value(), (5.0, -10.0));
        let mut color = Tween::new(
            [1.0_f64, 0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 0.0],
            1.0,
            Ease::Linear,
        );
        color.update(0.25);
        assert_eq!(color.value(), [0.75, 0.0, 0.25, 0.75]);
    }

    #[test]
    fn approach_is_frame_rate_independent() {
        let mut slow = 0.0_f32;
        for _ in 0..30 {
            slow = approach(slow, 100.0, 0.25, 1.0 / 30.0);
        }
        let mut fast = 0.0_f64;
        for _ in 0..144 {
            fast = approach(fast, 100.0, 0.25, 1.0 / 144.0);
        }
        // One second is four half-lives: 15/16 of the way.
        assert!((slow - 93.75).abs() < 0.01);
        assert!((fast - 93.75).abs() < 1e-9);
    }

    #[test]
    fn spring_never_overshoots() {
        let mut s = Spring::new(Vec2::ZERO, 0.3);
        s.set_target(Vec2::new(100.0, -50.0));
        let mut furthest = 0.0_f32;
        for _ in 0..300 {
            let v = s.update(1.0 / 60.0);
            furthest = furthest.max(v.x);
            assert!(v.y >= -50.0 - 1e-3);
        }
        assert!(furthest <= 100.0 + 1e-3);
        assert!(s.is_settled(0.01));
    }

    #[test]
    fn spring_works_in_f64() {
        let mut s = Spring::new(DVec2::ZERO, 0.32);
        s.set_target(DVec2::new(1000.0, 500.0));
        for _ in 0..600 {
            s.update(1.0 / 60.0);
        }
        assert!(s.is_settled(1e-6));
    }

    #[test]
    fn spring_keeps_its_speed_when_retargeted() {
        let mut s = Spring::new(0.0_f32, 0.3);
        s.set_target(100.0);
        for _ in 0..10 {
            s.update(1.0 / 60.0);
        }
        let speed = s.velocity();
        s.set_target(200.0);
        assert_eq!(s.velocity(), speed);
        assert!(speed > 0.0);
    }
}
