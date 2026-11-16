use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use rapier2d::prelude::*;

use super::{Body, BodyType, Fixture, Joint};

/// LÖVE's default: 30 units to a meter.
const DEFAULT_METER: f32 = 30.0;

pub(super) struct Inner {
    pub world: PhysicsWorld,
    pub meter: f32,
    contacts: Vec<(bool, ColliderHandle, ColliderHandle)>,
}

pub(super) type Shared = Rc<RefCell<Inner>>;

/// A space in which bodies move and collide, the equivalent of LÖVE's
/// `World`.
///
/// Cloning is cheap and gives another handle to the same world. Bodies,
/// fixtures and joints keep their world alive.
#[derive(Clone)]
pub struct World {
    pub(super) inner: Shared,
}

/// Two fixtures that started or stopped touching during the last
/// [`World::update`].
#[derive(Debug, Clone, PartialEq)]
pub struct Contact {
    /// `true` if they started touching, `false` if they stopped.
    pub began: bool,
    pub a: Fixture,
    pub b: Fixture,
}

impl Contact {
    /// Whether one of the two fixtures belongs to `body`.
    pub fn involves(&self, body: &Body) -> bool {
        self.a.body().as_ref() == Some(body) || self.b.body().as_ref() == Some(body)
    }
}

/// Where a ray first met a fixture.
#[derive(Debug, Clone, PartialEq)]
pub struct RayHit {
    pub fixture: Fixture,
    /// The point that was hit.
    pub x: f32,
    pub y: f32,
    /// The direction the surface faces at that point, of length 1.
    pub normal_x: f32,
    pub normal_y: f32,
    /// How far along the ray the hit is: 0.0 at its start, 1.0 at its end.
    pub fraction: f32,
}

impl World {
    /// Creates an empty world in which gravity accelerates bodies by
    /// `(gravity_x, gravity_y)` units per second, every second. Positive y
    /// is down, so `World::new(0.0, 9.81 * 30.0)` is Earth's gravity at
    /// the default 30 units to a meter.
    pub fn new(gravity_x: f32, gravity_y: f32) -> World {
        let mut world = PhysicsWorld::new();
        world.gravity = Vector::new(gravity_x, gravity_y);
        world.integration_parameters.length_unit = DEFAULT_METER;
        World {
            inner: Rc::new(RefCell::new(Inner {
                world,
                meter: DEFAULT_METER,
                contacts: Vec::new(),
            })),
        }
    }

    /// Moves the simulation forward by `dt` seconds. Call it once a frame
    /// from `update` with the frame's `dt`.
    ///
    /// For motion that is the same on every machine, call it with a fixed
    /// step such as `1.0 / 60.0` instead, as many times as have elapsed.
    pub fn update(&self, dt: f32) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let mut inner = self.inner.borrow_mut();
        let (collisions, collision_events) = mpsc::channel();
        let (forces, _force_events) = mpsc::channel();
        let (tears, _tear_events) = mpsc::channel();
        let collector = ChannelEventCollector::new(collisions, forces, tears);
        inner.world.integration_parameters.dt = dt;
        inner.world.step_with_events(&(), &collector);
        // A force lasts for one update, as in LÖVE. The engine would keep
        // applying it until told otherwise.
        for (_, body) in inner.world.bodies.iter_mut() {
            body.reset_forces(false);
            body.reset_torques(false);
        }

        inner.contacts.clear();
        while let Ok(event) = collision_events.try_recv() {
            inner.contacts.push(match event {
                CollisionEvent::Started(a, b, _) => (true, a, b),
                CollisionEvent::Stopped(a, b, _) => (false, a, b),
            });
        }
    }

    /// The fixtures that started or stopped touching during the last
    /// [`World::update`], in the order it happened. This takes the place
    /// of LÖVE's `beginContact` and `endContact` callbacks.
    ///
    /// A fixture that was destroyed while touching something is reported
    /// as having stopped; it then answers `true` to
    /// [`Fixture::is_destroyed`].
    pub fn contacts(&self) -> Vec<Contact> {
        let inner = self.inner.borrow();
        inner
            .contacts
            .iter()
            .map(|&(began, a, b)| Contact {
                began,
                a: Fixture::new(&self.inner, a),
                b: Fixture::new(&self.inner, b),
            })
            .collect()
    }

    /// Gravity as `(x, y)` in units per second, every second.
    pub fn gravity(&self) -> (f32, f32) {
        let gravity = self.inner.borrow().world.gravity;
        (gravity.x, gravity.y)
    }

    /// Changes gravity and wakes every body so it takes effect at once.
    pub fn set_gravity(&self, x: f32, y: f32) {
        let mut inner = self.inner.borrow_mut();
        inner.world.gravity = Vector::new(x, y);
        inner.world.wake_up_all(true);
    }

    /// How many units make a meter. See [`World::set_meter`].
    pub fn meter(&self) -> f32 {
        self.inner.borrow().meter
    }

    /// Says how many units make a meter, like `love.physics.setMeter`.
    ///
    /// Set it before creating bodies, to roughly the size of a small crate
    /// in your game. It scales the simulation's tolerances, and the mass
    /// fixtures created afterwards get from their density.
    pub fn set_meter(&self, units: f32) {
        assert!(
            units.is_finite() && units > 0.0,
            "a meter must be a positive number of units, got {units}"
        );
        let mut inner = self.inner.borrow_mut();
        inner.meter = units;
        inner.world.integration_parameters.length_unit = units;
    }

    /// Creates a body at `(x, y)`. It has no shape until fixtures are
    /// added to it, for example with [`Body::add_rectangle`].
    pub fn new_body(&self, x: f32, y: f32, body_type: BodyType) -> Body {
        let builder = match body_type {
            BodyType::Static => RigidBodyBuilder::fixed(),
            BodyType::Dynamic => RigidBodyBuilder::dynamic(),
            BodyType::Kinematic => RigidBodyBuilder::kinematic_velocity_based(),
        };
        let handle = self
            .inner
            .borrow_mut()
            .world
            .insert_body(builder.translation(Vector::new(x, y)));
        Body::new(&self.inner, handle)
    }

    /// Every body in the world.
    pub fn bodies(&self) -> Vec<Body> {
        let inner = self.inner.borrow();
        inner
            .world
            .rigid_bodies()
            .map(|(handle, _)| Body::new(&self.inner, handle))
            .collect()
    }

    /// How many bodies the world holds.
    pub fn body_count(&self) -> usize {
        self.inner.borrow().world.bodies.len()
    }

    /// Finds the first fixture a line from `(x1, y1)` to `(x2, y2)` runs
    /// into. Sensors are ignored. A ray that starts inside a fixture hits
    /// it at once, with a fraction of 0.
    pub fn ray_cast(&self, x1: f32, y1: f32, x2: f32, y2: f32) -> Option<RayHit> {
        let inner = self.inner.borrow();
        let (origin, direction) = (Vector::new(x1, y1), Vector::new(x2 - x1, y2 - y1));
        let ray = Ray::new(origin, direction);
        let filter = QueryFilter::default().exclude_sensors();
        let (collider, hit) = inner
            .world
            .cast_ray_and_get_normal(&ray, 1.0, true, filter)?;
        Some(RayHit {
            fixture: Fixture::new(&self.inner, collider),
            x: x1 + direction.x * hit.time_of_impact,
            y: y1 + direction.y * hit.time_of_impact,
            normal_x: hit.normal.x,
            normal_y: hit.normal.y,
            fraction: hit.time_of_impact,
        })
    }

    /// Every fixture that covers the point `(x, y)`, sensors included.
    /// Useful for picking things with the mouse.
    pub fn fixtures_at(&self, x: f32, y: f32) -> Vec<Fixture> {
        let inner = self.inner.borrow();
        inner
            .world
            .intersect_point(Vector::new(x, y), QueryFilter::default())
            .map(|(handle, _)| Fixture::new(&self.inner, handle))
            .collect()
    }

    /// Pins two bodies together at the point `(x, y)` so they can only
    /// turn around it, like a hinge or an axle. `collide` says whether
    /// the two bodies still collide with each other.
    pub fn new_revolute_joint(&self, a: &Body, b: &Body, x: f32, y: f32, collide: bool) -> Joint {
        let anchor = Vector::new(x, y);
        let joint = RevoluteJointBuilder::new()
            .local_anchor1(a.pose().inverse_transform_point(anchor))
            .local_anchor2(b.pose().inverse_transform_point(anchor))
            .contacts_enabled(collide);
        self.insert_joint(a, b, joint)
    }

    /// Glues two bodies together so they move as one.
    pub fn new_weld_joint(&self, a: &Body, b: &Body, collide: bool) -> Joint {
        // Both bodies hold on to the same frame in the world: the first
        // body's own.
        let (pose_a, pose_b) = (a.pose(), b.pose());
        let joint = FixedJointBuilder::new()
            .local_frame1(Pose::IDENTITY)
            .local_frame2(pose_b.inverse() * pose_a)
            .contacts_enabled(collide);
        self.insert_joint(a, b, joint)
    }

    /// Ties `(x1, y1)` on the first body to `(x2, y2)` on the second with
    /// a rope: they can come closer but never get further apart than
    /// `max_length`.
    #[allow(clippy::too_many_arguments)] // Mirrors love.physics.newRopeJoint.
    pub fn new_rope_joint(
        &self,
        a: &Body,
        b: &Body,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        max_length: f32,
        collide: bool,
    ) -> Joint {
        let joint = RopeJointBuilder::new(max_length)
            .local_anchor1(a.pose().inverse_transform_point(Vector::new(x1, y1)))
            .local_anchor2(b.pose().inverse_transform_point(Vector::new(x2, y2)))
            .contacts_enabled(collide);
        self.insert_joint(a, b, joint)
    }

    /// Connects `(x1, y1)` on the first body to `(x2, y2)` on the second
    /// with a spring that rests at their current distance. A higher
    /// `stiffness` pulls back harder, and `damping` makes the bouncing
    /// die away. The bodies still collide with each other.
    #[allow(clippy::too_many_arguments)] // Two bodies and two anchors.
    pub fn new_spring_joint(
        &self,
        a: &Body,
        b: &Body,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stiffness: f32,
        damping: f32,
    ) -> Joint {
        let (first, second) = (Vector::new(x1, y1), Vector::new(x2, y2));
        let joint = SpringJointBuilder::new((second - first).length(), stiffness, damping)
            .local_anchor1(a.pose().inverse_transform_point(first))
            .local_anchor2(b.pose().inverse_transform_point(second));
        self.insert_joint(a, b, joint)
    }

    /// Lets the second body slide along a line through `(x, y)` in the
    /// direction `(axis_x, axis_y)` relative to the first, without
    /// turning, like a piston or a lift.
    #[allow(clippy::too_many_arguments)] // Mirrors love.physics.newPrismaticJoint.
    pub fn new_prismatic_joint(
        &self,
        a: &Body,
        b: &Body,
        x: f32,
        y: f32,
        axis_x: f32,
        axis_y: f32,
        collide: bool,
    ) -> Joint {
        let axis = Vector::new(axis_x, axis_y);
        assert!(
            axis.length() > 0.0,
            "a prismatic joint needs a direction to slide in"
        );
        let (anchor, axis) = (Vector::new(x, y), axis.normalize());
        let (pose_a, pose_b) = (a.pose(), b.pose());
        let mut joint =
            PrismaticJointBuilder::new(pose_a.rotation.inverse().transform_vector(axis))
                .local_anchor1(pose_a.inverse_transform_point(anchor))
                .local_anchor2(pose_b.inverse_transform_point(anchor))
                .contacts_enabled(collide)
                .build();
        joint
            .data
            .set_local_axis2(pose_b.rotation.inverse().transform_vector(axis));
        self.insert_joint(a, b, joint)
    }

    fn insert_joint(&self, a: &Body, b: &Body, joint: impl Into<GenericJoint>) -> Joint {
        assert!(
            Rc::ptr_eq(&self.inner, &a.world) && Rc::ptr_eq(&self.inner, &b.world),
            "both bodies of a joint must belong to this world"
        );
        let handle = self
            .inner
            .borrow_mut()
            .world
            .insert_impulse_joint(a.handle, b.handle, joint);
        Joint::new(&self.inner, handle)
    }

    /// Draws the outline of every fixture in the current color, for
    /// seeing what the simulation sees. Needs a running game.
    pub fn draw_debug(&self) {
        use crate::graphics::{self, DrawMode};

        // Collect first: drawing must not happen while the world is
        // borrowed.
        let mut circles = Vec::new();
        let mut polygons = Vec::new();
        let mut lines = Vec::new();
        {
            let inner = self.inner.borrow();
            for (_, collider) in inner.world.all_colliders() {
                let pose = collider.position();
                let place = |point: Vector| {
                    let point = pose.transform_point(point);
                    (point.x, point.y)
                };
                let shape = collider.shape();
                if let Some(ball) = shape.as_ball() {
                    let (x, y) = place(Vector::ZERO);
                    circles.push((x, y, ball.radius));
                    // A spoke shows which way the body is turned.
                    lines.push(vec![(x, y), place(Vector::new(ball.radius, 0.0))]);
                } else if let Some(cuboid) = shape.as_cuboid() {
                    let half = cuboid.half_extents;
                    polygons.push(vec![
                        place(Vector::new(-half.x, -half.y)),
                        place(Vector::new(half.x, -half.y)),
                        place(Vector::new(half.x, half.y)),
                        place(Vector::new(-half.x, half.y)),
                    ]);
                } else if let Some(polygon) = shape.as_convex_polygon() {
                    polygons.push(polygon.points().iter().map(|&point| place(point)).collect());
                } else if let Some(segment) = shape.as_segment() {
                    lines.push(vec![place(segment.a), place(segment.b)]);
                } else if let Some(chain) = shape.as_polyline() {
                    let vertices = chain.vertices();
                    for segment in chain.indices() {
                        lines.push(vec![
                            place(vertices[segment[0] as usize]),
                            place(vertices[segment[1] as usize]),
                        ]);
                    }
                }
            }
        }
        for (x, y, radius) in circles {
            graphics::circle(DrawMode::Line, x, y, radius);
        }
        for polygon in polygons {
            graphics::polygon(DrawMode::Line, &polygon);
        }
        for line in lines {
            for pair in line.windows(2) {
                graphics::line(pair[0].0, pair[0].1, pair[1].0, pair[1].1);
            }
        }
    }
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World")
            .field("gravity", &self.gravity())
            .field("bodies", &self.body_count())
            .finish()
    }
}
