use glam::Vec2;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rapier2d::prelude::*;

use super::world::{Inner, Shared};
use super::{Fixture, Joint, Shape, Touch, World};

/// How a body takes part in the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyType {
    /// Never moves: ground and walls.
    Static,
    /// Moved by gravity, forces and collisions: most things.
    Dynamic,
    /// Moves at the velocity the game gives it and is not pushed by
    /// anything: moving platforms.
    Kinematic,
}

/// A thing that moves in a [`World`](super::World).
///
/// Cloning is cheap and gives another handle to the same body. A body
/// stays in its world until [`Body::destroy`] is called, even if every
/// handle is dropped. Using a destroyed body panics, except for
/// [`Body::is_destroyed`] and [`Body::destroy`].
#[derive(Clone)]
pub struct Body {
    pub(super) world: Shared,
    pub(super) handle: RigidBodyHandle,
}

impl PartialEq for Body {
    fn eq(&self, other: &Body) -> bool {
        self.handle == other.handle && Rc::ptr_eq(&self.world, &other.world)
    }
}

impl Eq for Body {}

/// Lets a body be a key in a `HashMap` or `HashSet`, so a game can keep
/// what it knows about each body in a map.
///
/// Clippy's `mutable_key_type` lint complains about such a map, wrongly:
/// a body hashes by which body it is, never by where it is. List
/// `smitten::physics::Body` under `ignore-interior-mutability` in your
/// `clippy.toml` to quiet it.
impl Hash for Body {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.handle.hash(state);
        Rc::as_ptr(&self.world).hash(state);
    }
}

impl Body {
    pub(super) fn new(world: &Shared, handle: RigidBodyHandle) -> Body {
        Body {
            world: world.clone(),
            handle,
        }
    }

    fn read<R>(&self, read: impl FnOnce(&RigidBody) -> R) -> R {
        let inner = self.world.borrow();
        read(
            inner
                .world
                .bodies
                .get(self.handle)
                .expect("this body was destroyed"),
        )
    }

    fn change<R>(&self, change: impl FnOnce(&mut RigidBody) -> R) -> R {
        let mut inner = self.world.borrow_mut();
        change(
            inner
                .world
                .bodies
                .get_mut(self.handle)
                .expect("this body was destroyed"),
        )
    }

    pub(super) fn pose(&self) -> Pose {
        self.read(|body| *body.position())
    }

    /// Whether the body has been removed from its world.
    pub fn is_destroyed(&self) -> bool {
        self.world.borrow().world.bodies.get(self.handle).is_none()
    }

    /// Removes the body from its world, with its fixtures and joints.
    /// Does nothing if it was already destroyed.
    pub fn destroy(&self) {
        let mut inner = self.world.borrow_mut();
        if let Some(body) = inner.world.bodies.get(self.handle) {
            let colliders = body.colliders().to_vec();
            for collider in colliders {
                inner.held_density.remove(&collider);
            }
        }
        inner.mass_set.remove(&self.handle);
        inner.sleepless.remove(&self.handle);
        inner.world.remove_body(self.handle);
    }

    /// The position of the body's origin as `(x, y)`.
    pub fn position(&self) -> Vec2 {
        self.read(|body| {
            let position = body.translation();
            (position.x, position.y)
        })
        .into()
    }

    /// The horizontal position of the body's origin.
    pub fn x(&self) -> f32 {
        self.position().x
    }

    /// The vertical position of the body's origin.
    pub fn y(&self) -> f32 {
        self.position().y
    }

    /// Moves the body to `(x, y)` at once, without sweeping through
    /// what lies between.
    pub fn set_position(&self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.change(|body| body.set_translation(Vector::new(x, y), true));
    }

    /// The body's angle in radians, clockwise on screen.
    pub fn angle(&self) -> f32 {
        self.read(|body| body.rotation().angle())
    }

    /// Turns the body to `angle` radians at once.
    pub fn set_angle(&self, angle: f32) {
        self.change(|body| body.set_rotation(Rotation::new(angle), true));
    }

    /// The body's velocity as `(x, y)` in units per second.
    pub fn linear_velocity(&self) -> Vec2 {
        self.read(|body| {
            let velocity = body.linvel();
            (velocity.x, velocity.y)
        })
        .into()
    }

    /// Sets the body's velocity in units per second.
    pub fn set_linear_velocity(&self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.change(|body| body.set_linvel(Vector::new(x, y), true));
    }

    /// How fast the body is turning, in radians per second.
    pub fn angular_velocity(&self) -> f32 {
        self.read(|body| body.angvel())
    }

    /// Sets how fast the body turns, in radians per second.
    pub fn set_angular_velocity(&self, velocity: f32) {
        self.change(|body| body.set_angvel(velocity, true));
    }

    /// Pushes the body's center with a force for the next update only.
    /// Call it every frame for a steady push, such as a thruster.
    ///
    /// How much a force moves a body depends on its mass, which depends on
    /// [`World::set_meter`](super::World::set_meter). To say how fast the
    /// body should speed up instead, whatever it weighs, use
    /// [`Body::apply_acceleration`].
    pub fn apply_force(&self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.change(|body| body.add_force(Vector::new(x, y), true));
    }

    /// Pushes the body with a force at the world point `(at_x, at_y)` for
    /// the next update only. Off center, this also turns it.
    pub fn apply_force_at(&self, position: impl Into<Vec2>, at: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        let Vec2 { x: at_x, y: at_y } = at.into();
        self.change(|body| {
            body.add_force_at_point(Vector::new(x, y), Vector::new(at_x, at_y), true)
        });
    }

    /// Twists the body for the next update only. Like a force, how much
    /// it turns the body depends on the body's mass and size; see
    /// [`Body::apply_angular_acceleration`].
    pub fn apply_torque(&self, torque: f32) {
        self.change(|body| body.add_torque(torque, true));
    }

    /// Speeds the body up by `(x, y)` units per second, every second, for
    /// the next update only, whatever its mass. Gravity is an acceleration
    /// of this kind. Call it every frame for a steady pull.
    pub fn apply_acceleration(&self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.change(|body| {
            let mass = body.mass();
            body.add_force(Vector::new(x, y) * mass, true);
        });
    }

    /// Speeds the body's spin up by `acceleration` radians per second,
    /// every second, for the next update only, whatever its mass and
    /// size. Good for springs and motors that should feel the same on
    /// any body.
    pub fn apply_angular_acceleration(&self, acceleration: f32) {
        self.change(|body| {
            let inertia = body.mass_properties().local_mprops.principal_inertia();
            body.add_torque(acceleration * inertia, true);
        });
    }

    /// How hard the body is to spin, the turning equivalent of
    /// [`Body::mass`]. It comes from the size, shape and density of its
    /// fixtures. A torque divided by this is the angular acceleration it
    /// gives.
    pub fn inertia(&self) -> f32 {
        self.read(|body| body.mass_properties().local_mprops.principal_inertia())
    }

    /// Changes the body's velocity at once, as a kick or a jump does. The
    /// change is the impulse divided by the body's mass.
    pub fn apply_linear_impulse(&self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.change(|body| body.apply_impulse(Vector::new(x, y), true));
    }

    /// Kicks the body at the world point `(at_x, at_y)`. Off center, this
    /// also spins it.
    pub fn apply_linear_impulse_at(&self, position: impl Into<Vec2>, at: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        let Vec2 { x: at_x, y: at_y } = at.into();
        self.change(|body| {
            body.apply_impulse_at_point(Vector::new(x, y), Vector::new(at_x, at_y), true)
        });
    }

    /// Changes how fast the body spins at once.
    pub fn apply_angular_impulse(&self, impulse: f32) {
        self.change(|body| body.apply_torque_impulse(impulse, true));
    }

    /// The body's mass, which comes from the size and density of its
    /// fixtures.
    pub fn mass(&self) -> f32 {
        self.read(|body| body.mass())
    }

    /// The body's type.
    pub fn body_type(&self) -> BodyType {
        self.read(|body| match body.body_type() {
            RigidBodyType::Fixed => BodyType::Static,
            RigidBodyType::Dynamic => BodyType::Dynamic,
            _ => BodyType::Kinematic,
        })
    }

    /// Changes the body's type.
    pub fn set_body_type(&self, body_type: BodyType) {
        let body_type = match body_type {
            BodyType::Static => RigidBodyType::Fixed,
            BodyType::Dynamic => RigidBodyType::Dynamic,
            BodyType::Kinematic => RigidBodyType::KinematicVelocityBased,
        };
        self.change(|body| body.set_body_type(body_type, true));
    }

    /// Whether the body is kept from turning.
    pub fn is_fixed_rotation(&self) -> bool {
        self.read(|body| body.is_rotation_locked())
    }

    /// Keeps the body from turning, or lets it turn again. Useful for
    /// characters that should stay upright.
    pub fn set_fixed_rotation(&self, fixed: bool) {
        self.change(|body| body.lock_rotations(fixed, true));
    }

    /// Whether the body gets the extra checks that stop fast things
    /// passing through thin ones.
    pub fn is_bullet(&self) -> bool {
        self.read(|body| body.is_ccd_enabled())
    }

    /// Turns on extra checks so a fast body cannot pass through a thin
    /// one between two updates. It costs time, so use it for bullets and
    /// the like.
    pub fn set_bullet(&self, bullet: bool) {
        self.change(|body| body.enable_ccd(bullet));
    }

    /// How quickly the body's movement dies away by itself.
    pub fn linear_damping(&self) -> f32 {
        self.read(|body| body.linear_damping())
    }

    /// Makes the body's movement die away by itself, like air resistance.
    /// Zero, the default, never slows it.
    pub fn set_linear_damping(&self, damping: f32) {
        self.change(|body| body.set_linear_damping(damping));
    }

    /// How quickly the body's spin dies away by itself.
    pub fn angular_damping(&self) -> f32 {
        self.read(|body| body.angular_damping())
    }

    /// Makes the body's spin die away by itself.
    pub fn set_angular_damping(&self, damping: f32) {
        self.change(|body| body.set_angular_damping(damping));
    }

    /// How strongly gravity acts on the body.
    pub fn gravity_scale(&self) -> f32 {
        self.read(|body| body.gravity_scale())
    }

    /// Scales gravity for this body: 0.0 floats, -1.0 falls upwards.
    pub fn set_gravity_scale(&self, scale: f32) {
        self.change(|body| body.set_gravity_scale(scale, true));
    }

    /// Whether the body is moving. Bodies that come to rest fall asleep
    /// to save time, and wake when something touches them.
    pub fn is_awake(&self) -> bool {
        self.read(|body| !body.is_sleeping())
    }

    /// Wakes the body, or puts it to sleep.
    pub fn set_awake(&self, awake: bool) {
        self.change(|body| {
            if awake {
                body.wake_up(true);
            } else {
                body.sleep();
            }
        });
    }

    /// A number the game attached to the body, 0 until one is set.
    pub fn user_data(&self) -> u64 {
        self.read(|body| body.user_data as u64)
    }

    /// Attaches a number to the body, such as an index into the game's
    /// own list of things, to recognize it in contacts and queries.
    pub fn set_user_data(&self, data: u64) {
        self.change(|body| body.user_data = u128::from(data));
    }

    /// Turns a point given relative to the body into world coordinates.
    pub fn world_point(&self, local_point: impl Into<Vec2>) -> Vec2 {
        let Vec2 {
            x: local_x,
            y: local_y,
        } = local_point.into();
        let point = self.pose().transform_point(Vector::new(local_x, local_y));
        (point.x, point.y).into()
    }

    /// Turns a world point into coordinates relative to the body.
    pub fn local_point(&self, position: impl Into<Vec2>) -> Vec2 {
        let Vec2 { x, y } = position.into();
        let point = self.pose().inverse_transform_point(Vector::new(x, y));
        (point.x, point.y).into()
    }

    /// The world the body is in.
    pub fn world(&self) -> World {
        World {
            inner: self.world.clone(),
        }
    }

    /// The body's position and angle together, as `(x, y, angle)`.
    pub fn transform(&self) -> (f32, f32, f32) {
        let Vec2 { x, y } = self.position();
        (x, y, self.angle())
    }

    /// Moves the body to `(x, y)` and turns it to `angle` at once.
    pub fn set_transform(&self, position: impl Into<Vec2>, angle: f32) {
        let Vec2 { x, y } = position.into();
        self.change(|body| body.set_position(Pose::new(Vector::new(x, y), angle), true));
    }

    /// Moves the body's origin across to `x`, keeping its height.
    pub fn set_x(&self, x: f32) {
        let y = self.y();
        self.set_position((x, y));
    }

    /// Moves the body's origin up or down to `y`.
    pub fn set_y(&self, y: f32) {
        let x = self.x();
        self.set_position((x, y));
    }

    /// The body's center of mass in world coordinates: the point it turns
    /// around when spun, and where [`Body::apply_force`] pushes.
    pub fn world_center(&self) -> Vec2 {
        self.read(|body| {
            let center = body.center_of_mass();
            (center.x, center.y)
        })
        .into()
    }

    /// The body's center of mass relative to its origin, without its
    /// turn.
    pub fn local_center(&self) -> Vec2 {
        self.read(|body| {
            let center = body.local_center_of_mass();
            (center.x, center.y)
        })
        .into()
    }

    /// Turns a direction given relative to the body into a direction in
    /// the world. Unlike [`Body::world_point`] it only turns, as a
    /// direction has no position.
    pub fn world_vector(&self, local_point: impl Into<Vec2>) -> Vec2 {
        let Vec2 {
            x: local_x,
            y: local_y,
        } = local_point.into();
        let vector = self
            .pose()
            .rotation
            .transform_vector(Vector::new(local_x, local_y));
        (vector.x, vector.y).into()
    }

    /// Turns a direction in the world into one relative to the body.
    pub fn local_vector(&self, position: impl Into<Vec2>) -> Vec2 {
        let Vec2 { x, y } = position.into();
        let vector = self
            .pose()
            .rotation
            .inverse()
            .transform_vector(Vector::new(x, y));
        (vector.x, vector.y).into()
    }

    /// Turns many points given relative to the body into world
    /// coordinates, as [`Body::world_point`] does one.
    pub fn world_points(&self, points: &[impl Into<Vec2> + Copy]) -> Vec<Vec2> {
        let points = crate::math::tuples(points);
        let points = &points[..];
        let pose = self.pose();
        points
            .iter()
            .map(|&(x, y)| {
                let point = pose.transform_point(Vector::new(x, y));
                Vec2::new(point.x, point.y)
            })
            .collect()
    }

    /// Turns many world points into coordinates relative to the body.
    pub fn local_points(&self, points: &[impl Into<Vec2> + Copy]) -> Vec<Vec2> {
        let points = crate::math::tuples(points);
        let points = &points[..];
        let pose = self.pose();
        points
            .iter()
            .map(|&(x, y)| {
                let point = pose.inverse_transform_point(Vector::new(x, y));
                Vec2::new(point.x, point.y)
            })
            .collect()
    }

    /// How fast the point of the body at world point `(x, y)` is moving,
    /// as `(x, y)` in units per second: the body's velocity plus what its
    /// spin adds there. The rim of a spinning wheel moves faster than its
    /// hub.
    pub fn linear_velocity_from_world_point(&self, position: impl Into<Vec2>) -> Vec2 {
        let Vec2 { x, y } = position.into();
        self.read(|body| {
            let velocity = body.velocity_at_point(Vector::new(x, y));
            (velocity.x, velocity.y)
        })
        .into()
    }

    /// How fast the point `(x, y)` relative to the body is moving, in
    /// world units per second.
    pub fn linear_velocity_from_local_point(&self, local_point: impl Into<Vec2>) -> Vec2 {
        let Vec2 {
            x: local_x,
            y: local_y,
        } = local_point.into();
        let Vec2 { x, y } = self.world_point((local_x, local_y));
        self.linear_velocity_from_world_point((x, y))
    }

    /// The body's `(center_x, center_y, mass, inertia)`: its center of mass
    /// relative to its origin, its mass, and its inertia around that center
    /// (as [`Body::inertia`] gives it).
    pub fn mass_data(&self) -> (f32, f32, f32, f32) {
        self.read(|body| {
            let props = &body.mass_properties().local_mprops;
            (
                props.local_com.x,
                props.local_com.y,
                props.mass(),
                props.principal_inertia(),
            )
        })
    }

    /// Sets the body's mass, center of mass relative to its origin, and
    /// inertia around that center, in place of what its fixtures give.
    /// Their densities then count for nothing until
    /// [`Body::reset_mass_data`] is called or a fixture is added. Only
    /// dynamic bodies have mass.
    pub fn set_mass_data(&self, center: impl Into<Vec2>, mass: f32, inertia: f32) {
        let Vec2 {
            x: center_x,
            y: center_y,
        } = center.into();
        let mut inner = self.world.borrow_mut();
        let Inner {
            world,
            held_density,
            mass_set,
            ..
        } = &mut *inner;
        let body = world
            .bodies
            .get_mut(self.handle)
            .expect("this body was destroyed");
        for &handle in body.colliders() {
            if let Some(collider) = world.colliders.get_mut(handle) {
                held_density
                    .entry(handle)
                    .or_insert_with(|| collider.density());
                collider.set_mass(0.0);
            }
        }
        mass_set.insert(self.handle);
        let props = MassProperties::new(
            Vector::new(center_x, center_y),
            mass.max(0.0),
            inertia.max(0.0),
        );
        body.set_additional_mass_properties(props, true);
        body.recompute_mass_properties_from_colliders(&world.colliders);
    }

    /// Sets the body's mass, keeping its center of mass and inertia.
    pub fn set_mass(&self, mass: f32) {
        let (x, y, _, inertia) = self.mass_data();
        self.set_mass_data((x, y), mass, inertia);
    }

    /// Sets how hard the body is to spin, keeping its mass and center.
    pub fn set_inertia(&self, inertia: f32) {
        let (x, y, mass, _) = self.mass_data();
        self.set_mass_data((x, y), mass, inertia);
    }

    /// Gives the body back the mass, center and inertia its fixtures'
    /// shapes and densities make, undoing [`Body::set_mass_data`].
    pub fn reset_mass_data(&self) {
        let mut inner = self.world.borrow_mut();
        assert!(
            inner.world.bodies.get(self.handle).is_some(),
            "this body was destroyed"
        );
        reset_mass(&mut inner, self.handle);
    }

    /// Every pair of fixtures touching now where one is this body's, with
    /// how hard they were pushed apart, as [`World::touching`] gives them.
    pub fn contacts(&self) -> Vec<Touch> {
        self.world()
            .touching()
            .into_iter()
            .filter(|touch| touch.involves(self))
            .collect()
    }

    /// Whether the body takes part in the simulation. See
    /// [`Body::set_active`].
    pub fn is_active(&self) -> bool {
        self.read(|body| body.is_enabled())
    }

    /// Takes the body out of the simulation, or puts it back, without
    /// destroying it: while inactive it does not move, collide or show up
    /// in queries, and its joints do nothing.
    pub fn set_active(&self, active: bool) {
        self.change(|body| body.set_enabled(active));
    }

    /// Whether the body may fall asleep when it comes to rest. See
    /// [`Body::set_sleeping_allowed`].
    pub fn is_sleeping_allowed(&self) -> bool {
        let inner = self.world.borrow();
        assert!(
            inner.world.bodies.get(self.handle).is_some(),
            "this body was destroyed"
        );
        !inner.sleepless.contains(&self.handle)
    }

    /// Keeps the body awake, or lets it fall asleep when it comes to rest
    /// as bodies do. A body the game moves by hand should stay awake.
    /// [`World::set_sleeping_allowed`] can keep every body awake.
    pub fn set_sleeping_allowed(&self, allowed: bool) {
        let mut inner = self.world.borrow_mut();
        assert!(
            inner.world.bodies.get(self.handle).is_some(),
            "this body was destroyed"
        );
        if allowed {
            inner.sleepless.remove(&self.handle);
        } else {
            inner.sleepless.insert(self.handle);
        }
        inner.apply_sleeping(self.handle);
    }

    /// The body's fixtures.
    pub fn fixtures(&self) -> Vec<Fixture> {
        self.read(|body| body.colliders().to_vec())
            .into_iter()
            .map(|handle| Fixture::new(&self.world, handle))
            .collect()
    }

    /// The joints attached to the body.
    pub fn joints(&self) -> Vec<Joint> {
        let inner = self.world.borrow();
        inner
            .world
            .impulse_joints
            .attached_joints(self.handle)
            .map(|(_, _, handle, _)| Joint::new(&self.world, handle))
            .collect()
    }

    /// Whether any of this body's fixtures is touching one of `other`'s.
    /// Sensors do not count.
    pub fn is_touching(&self, other: &Body) -> bool {
        let inner = self.world.borrow();
        let (Some(mine), Some(theirs)) = (
            inner.world.bodies.get(self.handle),
            inner.world.bodies.get(other.handle),
        ) else {
            return false;
        };
        mine.colliders().iter().any(|&a| {
            theirs.colliders().iter().any(|&b| {
                inner
                    .world
                    .contact_pair(a, b)
                    .is_some_and(|pair| pair.has_any_active_contact())
            })
        })
    }

    fn add(&self, collider: ColliderBuilder) -> Fixture {
        let mut inner = self.world.borrow_mut();
        assert!(
            inner.world.bodies.get(self.handle).is_some(),
            "this body was destroyed"
        );
        // Density is given per square meter; the engine wants
        // it per square unit.
        let density = super::fixture::DEFAULT_DENSITY / (inner.meter * inner.meter);
        let collider = collider
            .density(density)
            .friction(super::fixture::DEFAULT_FRICTION)
            .restitution(0.0)
            // As in Box2D: the bouncier of two fixtures decides the bounce.
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            // So that the pre-solve callback and contact filter hear about
            // it.
            .active_hooks(
                ActiveHooks::MODIFY_SOLVER_CONTACTS
                    | ActiveHooks::FILTER_CONTACT_PAIRS
                    | ActiveHooks::FILTER_INTERSECTION_PAIR,
            );
        let handle = inner.world.insert_collider(collider, Some(self.handle));
        // As in Box2D, a new fixture puts back the mass its fixtures give.
        if inner.mass_set.contains(&self.handle) {
            reset_mass(&mut inner, self.handle);
        }
        Fixture::new(&self.world, handle)
    }

    /// Gives the body a copy of `shape` as a new fixture. The fixture's
    /// density starts at 1.0.
    pub fn add_shape(&self, shape: &Shape) -> Fixture {
        let fixture = self.add(ColliderBuilder::new(shape.shape.clone()).position(shape.offset));
        fixture.set_ghosts(shape);
        fixture
    }

    /// Gives the body a circle centered on its origin.
    pub fn add_circle(&self, radius: f32) -> Fixture {
        self.add_shape(&Shape::circle(radius))
    }

    /// Gives the body a circle centered `(x, y)` from its origin.
    pub fn add_circle_at(&self, position: impl Into<Vec2>, radius: f32) -> Fixture {
        let Vec2 { x, y } = position.into();
        self.add_shape(&Shape::circle_at((x, y), radius))
    }

    /// Gives the body a rectangle of `width` by `height` centered on its
    /// origin.
    pub fn add_rectangle(&self, size: impl Into<Vec2>) -> Fixture {
        let Vec2 {
            x: width,
            y: height,
        } = size.into();
        self.add_shape(&Shape::rectangle((width, height)))
    }

    /// Gives the body a rectangle centered `(x, y)` from its origin and
    /// turned by `angle` radians.
    pub fn add_rectangle_at(
        &self,
        position: impl Into<Vec2>,
        size: impl Into<Vec2>,
        angle: f32,
    ) -> Fixture {
        let Vec2 { x, y } = position.into();
        let Vec2 {
            x: width,
            y: height,
        } = size.into();
        self.add_shape(&Shape::rectangle_at((x, y), (width, height), angle))
    }

    /// Gives the body a polygon with these corners, measured from its
    /// origin. A polygon with dents is filled out to the convex shape
    /// around it; build a dented shape from several fixtures instead.
    ///
    /// Panics if the points do not span an area.
    pub fn add_polygon(&self, points: &[impl Into<Vec2> + Copy]) -> Fixture {
        let points = crate::math::tuples(points);
        let points = &points[..];
        self.add_shape(&Shape::polygon(points))
    }

    /// Gives the body a thin line from `(x1, y1)` to `(x2, y2)`, measured
    /// from its origin. Best on static bodies, as ground or walls.
    pub fn add_edge(&self, anchor1: impl Into<Vec2>, anchor2: impl Into<Vec2>) -> Fixture {
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
        self.add_shape(&Shape::edge((x1, y1), (x2, y2)))
    }

    /// Gives the body a chain of lines through these points, measured
    /// from its origin, joined back to the start if `closed`. Best on
    /// static bodies, for uneven ground.
    ///
    /// Panics with fewer than two points.
    pub fn add_chain(&self, points: &[impl Into<Vec2> + Copy], closed: bool) -> Fixture {
        let points = crate::math::tuples(points);
        let points = &points[..];
        self.add_shape(&Shape::chain(points, closed))
    }
}

/// Puts back the mass a body's fixtures give it.
fn reset_mass(inner: &mut Inner, handle: RigidBodyHandle) {
    let Inner {
        world,
        held_density,
        mass_set,
        ..
    } = inner;
    mass_set.remove(&handle);
    let Some(body) = world.bodies.get_mut(handle) else {
        return;
    };
    for collider in body.colliders() {
        if let (Some(density), Some(fixture)) = (
            held_density.remove(collider),
            world.colliders.get_mut(*collider),
        ) {
            fixture.set_density(density);
        }
    }
    body.set_additional_mass_properties(MassProperties::default(), true);
    body.recompute_mass_properties_from_colliders(&world.colliders);
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_destroyed() {
            return f.write_str("Body(destroyed)");
        }
        f.debug_struct("Body")
            .field("type", &self.body_type())
            .field("position", &self.position())
            .field("angle", &self.angle())
            .finish()
    }
}
