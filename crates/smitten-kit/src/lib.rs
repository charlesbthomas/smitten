//! Game structure on top of [Smitten](smitten): the pieces nearly every
//! game writes by hand, in the spirit of hump and batteries for LÖVE.
//!
//! Screens and input:
//!
//! - [`scene`]: a stack of screens (title, play, pause) with enter and exit
//!   hooks, overlays and fades.
//! - [`input`]: named actions bound to keys, mouse buttons and gamepads,
//!   with pressed, held and released states, rebinding, and saving the
//!   bindings as text.
//! - [`ui`]: an immediate-mode menu of buttons, sliders and toggles
//!   worked by those actions or the mouse.
//! - [`debug`]: an overlay with frame times, draw statistics and watched
//!   values.
//!
//! Time:
//!
//! - [`timer`]: cooldowns, countdowns and a scheduler that hands back
//!   events.
//! - [`tween`]: easing curves, tweens, and frame-rate independent
//!   smoothing and springs.
//! - [`sequence`]: scripted runs of waits and events, for wave banners and
//!   cutscenes.
//!
//! The world:
//!
//! - [`camera`]: follow with a dead zone, bounds, zoom, shake, and
//!   screen-to-world conversion.
//! - [`sprite`] and [`animation`]: sprite sheets cut into frames, clips,
//!   and an animator per character.
//! - [`tilemap`]: maps from the Tiled editor, drawn culled to the camera
//!   (feature `tiled`, on by default).
//! - [`collision`]: overlap tests that return push-out vectors, rays, a
//!   spatial hash, and moving a box through a tile map.
//! - [`grid`]: 2D grids with A* paths, flood fills and distance maps.
//! - [`pool`]: things that come and go, named by handles that know when
//!   they are gone.
//! - [`math`], [`random`], [`events`]: rectangles and circles, weighted
//!   picks, and an event queue.
//! - [`save`]: versioned save slots (feature `save`, on by default).
//!
//! Everything that needs no window comes from
//! [`smitten-kit-core`](smitten_kit_core), which a game's rules crate can
//! use without depending on SDL. Timers, tweens and sequences there take
//! `f32` or `f64` time.
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
pub mod debug;
pub mod input;
#[cfg(feature = "save")]
pub mod save;
pub mod scene;
pub mod sprite;
#[cfg(feature = "tiled")]
pub mod tilemap;
pub mod ui;

pub use smitten_kit_core::{
    Float, animation, collision, events, glam, grid, math, pool, random, sequence, timer, tween,
};

/// The kit's types, plus the parts of Smitten that scenes use most, for
/// `use smitten_kit::prelude::*`.
pub mod prelude {
    pub use crate::animation::{Animation, Animator};
    pub use crate::camera::{Camera, Follow, Shake, Smoothing};
    pub use crate::collision::{SpatialHash, move_and_slide};
    pub use crate::debug::DebugOverlay;
    pub use crate::events::Events;
    pub use crate::grid::{Grid, Neighbors};
    pub use crate::input::{Actions, Binding, Device};
    pub use crate::math::{Circle, Rect};
    pub use crate::pool::{Handle, Pool};
    pub use crate::scene::{self, Scene, SceneStack, Transition};
    pub use crate::sequence::Sequence;
    pub use crate::sprite::{Flip, SpriteSheet};
    pub use crate::timer::{Cooldown, Countdown, TimerId, Timers};
    pub use crate::tween::{Ease, Lerp, Repeat, Spring, Tween, approach};
    pub use crate::ui::{Ui, UiActions, UiInput};
    pub use smitten::event::Event;
    pub use smitten::{
        Color, Config, GamepadAxis, GamepadButton, Joystick, Key, MouseButton, Vec2, graphics,
    };
}
