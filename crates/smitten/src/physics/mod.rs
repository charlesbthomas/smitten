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
//! let world = World::new((0.0, 600.0));
//!
//! let ground = world.new_body((400.0, 580.0), BodyType::Static);
//! ground.add_rectangle((800.0, 40.0));
//!
//! let ball = world.new_body((400.0, 100.0), BodyType::Dynamic);
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
//! position and angle can be passed straight to
//! [`DrawParams`](crate::graphics::DrawParams).
//!
//! Like LÖVE, the simulation is tuned by saying how many units make a
//! meter ([`World::set_meter`], 30 by default): bodies behave best when
//! they are between a tenth of a meter and ten meters across.
//!
//! The simulation is done by the Rapier engine, where LÖVE uses Box2D, so
//! the API matches LÖVE's but the exact motion does not. This module works
//! without a running game, except for [`World::draw_debug`].

use glam::Vec2;

mod body;
mod coupling;
mod fixture;
mod ghost;
mod joint;
mod shape;
mod world;

#[cfg(test)]
mod tests;

pub use body::{Body, BodyType};
pub use fixture::Fixture;
pub use joint::{
    AnyJoint, DistanceJoint, FrictionJoint, GearJoint, Joint, JointOf, JointType, MotorJoint,
    MouseJoint, PrismaticJoint, PulleyJoint, RevoluteJoint, RopeJoint, SpringJoint, WeldJoint,
    WheelJoint, joint_kind,
};
pub use shape::{
    AnyShape, ChainShape, CircleShape, EdgeShape, PolygonShape, Shape, ShapeOf, ShapeType,
    shape_kind,
};
pub use world::{Contact, ContactFilter, PreSolve, RayHit, Touch, World};

/// How far apart two fixtures are, and the closest points of each, as
/// `(distance, (x1, y1), (x2, y2))` in world coordinates. Fixtures that
/// overlap are 0 apart, and both points are then a point where they
/// touch. LÖVE's `love.physics.getDistance`.
///
/// Panics if the two belong to different worlds or either was destroyed.
#[doc(alias = "love.physics.getDistance")]
pub fn distance(a: &Fixture, b: &Fixture) -> (f32, Vec2, Vec2) {
    use rapier2d::parry::query::{self, ClosestPoints};

    assert!(
        std::rc::Rc::ptr_eq(&a.world, &b.world),
        "the fixtures must belong to the same world"
    );
    let inner = a.world.borrow();
    let collider = |fixture: &Fixture| {
        inner
            .world
            .colliders
            .get(fixture.handle)
            .expect("this fixture was destroyed")
    };
    let (one, two) = (collider(a), collider(b));
    let (pose_one, pose_two) = (*one.position(), *two.position());
    match query::closest_points(&pose_one, one.shape(), &pose_two, two.shape(), f32::MAX) {
        Ok(ClosestPoints::WithinMargin(p, q)) => {
            ((q - p).length(), (p.x, p.y).into(), (q.x, q.y).into())
        }
        _ => {
            // Overlapping: report where they touch.
            let point = query::contact(&pose_one, one.shape(), &pose_two, two.shape(), 0.0)
                .ok()
                .flatten()
                .map_or_else(|| pose_one.translation, |contact| contact.point1);
            (0.0, (point.x, point.y).into(), (point.x, point.y).into())
        }
    }
}
