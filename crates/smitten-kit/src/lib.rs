//! Game structure on top of [Smitten](smitten): the pieces nearly every
//! game writes by hand, in the spirit of hump and batteries for LÖVE.
//!
//! - [`scene`]: a stack of screens (title, play, pause) with enter and exit
//!   hooks, overlays and fades.
//! - [`timer`]: cooldowns, countdowns and a scheduler that hands back
//!   events.
//! - [`tween`]: easing curves, tweens, and frame-rate independent
//!   smoothing and springs.
//! - [`sequence`]: scripted runs of waits and events, for wave banners and
//!   cutscenes.
//! - [`input`]: named actions bound to keys, mouse buttons and gamepads,
//!   with pressed, held and released states and rebinding.
//! - [`camera`]: follow with a dead zone, bounds, zoom, shake, and
//!   screen-to-world conversion.
//! - [`math`]: [`Rect`](math::Rect), shared by the camera and later modules.
//!
//! Smitten's own modules are global because they wrap the platform. The
//! kit's state is game state, so it lives in plain values the game owns:
//! a game can run two cameras or two timer sets, and everything but
//! drawing works in tests without a window.
//!
//! ```no_run
//! use smitten_kit::prelude::*;
//!
//! struct Shared {
//!     best: u32,
//! }
//!
//! struct Title;
//!
//! impl Scene<Shared> for Title {
//!     fn draw(&mut self, ctx: &mut Shared) {
//!         graphics::print(format!("Best: {}", ctx.best).as_str(), (20.0, 20.0));
//!     }
//!
//!     fn event(&mut self, _: &mut Shared, event: &Event) -> Transition<Shared> {
//!         match event {
//!             Event::KeyPressed { key: Key::Escape, .. } => Transition::Quit,
//!             _ => Transition::None,
//!         }
//!     }
//! }
//!
//! impl scene::Context for Shared {}
//!
//! fn main() {
//!     scene::run(Config::default(), || Ok((Shared { best: 0 }, Title)));
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod camera;
pub mod input;
pub mod scene;

pub use smitten_kit_core::{Float, math, sequence, timer, tween};

/// The kit's types, plus the parts of Smitten that scenes use most, for
/// `use smitten_kit::prelude::*`.
pub mod prelude {
    pub use crate::camera::{Camera, Follow, Shake, Smoothing};
    pub use crate::input::{Actions, Binding, Device};
    pub use crate::math::Rect;
    pub use crate::scene::{self, Scene, SceneStack, Transition};
    pub use crate::sequence::Sequence;
    pub use crate::timer::{Cooldown, Countdown, TimerId, Timers};
    pub use crate::tween::{Ease, Lerp, Repeat, Spring, Tween, approach};
    pub use smitten::event::Event;
    pub use smitten::{
        Color, Config, GamepadAxis, GamepadButton, Joystick, Key, MouseButton, Vec2, graphics,
    };
}
