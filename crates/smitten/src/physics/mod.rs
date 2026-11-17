//! Rigid-body physics, the equivalent of `love.physics`.
//!
//! A [`World`] holds [`Body`]s. Each body has one or more [`Fixture`]s,
//! the shapes that collide, and bodies can be tied together with
//! [`Joint`]s. Call [`World::update`] once a frame to move everything.
//!
//! ```
//! use smitten::physics::{BodyType, World};
//!
//! // Gravity pulls down at 600 units per second, every second.
//! let world = World::new(0.0, 600.0);
//!
//! let ground = world.new_body(400.0, 580.0, BodyType::Static);
//! ground.add_rectangle(800.0, 40.0);
//!
//! let ball = world.new_body(400.0, 100.0, BodyType::Dynamic);
//! ball.add_circle(20.0).set_restitution(0.5);
//!
//! for _ in 0..120 {
//!     world.update(1.0 / 60.0);
//! }
//! // The ball has fallen onto the ground.
//! assert!(ball.y() > 400.0 && ball.y() < 560.0);
//! ```
//!
//! Positions are in the same units `graphics` draws in, with y pointing
//! down, and angles are in radians, clockwise on screen, so a body's
//! position and angle can be passed straight to `graphics::draw_ex`.
//!
//! Like LÖVE, the simulation is tuned by saying how many units make a
//! meter ([`World::set_meter`], 30 by default): bodies behave best when
//! they are between a tenth of a meter and ten meters across.
//!
//! The simulation is done by the Rapier engine, where LÖVE uses Box2D, so
//! the API matches LÖVE's but the exact motion does not. This module works
//! without a running game, except for [`World::draw_debug`].

mod body;
mod fixture;
mod joint;
mod world;

#[cfg(test)]
mod tests;

pub use body::{Body, BodyType};
pub use fixture::Fixture;
pub use joint::{Joint, JointType};
pub use world::{Contact, PreSolve, RayHit, Touch, World};
