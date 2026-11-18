use glam::Vec2;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc;

use rapier2d::prelude::*;

use super::coupling::{self, Coupling};
use super::ghost::{self, Ghosts};
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
    contact_filter: Option<ContactFilterCallback>,
    /// Whether bodies may fall asleep at all. See
    /// [`World::set_sleeping_allowed`].
    pub sleeping_allowed: bool,
    /// Bodies told they may not sleep with [`Body::set_sleeping_allowed`].
    pub sleepless: HashSet<RigidBodyHandle>,
    /// The densities of fixtures whose body's mass was set by hand, which
    /// count for nothing until [`Body::reset_mass_data`]. Per square unit.
    pub held_density: HashMap<ColliderHandle, f32>,
    /// Bodies whose mass was set by hand with [`Body::set_mass_data`].
    pub mass_set: HashSet<RigidBodyHandle>,
    /// What each gear and pulley joint ties together.
    pub couplings: HashMap<ImpulseJointHandle, Coupling>,
    /// The ghost vertices of edges and chains that have them.
    pub ghosts: HashMap<ColliderHandle, Ghosts>,
}

type PreSolveCallback = Box<dyn FnMut(&mut PreSolve)>;
type ContactFilterCallback = Box<dyn FnMut(&ContactFilter) -> bool>;

impl Inner {
    /// Lets a body fall asleep, or keeps it awake, as the world's and the
    /// body's own settings say.
    pub fn apply_sleeping(&mut self, handle: RigidBodyHandle) {
        let allowed = self.sleeping_allowed && !self.sleepless.contains(&handle);
        let Some(body) = self.world.bodies.get_mut(handle) else {
            return;
        };
        let activation = body.activation_mut();
        if allowed {
            activation.normalized_linear_threshold =
                RigidBodyActivation::default_normalized_linear_threshold();
            activation.angular_threshold = RigidBodyActivation::default_angular_threshold();
        } else {
            activation.normalized_linear_threshold = -1.0;
            activation.angular_threshold = -1.0;
            body.wake_up(true);
        }
    }
}

/// A contact as the engine reported it, before handles are made for the
/// game.
#[derive(Clone)]
struct RawContact {
    began: bool,
    a: ColliderHandle,
    b: ColliderHandle,
    point: Vector,
    normal: Vector,
    speed: f32,
    points: Vec<(f32, f32)>,
    children: (usize, usize),
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
    /// Every point where they touch, in world coordinates: one or two,
    /// or none for a sensor or once they have stopped touching. `x` and
    /// `y` are the middle of these. LÖVE's `Contact:getPositions`.
    pub points: Vec<(f32, f32)>,
    /// Which piece of each fixture is touched: the index of the segment
    /// of a chain, 0 for any other shape. LÖVE's `Contact:getChildren`.
    pub children: (usize, usize),
}

impl Contact {
    /// Whether one of the two fixtures belongs to `body`.
    pub fn involves(&self, body: &Body) -> bool {
        self.a.body().as_ref() == Some(body) || self.b.body().as_ref() == Some(body)
    }
}

/// Where two fixtures touch, as worked out by [`touching_at`].
struct Touching {
    /// The middle of the points.
    point: Vector,
    /// From the first fixture into the second, one unit long.
    normal: Vector,
    points: Vec<(f32, f32)>,
    /// The pieces of the first and second fixture that touch.
    children: (usize, usize),
}

/// Where two fixtures touch and the direction from the first into the
/// second there. Without contact points to go by (a sensor, or two that
/// no longer touch) this is half way between them.
fn touching_at(world: &PhysicsWorld, a: ColliderHandle, b: ColliderHandle) -> Touching {
    let place = |handle| {
        world
            .colliders
            .get(handle)
            .map(|collider| *collider.position())
    };
    let (Some(pose_a), Some(pose_b)) = (place(a), place(b)) else {
        // One of them was destroyed.
        return Touching {
            point: Vector::ZERO,
            normal: Vector::new(0.0, 1.0),
            points: Vec::new(),
            children: (0, 0),
        };
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
    let between = Touching {
        point: (center_a + center_b) / 2.0,
        normal: apart,
        points: Vec::new(),
        children: (0, 0),
    };

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
    let points = manifold
        .points
        .iter()
        .map(|contact| {
            let point = to_world(contact.local_p1);
            (point.x, point.y)
        })
        .collect();
    let mut normal = unit(to_world(local + manifold.local_n1) - point, apart);
    let mut children = (manifold.subshape1 as usize, manifold.subshape2 as usize);
    if pair.collider1 != a {
        normal = -normal;
        children = (children.1, children.0);
    }
    Touching {
        point,
        normal,
        points,
        children,
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
    /// Every point where they touch, one or two, in world coordinates.
    pub points: Vec<(f32, f32)>,
    /// Which piece of each fixture is touched: the index of the segment
    /// of a chain, 0 for any other shape.
    pub children: (usize, usize),
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
    /// What `friction` and `restitution` started as.
    fixture_friction: f32,
    fixture_restitution: f32,
}

impl PreSolve {
    /// Puts [`PreSolve::friction`] back to the value mixed from the two
    /// fixtures, the equivalent of `Contact:resetFriction`. Changes only
    /// ever last one update here, so this undoes one made earlier in the
    /// same callback.
    pub fn reset_friction(&mut self) {
        self.friction = self.fixture_friction;
    }

    /// Puts [`PreSolve::restitution`] back to the value mixed from the
    /// two fixtures, the equivalent of `Contact:resetRestitution`.
    pub fn reset_restitution(&mut self) {
        self.restitution = self.fixture_restitution;
    }
}

/// Two fixtures about to be checked for a collision, handed to the
/// callback given to [`World::set_contact_filter`], the equivalent of
/// the two fixtures LÖVE's contact filter is called with.
///
/// As in [`PreSolve`], do not call methods on `a` and `b` inside the
/// callback: compare them, or use the user data copied here.
#[derive(Debug)]
pub struct ContactFilter {
    pub a: Fixture,
    pub b: Fixture,
    /// [`Fixture::user_data`] of `a` and `b`.
    pub a_data: u64,
    pub b_data: u64,
    /// [`Body::user_data`] of the bodies of `a` and `b`.
    pub a_body_data: u64,
    pub b_body_data: u64,
}

/// The callbacks run from inside the engine's step, and the world they
/// belong to.
struct Callbacks {
    world: Shared,
    pre_solve: Option<PreSolveCallback>,
    contact_filter: Option<ContactFilterCallback>,
    ghosts: HashMap<ColliderHandle, Ghosts>,
}

thread_local! {
    /// The world being stepped and its callbacks, for [`Hooks`]. The
    /// engine wants hooks it could share between threads, which the
    /// callbacks are not; the step runs on this thread, so they wait here.
    static CALLBACKS: RefCell<Option<Callbacks>> = const { RefCell::new(None) };
}

/// Runs the game's callbacks from inside the engine's step.
struct Hooks;

/// The user data of a fixture and of its body.
fn user_data(colliders: &ColliderSet, bodies: &RigidBodySet, handle: ColliderHandle) -> (u64, u64) {
    colliders.get(handle).map_or((0, 0), |collider| {
        let body = collider
            .parent()
            .and_then(|parent| bodies.get(parent))
            .map_or(0, |body| body.user_data as u64);
        (collider.user_data as u64, body)
    })
}

impl Hooks {
    /// Whether the game's contact filter lets the two collide.
    fn allowed(context: &PairFilterContext) -> bool {
        let (a, b) = (context.collider1, context.collider2);
        let (a_data, a_body_data) = user_data(context.colliders, context.bodies, a);
        let (b_data, b_body_data) = user_data(context.colliders, context.bodies, b);
        CALLBACKS.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(callbacks) = slot.as_mut() else {
                return true;
            };
            let Some(filter) = callbacks.contact_filter.as_mut() else {
                return true;
            };
            filter(&ContactFilter {
                a: Fixture::new(&callbacks.world, a),
                b: Fixture::new(&callbacks.world, b),
                a_data,
                b_data,
                a_body_data,
                b_body_data,
            })
        })
    }
}

impl PhysicsHooks for Hooks {
    fn filter_contact_pair(&self, context: &PairFilterContext) -> Option<SolverFlags> {
        Hooks::allowed(context).then(SolverFlags::default)
    }

    fn filter_intersection_pair(&self, context: &PairFilterContext) -> bool {
        Hooks::allowed(context)
    }

    fn modify_solver_contacts(&self, context: &mut ContactModificationContext) {
        CALLBACKS.with(|slot| {
            if let Some(callbacks) = slot.borrow().as_ref() {
                ghost::smooth(context, &callbacks.ghosts);
            }
        });
        let (a, b) = (context.collider1, context.collider2);
        let (a_data, a_body_data) = user_data(context.colliders, context.bodies, a);
        let (b_data, b_body_data) = user_data(context.colliders, context.bodies, b);
        let Some(manifold) = context.rigid_mut() else {
            return;
        };
        let count = manifold.solver_contacts.len().max(1) as f32;
        let point = manifold
            .solver_contacts
            .iter()
            .fold(Vector::ZERO, |sum, contact| sum + contact.anchor1)
            / count;
        CALLBACKS.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(callbacks) = slot.as_mut() else {
                return;
            };
            let Some(callback) = callbacks.pre_solve.as_mut() else {
                return;
            };
            let mut contact = PreSolve {
                a: Fixture::new(&callbacks.world, a),
                b: Fixture::new(&callbacks.world, b),
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
                fixture_friction: *manifold.friction,
                fixture_restitution: *manifold.restitution,
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
    pub fn new(gravity: impl Into<Vec2>) -> World {
        let Vec2 {
            x: gravity_x,
            y: gravity_y,
        } = gravity.into();
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
                contact_filter: None,
                sleeping_allowed: true,
                sleepless: HashSet::new(),
                held_density: HashMap::new(),
                mass_set: HashSet::new(),
                couplings: HashMap::new(),
                ghosts: HashMap::new(),
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
        // The callbacks are taken out while the step runs, so the world
        // can be borrowed for them. The hooks always run, because joins
        // between lines are smoothed in them.
        let callbacks = Callbacks {
            world: self.inner.clone(),
            pre_solve: inner.pre_solve.take(),
            contact_filter: inner.contact_filter.take(),
            ghosts: std::mem::take(&mut inner.ghosts),
        };
        CALLBACKS.with(|slot| *slot.borrow_mut() = Some(callbacks));
        inner.world.step_with_events(&Hooks, &collector);
        if let Some(callbacks) = CALLBACKS.with(|slot| slot.borrow_mut().take()) {
            inner.pre_solve = callbacks.pre_solve;
            inner.contact_filter = callbacks.contact_filter;
            inner.ghosts = callbacks.ghosts;
        }
        coupling::solve(&mut inner);
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
            let touching = touching_at(&inner.world, a, b);
            let (point, normal) = (touching.point, touching.normal);
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
                points: if began { touching.points } else { Vec::new() },
                children: touching.children,
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
                points: contact.points.clone(),
                children: contact.children,
            })
            .collect()
    }

    /// Gravity as `(x, y)` in units per second, every second.
    pub fn gravity(&self) -> Vec2 {
        let gravity = self.inner.borrow().world.gravity;
        (gravity.x, gravity.y).into()
    }

    /// Changes gravity and wakes every body so it takes effect at once.
    pub fn set_gravity(&self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
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
    pub fn new_body(&self, position: impl Into<Vec2>, body_type: BodyType) -> Body {
        let Vec2 { x, y } = position.into();
        let builder = match body_type {
            BodyType::Static => RigidBodyBuilder::fixed(),
            BodyType::Dynamic => RigidBodyBuilder::dynamic(),
            BodyType::Kinematic => RigidBodyBuilder::kinematic_velocity_based(),
        };
        let mut inner = self.inner.borrow_mut();
        let handle = inner
            .world
            .insert_body(builder.translation(Vector::new(x, y)));
        if !inner.sleeping_allowed {
            inner.apply_sleeping(handle);
        }
        drop(inner);
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
            let touching = touching_at(&inner.world, a, b);
            let (point, normal) = (touching.point, touching.normal);
            touches.push(Touch {
                a: Fixture::new(&self.inner, a),
                b: Fixture::new(&self.inner, b),
                x: point.x,
                y: point.y,
                normal: (normal.x, normal.y),
                normal_impulse,
                tangent_impulse,
                points: touching.points,
                children: touching.children,
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
    /// let world = World::new((0.0, 600.0));
    /// let platform = world.new_body((0.0, 0.0), BodyType::Static);
    /// let top = platform.add_rectangle((200.0, 10.0));
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

    /// Calls `callback` for pairs of fixtures that come close, during
    /// each [`World::update`], to decide whether they collide: `false`
    /// lets them pass through each other and leaves them out of
    /// [`World::contacts`]. This is LÖVE's `World:setContactFilter`.
    ///
    /// It is asked only about fixtures whose [filters](Fixture::set_filter)
    /// already let them collide, and is asked again at each update while
    /// they stay close, so its answer may change. As with
    /// [`World::set_pre_solve`], the callback must not call methods of the
    /// world or of its bodies, fixtures and joints.
    ///
    /// ```
    /// use smitten::physics::{BodyType, World};
    ///
    /// let world = World::new((0.0, 600.0));
    /// let ghost = world.new_body((0.0, 0.0), BodyType::Dynamic);
    /// ghost.add_circle(10.0).set_user_data(1);
    /// // Fixtures with user data 1 pass through everything.
    /// world.set_contact_filter(|pair| pair.a_data != 1 && pair.b_data != 1);
    /// ```
    pub fn set_contact_filter(&self, callback: impl FnMut(&ContactFilter) -> bool + 'static) {
        self.inner.borrow_mut().contact_filter = Some(Box::new(callback));
    }

    /// Stops calling the callback given to [`World::set_contact_filter`],
    /// so fixtures collide as their filters say.
    pub fn clear_contact_filter(&self) {
        self.inner.borrow_mut().contact_filter = None;
    }

    /// Whether a callback was given to [`World::set_contact_filter`].
    pub fn has_contact_filter(&self) -> bool {
        self.inner.borrow().contact_filter.is_some()
    }

    /// Whether bodies that come to rest may fall asleep. `true` at first.
    pub fn is_sleeping_allowed(&self) -> bool {
        self.inner.borrow().sleeping_allowed
    }

    /// Lets bodies that come to rest fall asleep, which saves time, or
    /// keeps every body awake and wakes those asleep now. A body can also
    /// be kept awake by itself with [`Body::set_sleeping_allowed`].
    pub fn set_sleeping_allowed(&self, allowed: bool) {
        let mut inner = self.inner.borrow_mut();
        inner.sleeping_allowed = allowed;
        let handles: Vec<RigidBodyHandle> = inner.world.bodies.iter().map(|(h, _)| h).collect();
        for handle in handles {
            inner.apply_sleeping(handle);
        }
    }

    /// How many pairs of fixtures are close enough to be checked for
    /// touching: those whose bounding boxes overlap, touching or not, as
    /// LÖVE's `World:getContactCount` counts them.
    pub fn contact_count(&self) -> usize {
        let inner = self.inner.borrow();
        inner.world.contact_pairs().count() + inner.world.intersection_pairs().count()
    }

    /// Whether the world is in the middle of an update, which is so only
    /// inside the callbacks given to [`World::set_pre_solve`] and
    /// [`World::set_contact_filter`]. The world cannot be changed then.
    pub fn is_locked(&self) -> bool {
        self.inner.try_borrow_mut().is_err()
    }

    /// Moves the world's origin to `(x, y)`: every body, and every mouse
    /// joint's target, moves by `(-x, -y)`, without changing how anything
    /// moves. Useful in a large world, to keep the numbers near the player
    /// small, where they are precise.
    pub fn translate_origin(&self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        let shift = Vector::new(x, y);
        let mut inner = self.inner.borrow_mut();
        let ground = inner.ground;
        let world = &mut inner.world;
        for (handle, body) in world.bodies.iter_mut() {
            if Some(handle) == ground {
                continue;
            }
            let mut pose = *body.position();
            pose.translation -= shift;
            body.set_position(pose, false);
        }
        world
            .bodies
            .propagate_modified_body_positions_to_colliders(&mut world.colliders);
        if let Some(ground) = ground {
            let mice: Vec<ImpulseJointHandle> = world
                .impulse_joints
                .attached_joints(ground)
                .map(|(_, _, handle, _)| handle)
                .collect();
            for handle in mice {
                if let Some(joint) = world.impulse_joints.get_mut(handle, false) {
                    let anchor = joint.data.local_anchor1();
                    joint.data.set_local_anchor1(anchor - shift);
                }
            }
        }
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
    pub fn ray_cast_all(&self, anchor1: impl Into<Vec2>, anchor2: impl Into<Vec2>) -> Vec<RayHit> {
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
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
    pub fn ray_cast(&self, anchor1: impl Into<Vec2>, anchor2: impl Into<Vec2>) -> Option<RayHit> {
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
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
    pub fn fixtures_at(&self, position: impl Into<Vec2>) -> Vec<Fixture> {
        let Vec2 { x, y } = position.into();
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
    pub fn new_revolute_joint(
        &self,
        a: &Body,
        b: &Body,
        position: impl Into<Vec2>,
        collide: bool,
    ) -> Joint {
        let Vec2 { x, y } = position.into();
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
        anchor1: impl Into<Vec2>,
        anchor2: impl Into<Vec2>,
        max_length: f32,
        collide: bool,
    ) -> Joint {
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
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
        anchor1: impl Into<Vec2>,
        anchor2: impl Into<Vec2>,
        collide: bool,
    ) -> Joint {
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
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
        anchor1: impl Into<Vec2>,
        anchor2: impl Into<Vec2>,
        stiffness: f32,
        damping: f32,
    ) -> Joint {
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
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
        position: impl Into<Vec2>,
        axis: impl Into<Vec2>,
        collide: bool,
    ) -> Joint {
        let Vec2 { x, y } = position.into();
        let Vec2 {
            x: axis_x,
            y: axis_y,
        } = axis.into();
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
        position: impl Into<Vec2>,
        axis: impl Into<Vec2>,
        collide: bool,
    ) -> Joint {
        let Vec2 { x, y } = position.into();
        let Vec2 {
            x: axis_x,
            y: axis_y,
        } = axis.into();
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
    pub fn new_mouse_joint(&self, body: &Body, position: impl Into<Vec2>) -> Joint {
        let Vec2 { x, y } = position.into();
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
    pub fn new_friction_joint(
        &self,
        a: &Body,
        b: &Body,
        position: impl Into<Vec2>,
        collide: bool,
    ) -> Joint {
        let Vec2 { x, y } = position.into();
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

    /// Hangs two bodies from a rope over two fixed pulleys, at
    /// `(ground_x1, ground_y1)` and `(ground_x2, ground_y2)`. The rope is
    /// tied to `(x1, y1)` on the first body and `(x2, y2)` on the second,
    /// and keeps `length_a + ratio * length_b` as it is now, so when one
    /// side goes down the other comes up. A `ratio` above 1 makes the
    /// second side move less and pull harder, like a block and tackle.
    ///
    /// Box2D's pulley holds the rope taut both ways, as this does: it
    /// never goes slack. Rapier has no pulley, so smitten enforces it
    /// itself after each step, on top of the engine's own joints and
    /// contacts. One difference shows: while one side rests on the
    /// ground, the two keep a slight speed against each other, too small
    /// to see, and so never fall asleep.
    #[allow(clippy::too_many_arguments)] // Mirrors love.physics.newPulleyJoint.
    pub fn new_pulley_joint(
        &self,
        a: &Body,
        b: &Body,
        ground1: impl Into<Vec2>,
        ground2: impl Into<Vec2>,
        anchor1: impl Into<Vec2>,
        anchor2: impl Into<Vec2>,
        ratio: f32,
        collide: bool,
    ) -> Joint {
        let Vec2 {
            x: ground_x1,
            y: ground_y1,
        } = ground1.into();
        let Vec2 {
            x: ground_x2,
            y: ground_y2,
        } = ground2.into();
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
        assert!(
            ratio.is_finite() && ratio > 0.0,
            "a pulley's ratio must be positive, got {ratio}"
        );
        let (ground_a, ground_b) = (
            Vector::new(ground_x1, ground_y1),
            Vector::new(ground_x2, ground_y2),
        );
        let (first, second) = (Vector::new(x1, y1), Vector::new(x2, y2));
        let constant = (first - ground_a).length() + ratio * (second - ground_b).length();
        let data = GenericJointBuilder::new(JointAxesMask::empty())
            .local_anchor1(a.pose().inverse_transform_point(first))
            .local_anchor2(b.pose().inverse_transform_point(second))
            .contacts_enabled(collide)
            .build();
        let joint = self.insert_joint(a, b, data, JointType::Pulley);
        self.inner.borrow_mut().couplings.insert(
            joint.handle,
            Coupling::Pulley {
                ground_a,
                ground_b,
                ratio,
                constant,
                impulse: 0.0,
            },
        );
        joint
    }

    /// Ties two revolute or prismatic joints together like gears, so that
    /// `angle_a + ratio * angle_b` stays as it is now, where each is the
    /// joint's angle (or, for a prismatic joint, how far it has slid).
    /// With a `ratio` of 2, the second joint turns half as far as the
    /// first, the other way. A revolute and a prismatic joint make a rack
    /// and pinion.
    ///
    /// The gear joint connects the second bodies of the two joints, which
    /// are usually each pinned to the ground. Destroy the gear joint
    /// before either of its joints; once one is gone the gear does
    /// nothing.
    ///
    /// Rapier has no gear joint, so smitten enforces it itself after each
    /// step, on top of the engine's own joints and contacts.
    pub fn new_gear_joint(
        &self,
        joint_a: &Joint,
        joint_b: &Joint,
        ratio: f32,
        collide: bool,
    ) -> Joint {
        assert!(
            ratio.is_finite() && ratio != 0.0,
            "a gear's ratio must be a number other than 0, got {ratio}"
        );
        for joint in [joint_a, joint_b] {
            assert!(
                Rc::ptr_eq(&self.inner, &joint.world),
                "both joints of a gear must belong to this world"
            );
            let kind = joint.kind();
            assert!(
                matches!(kind, JointType::Revolute | JointType::Prismatic),
                "a gear joint ties revolute and prismatic joints, not a {kind:?} joint"
            );
        }
        let (handle_a, handle_b) = (joint_a.handle, joint_b.handle);
        let (body_a, body_b, anchor_a, anchor_b, constant) = {
            let inner = self.inner.borrow();
            let world = &inner.world;
            let joint = |handle| {
                world
                    .impulse_joints
                    .get(handle)
                    .expect("a gear's joints must not be destroyed")
            };
            let (one, two) = (joint(handle_a), joint(handle_b));
            let mut rates = Vec::new();
            let a = coupling::joint_coordinate(world, handle_a, &mut rates, 1.0)
                .expect("the first joint exists");
            let b = coupling::joint_coordinate(world, handle_b, &mut rates, 1.0)
                .expect("the second joint exists");
            (
                one.body2(),
                two.body2(),
                one.data.local_anchor2(),
                two.data.local_anchor2(),
                a + ratio * b,
            )
        };
        let data = GenericJointBuilder::new(JointAxesMask::empty())
            .local_anchor1(anchor_a)
            .local_anchor2(anchor_b)
            .contacts_enabled(collide)
            .build();
        let (a, b) = (
            Body::new(&self.inner, body_a),
            Body::new(&self.inner, body_b),
        );
        let joint = self.insert_joint(&a, &b, data, JointType::Gear);
        self.inner.borrow_mut().couplings.insert(
            joint.handle,
            Coupling::Gear {
                joint_a: handle_a,
                joint_b: handle_b,
                ratio,
                constant,
                impulse: 0.0,
            },
        );
        joint
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
            graphics::circle(DrawMode::Line, (x, y), radius);
        }
        for polygon in polygons {
            graphics::polygon(DrawMode::Line, &polygon);
        }
        for line in lines {
            for pair in line.windows(2) {
                graphics::line((pair[0].0, pair[0].1), (pair[1].0, pair[1].1));
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
