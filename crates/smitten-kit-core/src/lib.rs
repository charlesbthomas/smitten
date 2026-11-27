//! The parts of [smitten-kit] that need no window or GPU: timers, tweens,
//! springs, sequences, shapes, collision, grids, pathfinding, pools and
//! event queues.
//!
//! Everything here is plain data and plain functions, so it can sit in a
//! game's rules crate (one that must build and test without SDL) as well
//! as in the game itself. `smitten-kit` re-exports all of it, so a game
//! that uses the whole kit never needs to name this crate.
//!
//! Timers, tweens and sequences count time in `f32` by default, as
//! smitten's `dt` is, and take `f64` too (see [`Float`]). Shapes come in
//! both: [`Rect`](math::Rect) and [`DRect`](math::DRect).
//!
//! The `color` feature adds [`Lerp`](tween::Lerp) for smitten's `Color`.
//!
//! [smitten-kit]: https://github.com/charlesbthomas/smitten/tree/main/crates/smitten-kit

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod animation;
pub mod collision;
pub mod events;
pub mod float;
pub mod grid;
pub mod math;
pub mod pool;
pub mod random;
pub mod sequence;
#[cfg(feature = "tiled")]
pub mod tilemap;
pub mod timer;
pub mod tween;

pub use float::Float;
pub use glam;
