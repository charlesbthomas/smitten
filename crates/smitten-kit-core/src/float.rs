//! [`Float`]: what lets timers, tweens and sequences count time in `f32`
//! (as smitten's `dt` is) or `f64` (as a game that keeps its own clock in
//! doubles does).

use std::fmt::Debug;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Rem, Sub, SubAssign};

/// `f32` or `f64`.
///
/// Implemented for exactly those two; it is not meant for other types.
pub trait Float:
    Copy
    + Debug
    + Default
    + PartialOrd
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Rem<Output = Self>
    + Neg<Output = Self>
    + AddAssign
    + SubAssign
    + Send
    + Sync
    + 'static
{
    /// 0.0
    const ZERO: Self;
    /// 1.0
    const ONE: Self;
    /// 0.5
    const HALF: Self;
    /// 2.0
    const TWO: Self;
    /// The type's machine epsilon.
    const EPSILON: Self;
    /// Positive infinity.
    const INFINITY: Self;

    /// Converts from `f64`, rounding for `f32`.
    fn from_f64(value: f64) -> Self;
    /// Converts to `f64`.
    fn to_f64(self) -> f64;
    /// The square root.
    fn sqrt(self) -> Self;
    /// `self` raised to `power`.
    fn powf(self, power: Self) -> Self;
    /// e raised to `self`.
    fn exp(self) -> Self;
    /// The natural logarithm.
    fn ln(self) -> Self;
    /// The absolute value.
    fn abs(self) -> Self;
    /// Whether this is neither infinite nor NaN.
    fn is_finite(self) -> bool;

    /// The smaller of the two.
    fn min(self, other: Self) -> Self {
        if other < self { other } else { self }
    }

    /// The larger of the two.
    fn max(self, other: Self) -> Self {
        if other > self { other } else { self }
    }

    /// Kept between `low` and `high`.
    fn clamp(self, low: Self, high: Self) -> Self {
        self.max(low).min(high)
    }
}

macro_rules! float {
    ($t:ty) => {
        impl Float for $t {
            const ZERO: $t = 0.0;
            const ONE: $t = 1.0;
            const HALF: $t = 0.5;
            const TWO: $t = 2.0;
            const EPSILON: $t = <$t>::EPSILON;
            const INFINITY: $t = <$t>::INFINITY;

            #[allow(clippy::cast_possible_truncation, trivial_numeric_casts)]
            fn from_f64(value: f64) -> $t {
                value as $t
            }
            #[allow(trivial_numeric_casts)]
            fn to_f64(self) -> f64 {
                self as f64
            }
            fn sqrt(self) -> $t {
                <$t>::sqrt(self)
            }
            fn powf(self, power: $t) -> $t {
                <$t>::powf(self, power)
            }
            fn exp(self) -> $t {
                <$t>::exp(self)
            }
            fn ln(self) -> $t {
                <$t>::ln(self)
            }
            fn abs(self) -> $t {
                <$t>::abs(self)
            }
            fn is_finite(self) -> bool {
                <$t>::is_finite(self)
            }
        }
    };
}

float!(f32);
float!(f64);
