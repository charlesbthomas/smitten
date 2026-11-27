//! The parts of [smitten-kit] that need no window or GPU: timers, tweens,
//! springs, sequences, shapes, collision, grids, pathfinding, pools and
//! event queues.
//!
//! Everything here is plain data and plain functions, so it can sit in a
//! game's rules crate (one that must build and test without SDL) as well
//! as in the game itself. `smitten-kit` re-exports all of it, so a game
//! that uses the whole kit never needs to name this crate.
//!
//! - [`timer`], [`tween`], [`sequence`]: cooldowns and scheduled events,
//!   easing and springs, scripted waits and events.
//! - [`math`], [`collision`]: rectangles and circles, overlap tests with
//!   push-out vectors, rays, a spatial hash, and moving through tiles.
//! - [`grid`]: 2D grids, A* (on grids or any graph), flood fills and
//!   distance maps.
//! - [`pool`], [`events`], [`random`]: handles that know when what they
//!   named is gone, an event queue, and weighted picks.
//! - [`animation`]: frame clips and an animator per character.
//! - `tilemap` (feature `tiled`): reading maps from the Tiled editor.
//! - `save` (feature `save`): versioned save data as JSON.
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
#[cfg(feature = "save")]
pub mod save;
pub mod sequence;
#[cfg(feature = "tiled")]
pub mod tilemap;
pub mod timer;
pub mod tween;

pub use float::Float;
pub use glam;
