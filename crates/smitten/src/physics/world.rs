use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;

use rapier2d::prelude::*;

use super::joint::{self, JointType, MOTOR_DAMPING};
use super::{Body, BodyType, Fixture, Joint};

/// LÖVE's default: 30 units to a meter.
const DEFAULT_METER: f32 = 30.0;

pub(super) struct Inner {
    pub world: PhysicsWorld,
    pub meter: f32,
    contacts: Vec<RawContact>,
    /// A fixed body with no fixtures at the origin, made the first time a
    /// mouse joint needs something to hold its target. Hidden from the
    /// game.
    pub ground: Option<RigidBodyHandle>,
    pre_solve: Option<PreSolveCallback>,
}

type PreSolveCallback = Box<dyn FnMut(&mut PreSolve)>;

/// A contact as the engine reported it, before handles are made for the
/// game.
#[derive(Clone, Copy)]
struct RawContact {
    began: bool,
    a: ColliderHandle,
    b: ColliderHandle,
    point: Vector,
    normal: Vector,
    speed: f32,
}

/// How a body was moving before an update: its velocity, its spin, and
/// the point it spins around.
type Motion = (Vector, f32, Vector);

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
    /// Where they touch, in world coordinates. For a sensor, or once they
    /// have stopped touching, it is the point half way between the two.
    pub x: f32,
    pub y: f32,
    /// The direction from `a` into `b` at that point, as a vector one
    /// unit long.
    pub normal: (f32, f32),
    /// How fast the two were closing on each other along the normal just
    /// before the update, in units per second. It says how hard they hit:
    /// near zero for things settling against each other, large for a
    /// crash. Zero when they were moving apart, and when they stopped
    /// touching.
    pub speed: f32,
}

impl Contact {
    /// Whether one of the two fixtures belongs to `body`.
    pub fn involves(&self, body: &Body) -> bool {
        self.a.body().as_ref() == Some(body) || self.b.body().as_ref() == Some(body)
    }
}

/// Where two fixtures touch and the direction from the first into the
/// second there. Without contact points to go by (a sensor, or two that
/// no longer touch) this is half way between them.
fn touching_at(world: &PhysicsWorld, a: ColliderHandle, b: ColliderHandle) -> (Vector, Vector) {
    let place = |handle| {
        world
            .colliders
            .get(handle)
            .map(|collider| *collider.position())
    };
    let (Some(pose_a), Some(pose_b)) = (place(a), place(b)) else {
        // One of them was destroyed.
        return (Vector::ZERO, Vector::new(0.0, 1.0));
    };
    let unit = |vector: Vector, fallback: Vector| {
        let length = vector.length();
        if length > 1e-6 {
            vector / length
        } else {
            fallback
        }
    };
    let (center_a, center_b) = (
        pose_a.transform_point(Vector::ZERO),
        pose_b.transform_point(Vector::ZERO),
    );
    let apart = unit(center_b - center_a, Vector::new(0.0, 1.0));
    let between = ((center_a + center_b) / 2.0, apart);

    let Some(pair) = world.contact_pair(a, b) else {
        return between;
    };
    let PairContacts::Rigid(contacts) = &pair.contacts else {
        return between;
    };
    let Some(manifold) = contacts
        .manifolds
        .iter()
        .find(|manifold| !manifold.points.is_empty())
    else {
        return between;
    };
    // The manifold's points are relative to the pair's first collider,
    // which may be either of the two, or to one piece of it.
    let first = if pair.collider1 == a { pose_a } else { pose_b };
    let piece = manifold
        .subshape_poses
        .as_ref()
        .and_then(|poses| poses.pos1);
    let to_world = |local: Vector| {
        first.transform_point(piece.map_or(local, |piece| piece.transform_point(local)))
    };
    let count = manifold.points.len() as f32;
    let local = manifold
        .points
        .iter()
        .fold(Vector::ZERO, |sum, point| sum + point.local_p1)
        / count;
    let point = to_world(local);
    let mut normal = unit(to_world(local + manifold.local_n1) - point, apart);
    if pair.collider1 != a {
        normal = -normal;
    }
    (point, normal)
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

/// Two fixtures touching after an update, and how hard they were pushed
/// apart. See [`World::touching`].
#[derive(Debug, Clone, PartialEq)]
pub struct Touch {
    pub a: Fixture,
    pub b: Fixture,
    /// Where they touch, in world coordinates.
    pub x: f32,
    pub y: f32,
    /// The direction from `a` into `b` there, one unit long.
    pub normal: (f32, f32),
    /// The push along the normal that kept them apart during the last
    /// update, summed over the points where they touch. Divided by the
    /// update's `dt` it is a force: something resting on `b` pushes with
    /// its weight.
    pub normal_impulse: f32,
    /// The push across the normal that friction gave, likewise.
    pub tangent_impulse: f32,
}

impl Touch {
    /// Whether one of the two fixtures belongs to `body`.
    pub fn involves(&self, body: &Body) -> bool {
        self.a.body().as_ref() == Some(body) || self.b.body().as_ref() == Some(body)
    }
}

/// A contact about to be solved, handed to the callback given to
/// [`World::set_pre_solve`], the equivalent of the contact in LÖVE's
/// `preSolve`. Change its public fields to change how it is solved.
///
/// Do not call methods on `a` and `b` inside the callback: the world is
/// busy and they panic. Compare them with `==` or look them up in a map,
/// and use the user data copied here.
#[derive(Debug)]
pub struct PreSolve {
    pub a: Fixture,
    pub b: Fixture,
    /// [`Fixture::user_data`] of `a` and `b`.
    pub a_data: u64,
    pub b_data: u64,
    /// [`Body::user_data`] of the bodies of `a` and `b`.
    pub a_body_data: u64,
    pub b_body_data: u64,
    /// The direction from `a` into `b`, one unit long.
    pub normal: (f32, f32),
    /// Where they touch, in world coordinates.
    pub x: f32,
    pub y: f32,
    /// Set to `false` to let the two pass through each other for this
    /// update. `true` to begin with.
    pub enabled: bool,
    /// The friction and bounce used for this contact, starting from the
    /// fixtures' own. Changes last for this update only.
    pub friction: f32,
    pub restitution: f32,
}

thread_local! {
    /// The world being stepped and its pre-solve callback, for [`Hooks`].
    /// The engine wants hooks it could share between threads, which the
    /// callback is not; the step runs on this thread, so it waits here.
    static PRE_SOLVE: RefCell<Option<(Shared, PreSolveCallback)>> = const { RefCell::new(None) };
}

/// Runs the pre-solve callback from inside the engine's step.
struct Hooks;

impl PhysicsHooks for Hooks {
    fn modify_solver_contacts(&self, context: &mut ContactModificationContext) {
        let data = |handle| {
            context
                .colliders
                .get(handle)
                .map_or((0, 0), |collider: &Collider| {
                    let body = collider
                        .parent()
                        .and_then(|parent| context.bodies.get(parent))
                        .map_or(0, |body| body.user_data as u64);
                    (collider.user_data as u64, body)
                })
        };
        let (a, b) = (context.collider1, context.collider2);
        let ((a_data, a_body_data), (b_data, b_body_data)) = (data(a), data(b));
        let Some(manifold) = context.rigid_mut() else {
            return;
        };
        let count = manifold.solver_contacts.len().max(1) as f32;
        let point = manifold
            .solver_contacts
            .iter()
            .fold(Vector::ZERO, |sum, contact| sum + contact.anchor1)
            / count;
        PRE_SOLVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some((world, callback)) = slot.as_mut() else {
                return;
            };
            let mut contact = PreSolve {
                a: Fixture::new(world, a),
                b: Fixture::new(world, b),
                a_data,
                b_data,
                a_body_data,
                b_body_data,
                normal: (manifold.normal.x, manifold.normal.y),
                x: point.x,
                y: point.y,
                enabled: true,
                friction: *manifold.friction,
                restitution: *manifold.restitution,
            };
            callback(&mut contact);
            if contact.enabled {
                *manifold.friction = contact.friction;
                *manifold.restitution = contact.restitution;
            } else {
                manifold.solver_contacts.clear();
            }
        });
    }
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
                ground: None,
                pre_solve: None,
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
        // Remembered so that a contact can say how hard the two hit.
        let before: HashMap<RigidBodyHandle, Motion> = inner
            .world
            .bodies
            .iter()
            .map(|(handle, body)| {
                (
                    handle,
                    (body.linvel(), body.angvel(), body.center_of_mass()),
                )
            })
            .collect();
        // The pre-solve callback is taken out while the step runs, so the
        // world can be borrowed for it.
        if let Some(callback) = inner.pre_solve.take() {
            PRE_SOLVE.with(|slot| *slot.borrow_mut() = Some((self.inner.clone(), callback)));
            inner.world.step_with_events(&Hooks, &collector);
            inner.pre_solve = PRE_SOLVE
                .with(|slot| slot.borrow_mut().take())
                .map(|(_, callback)| callback);
        } else {
            inner.world.step_with_events(&(), &collector);
        }
        // A force lasts for one update, as in LÖVE. The engine would keep
        // applying it until told otherwise.
        for (_, body) in inner.world.bodies.iter_mut() {
            body.reset_forces(false);
            body.reset_torques(false);
        }

        inner.contacts.clear();
        while let Ok(event) = collision_events.try_recv() {
            let (began, a, b) = match event {
                CollisionEvent::Started(a, b, _) => (true, a, b),
                CollisionEvent::Stopped(a, b, _) => (false, a, b),
            };
            let (point, normal) = touching_at(&inner.world, a, b);
            // The velocity each fixture's body had at the point.
            let velocity = |collider: ColliderHandle| {
                inner
                    .world
                    .colliders
                    .get(collider)
                    .and_then(|collider| collider.parent())
                    .and_then(|body| before.get(&body))
                    .map_or(Vector::ZERO, |&(velocity, spin, center)| {
                        let arm = point - center;
                        velocity + Vector::new(-arm.y, arm.x) * spin
                    })
            };
            let speed = if began {
                (velocity(a) - velocity(b)).dot(normal).max(0.0)
            } else {
                0.0
            };
            inner.contacts.push(RawContact {
                began,
                a,
                b,
                point,
                normal,
                speed,
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
            .map(|contact| Contact {
                began: contact.began,
                a: Fixture::new(&self.inner, contact.a),
                b: Fixture::new(&self.inner, contact.b),
                x: contact.point.x,
                y: contact.point.y,
                normal: (contact.normal.x, contact.normal.y),
                speed: contact.speed,
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
            .filter(|&(handle, _)| Some(handle) != inner.ground)
            .map(|(handle, _)| Body::new(&self.inner, handle))
            .collect()
    }

    /// How many bodies the world holds.
    pub fn body_count(&self) -> usize {
        let inner = self.inner.borrow();
        inner.world.bodies.len() - usize::from(inner.ground.is_some())
    }

    /// Every joint in the world.
    pub fn joints(&self) -> Vec<Joint> {
        let inner = self.inner.borrow();
        inner
            .world
            .impulse_joints
            .iter()
            .map(|(handle, _)| Joint::new(&self.inner, handle))
            .collect()
    }

    /// How many joints the world holds.
    pub fn joint_count(&self) -> usize {
        self.inner.borrow().world.impulse_joints.len()
    }

    /// Every pair of fixtures touching now, with how hard the last update
    /// pushed them apart. This takes the place of LÖVE's `postSolve`
    /// callback and of `World:getContacts`. Sensors are not included;
    /// see [`World::contacts`] for those.
    pub fn touching(&self) -> Vec<Touch> {
        let inner = self.inner.borrow();
        let mut touches = Vec::new();
        for pair in inner.world.contact_pairs() {
            if !pair.has_any_active_contact() {
                continue;
            }
            let PairContacts::Rigid(contacts) = &pair.contacts else {
                continue;
            };
            let (mut normal_impulse, mut tangent_impulse) = (0.0, 0.0);
            for manifold in &contacts.manifolds {
                for point in &manifold.points {
                    normal_impulse += point.data.impulse;
                    tangent_impulse += point.data.tangent_impulse[0];
                }
            }
            let (a, b) = (pair.collider1, pair.collider2);
            let (point, normal) = touching_at(&inner.world, a, b);
            touches.push(Touch {
                a: Fixture::new(&self.inner, a),
                b: Fixture::new(&self.inner, b),
                x: point.x,
                y: point.y,
                normal: (normal.x, normal.y),
                normal_impulse,
                tangent_impulse,
            });
        }
        touches
    }

    /// Calls `callback` for every pair of fixtures about to be pushed
    /// apart, during each [`World::update`], before the push is worked
    /// out. This is LÖVE's `preSolve`: the callback can turn the contact
    /// off, for a one-way platform, or change its friction or bounce.
    ///
    /// The world is busy while the callback runs, so it must not call
    /// methods of the world or of its bodies, fixtures and joints; doing
    /// so panics. Comparing fixtures with `==` and looking them up in a
    /// `HashMap` is fine, and the [`PreSolve`] carries the user data of
    /// both fixtures and their bodies.
    ///
    /// ```
    /// use smitten::physics::{BodyType, World};
    ///
    /// let world = World::new(0.0, 600.0);
    /// let platform = world.new_body(0.0, 0.0, BodyType::Static);
    /// let top = platform.add_rectangle(200.0, 10.0);
    /// world.set_pre_solve(move |contact| {
    ///     // Things below the platform pass up through it: only a
    ///     // contact pushing up out of its top counts.
    ///     let up = if contact.a == top { contact.normal.1 } else { -contact.normal.1 };
    ///     if up > -0.5 {
    ///         contact.enabled = false;
    ///     }
    /// });
    /// ```
    pub fn set_pre_solve(&self, callback: impl FnMut(&mut PreSolve) + 'static) {
        self.inner.borrow_mut().pre_solve = Some(Box::new(callback));
    }

    /// Stops calling the callback given to [`World::set_pre_solve`].
    pub fn clear_pre_solve(&self) {
        self.inner.borrow_mut().pre_solve = None;
    }

    /// Whether a callback was given to [`World::set_pre_solve`].
    pub fn has_pre_solve(&self) -> bool {
        self.inner.borrow().pre_solve.is_some()
    }

    /// Every fixture whose bounding box overlaps the box from
    /// `(left, top)` to `(right, bottom)`, sensors included. This is
    /// LÖVE's `World:queryBoundingBox`: like it, it goes by each fixture's
    /// bounding box, so a fixture near a corner may be listed without its
    /// shape quite reaching the box. Check with [`Fixture::test_point`]
    /// or [`World::fixtures_at`] where that matters.
    pub fn fixtures_in_box(&self, left: f32, top: f32, right: f32, bottom: f32) -> Vec<Fixture> {
        let query = Aabb::new(
            Vector::new(left.min(right), top.min(bottom)),
            Vector::new(left.max(right), top.max(bottom)),
        );
        let inner = self.inner.borrow();
        inner
            .world
            .colliders
            .iter()
            .filter(|(_, collider)| collider.compute_aabb().intersects(&query))
            .map(|(handle, _)| Fixture::new(&self.inner, handle))
            .collect()
    }

    /// Every fixture the line from `(x1, y1)` to `(x2, y2)` runs into,
    /// nearest first. Sensors are ignored. Where [`World::ray_cast`]
    /// stops at the first, this is LÖVE's `rayCast` with a callback that
    /// keeps going.
    pub fn ray_cast_all(&self, x1: f32, y1: f32, x2: f32, y2: f32) -> Vec<RayHit> {
        let inner = self.inner.borrow();
        let direction = Vector::new(x2 - x1, y2 - y1);
        let ray = Ray::new(Vector::new(x1, y1), direction);
        let filter = QueryFilter::default().exclude_sensors();
        let mut hits: Vec<RayHit> = inner
            .world
            .intersect_ray(ray, 1.0, true, filter)
            .map(|(collider, _, hit)| RayHit {
                fixture: Fixture::new(&self.inner, collider),
                x: x1 + direction.x * hit.time_of_impact,
                y: y1 + direction.y * hit.time_of_impact,
                normal_x: hit.normal.x,
                normal_y: hit.normal.y,
                fraction: hit.time_of_impact,
            })
            .collect();
        hits.sort_by(|a, b| a.fraction.total_cmp(&b.fraction));
        hits
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
    ///
    /// The joint can have a motor and limits; see
    /// [`Joint::set_motor_enabled`] and [`Joint::set_limits`].
    pub fn new_revolute_joint(&self, a: &Body, b: &Body, x: f32, y: f32, collide: bool) -> Joint {
        let anchor = Vector::new(x, y);
        let (pose_a, pose_b) = (a.pose(), b.pose());
        let mut joint = RevoluteJointBuilder::new()
            .local_anchor1(pose_a.inverse_transform_point(anchor))
            .local_anchor2(pose_b.inverse_transform_point(anchor))
            .contacts_enabled(collide)
            .build()
            .data;
        // Angles are measured from how the bodies are turned now, as in
        // Box2D, so limits mean the same whichever way they started.
        joint.local_frame2.rotation = pose_b.rotation.inverse() * pose_a.rotation;
        idle_motor(&mut joint, JointAxis::AngX);
        self.insert_joint(a, b, joint, JointType::Revolute)
    }

    /// Glues two bodies together so they move as one.
    pub fn new_weld_joint(&self, a: &Body, b: &Body, collide: bool) -> Joint {
        // Both bodies hold on to the same frame in the world: the first
        // body's own.
        let (pose_a, pose_b) = (a.pose(), b.pose());
        let joint = FixedJointBuilder::new()
            .local_frame1(Pose::IDENTITY)
            .local_frame2(pose_b.inverse() * pose_a)
            .contacts_enabled(collide)
            .build()
            .data;
        self.insert_joint(a, b, joint, JointType::Weld)
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
            .contacts_enabled(collide)
            .build()
            .data;
        self.insert_joint(a, b, joint, JointType::Rope)
    }

    /// Holds `(x1, y1)` on the first body and `(x2, y2)` on the second at
    /// the distance they are apart now, like a rigid rod with a pivot at
    /// each end. For a springy rod, use [`World::new_spring_joint`].
    #[allow(clippy::too_many_arguments)] // Mirrors love.physics.newDistanceJoint.
    pub fn new_distance_joint(
        &self,
        a: &Body,
        b: &Body,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        collide: bool,
    ) -> Joint {
        let (first, second) = (Vector::new(x1, y1), Vector::new(x2, y2));
        let length = (second - first).length();
        // The engine's distance limits only stop stretching, so a stiff
        // spring stops squashing.
        let (stiffness, damping) = joint::spring(30.0, 1.0);
        let mut joint = RopeJointBuilder::new(length.max(f32::EPSILON))
            .local_anchor1(a.pose().inverse_transform_point(first))
            .local_anchor2(b.pose().inverse_transform_point(second))
            .contacts_enabled(collide)
            .build()
            .data;
        joint.set_motor_position(JointAxis::LinX, length, stiffness, damping);
        self.insert_joint(a, b, joint, JointType::Distance)
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
            .local_anchor2(b.pose().inverse_transform_point(second))
            .build()
            .data;
        self.insert_joint(a, b, joint, JointType::Spring)
    }

    /// Lets the second body slide along a line through `(x, y)` in the
    /// direction `(axis_x, axis_y)` relative to the first, without
    /// turning, like a piston or a lift.
    ///
    /// The joint can have a motor and limits; see
    /// [`Joint::set_motor_enabled`] and [`Joint::set_limits`].
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
                .build()
                .data;
        joint.set_local_axis2(pose_b.rotation.inverse().transform_vector(axis));
        idle_motor(&mut joint, JointAxis::LinX);
        self.insert_joint(a, b, joint, JointType::Prismatic)
    }

    /// Puts the second body on the first as a wheel at `(x, y)`: it turns
    /// freely and rides up and down the direction `(axis_x, axis_y)` on a
    /// springy suspension, as a car's wheels do. The first body is the
    /// car.
    ///
    /// The wheel can be driven by a motor (see
    /// [`Joint::set_motor_enabled`]), and the suspension tuned with
    /// [`Joint::set_frequency`] and [`Joint::set_damping_ratio`].
    #[allow(clippy::too_many_arguments)] // Mirrors love.physics.newWheelJoint.
    pub fn new_wheel_joint(
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
            "a wheel joint needs a direction for its suspension"
        );
        let (anchor, axis) = (Vector::new(x, y), axis.normalize());
        let (pose_a, pose_b) = (a.pose(), b.pose());
        let (stiffness, damping) = joint::spring(2.0, 0.7);
        let mut joint = GenericJointBuilder::new(JointAxesMask::LIN_Y)
            .local_axis1(pose_a.rotation.inverse().transform_vector(axis))
            .local_axis2(pose_b.rotation.inverse().transform_vector(axis))
            .local_anchor1(pose_a.inverse_transform_point(anchor))
            .local_anchor2(pose_b.inverse_transform_point(anchor))
            .contacts_enabled(collide)
            .motor_position(JointAxis::LinX, 0.0, stiffness, damping)
            .build();
        idle_motor(&mut joint, JointAxis::AngX);
        self.insert_joint(a, b, joint, JointType::Wheel)
    }

    /// Pulls the point `(x, y)` on `body` towards a target, which starts
    /// there, like a rubber band. Move the target with
    /// [`Joint::set_target`] to drag the body around with the mouse, and
    /// destroy the joint to let go.
    pub fn new_mouse_joint(&self, body: &Body, x: f32, y: f32) -> Joint {
        let ground = self.ground();
        let (stiffness, damping) = joint::spring(5.0, 0.7);
        let max_force = 1000.0 * body.mass() * self.meter();
        let mut joint = GenericJointBuilder::new(JointAxesMask::empty())
            .local_anchor1(Vector::new(x, y))
            .local_anchor2(body.pose().inverse_transform_point(Vector::new(x, y)))
            .build();
        for axis in [JointAxis::LinX, JointAxis::LinY] {
            joint.set_motor_position(axis, 0.0, stiffness, damping);
            joint.set_motor_max_force(axis, max_force);
        }
        self.insert_joint(&ground, body, joint, JointType::Mouse)
    }

    /// Slows the movement and turning of the second body relative to the
    /// first, up to a [maximum force](Joint::set_max_force) and
    /// [torque](Joint::set_max_torque), which start at 0. This is
    /// friction for games seen from above: join each thing to the ground
    /// at its center.
    pub fn new_friction_joint(&self, a: &Body, b: &Body, x: f32, y: f32, collide: bool) -> Joint {
        let anchor = Vector::new(x, y);
        let mut joint = GenericJointBuilder::new(JointAxesMask::empty())
            .local_anchor1(a.pose().inverse_transform_point(anchor))
            .local_anchor2(b.pose().inverse_transform_point(anchor))
            .contacts_enabled(collide)
            .build();
        for axis in [JointAxis::LinX, JointAxis::LinY, JointAxis::AngX] {
            idle_motor(&mut joint, axis);
            joint.motor_axes |= axis.into();
        }
        self.insert_joint(a, b, joint, JointType::Friction)
    }

    /// Holds the second body where it is now relative to the first, and
    /// pulls it back when it moves away, with no more than a
    /// [maximum force](Joint::set_max_force) and
    /// [torque](Joint::set_max_torque). Change where it is held with
    /// [`Joint::set_linear_offset`] and [`Joint::set_angular_offset`] to
    /// steer it. `correction_factor`, from 0.0 to 1.0, says how quickly it
    /// is pulled back; LÖVE uses 0.3.
    pub fn new_motor_joint(
        &self,
        a: &Body,
        b: &Body,
        correction_factor: f32,
        collide: bool,
    ) -> Joint {
        let (stiffness, damping) = joint::correction_spring(correction_factor);
        let mut joint = GenericJointBuilder::new(JointAxesMask::empty())
            .local_frame1(a.pose().inverse() * b.pose())
            .local_frame2(Pose::IDENTITY)
            .contacts_enabled(collide)
            .build();
        for axis in [JointAxis::LinX, JointAxis::LinY, JointAxis::AngX] {
            joint.set_motor_position(axis, 0.0, stiffness, damping);
        }
        self.insert_joint(a, b, joint, JointType::Motor)
    }

    /// The world's hidden ground body, made the first time it is needed.
    fn ground(&self) -> Body {
        let mut inner = self.inner.borrow_mut();
        let handle = match inner.ground {
            Some(handle) => handle,
            None => {
                let handle = inner.world.insert_body(RigidBodyBuilder::fixed());
                inner.ground = Some(handle);
                handle
            }
        };
        Body::new(&self.inner, handle)
    }

    fn insert_joint(&self, a: &Body, b: &Body, mut joint: GenericJoint, kind: JointType) -> Joint {
        assert!(
            Rc::ptr_eq(&self.inner, &a.world) && Rc::ptr_eq(&self.inner, &b.world),
            "both bodies of a joint must belong to this world"
        );
        joint::tag(&mut joint, kind);
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

/// Readies a motor that drives a speed, as Box2D's do, but leaves it
/// off and with no strength until the game turns it on.
fn idle_motor(joint: &mut GenericJoint, axis: JointAxis) {
    let motor = &mut joint.motors[axis as usize];
    motor.target_vel = 0.0;
    motor.target_pos = 0.0;
    motor.stiffness = 0.0;
    motor.damping = MOTOR_DAMPING;
    motor.max_force = 0.0;
    motor.model = MotorModel::AccelerationBased;
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World")
            .field("gravity", &self.gravity())
            .field("bodies", &self.body_count())
            .finish()
    }
}
