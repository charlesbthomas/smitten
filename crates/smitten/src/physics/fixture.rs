use glam::Vec2;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rapier2d::prelude::*;

use super::world::Shared;
use super::{Body, RayHit, Shape};

/// Box2D's defaults, which LÖVE games are tuned around.
pub(super) const DEFAULT_DENSITY: f32 = 1.0;
pub(super) const DEFAULT_FRICTION: f32 = 0.2;

/// A shape attached to a [`Body`], the part that collides. The equivalent
/// of LÖVE's `Fixture` and its `Shape` together.
///
/// Make one with the `add_` methods of [`Body`]. Cloning is cheap and
/// gives another handle to the same fixture. Using a destroyed fixture
/// panics, except for [`Fixture::is_destroyed`], [`Fixture::destroy`] and
/// [`Fixture::body`].
#[derive(Clone)]
pub struct Fixture {
    pub(super) world: Shared,
    pub(super) handle: ColliderHandle,
}

impl PartialEq for Fixture {
    fn eq(&self, other: &Fixture) -> bool {
        self.handle == other.handle && Rc::ptr_eq(&self.world, &other.world)
    }
}

impl Eq for Fixture {}

/// Lets a fixture be a key in a `HashMap` or `HashSet`.
impl Hash for Fixture {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.handle.hash(state);
        Rc::as_ptr(&self.world).hash(state);
    }
}

impl Fixture {
    pub(super) fn new(world: &Shared, handle: ColliderHandle) -> Fixture {
        Fixture {
            world: world.clone(),
            handle,
        }
    }

    fn read<R>(&self, read: impl FnOnce(&Collider) -> R) -> R {
        let inner = self.world.borrow();
        read(
            inner
                .world
                .colliders
                .get(self.handle)
                .expect("this fixture was destroyed"),
        )
    }

    fn change<R>(&self, change: impl FnOnce(&mut Collider) -> R) -> R {
        let mut inner = self.world.borrow_mut();
        change(
            inner
                .world
                .colliders
                .get_mut(self.handle)
                .expect("this fixture was destroyed"),
        )
    }

    /// Whether the fixture, or the body it was on, has been destroyed.
    pub fn is_destroyed(&self) -> bool {
        self.world
            .borrow()
            .world
            .colliders
            .get(self.handle)
            .is_none()
    }

    /// Removes the fixture from its body. Does nothing if it was already
    /// destroyed.
    pub fn destroy(&self) {
        let mut inner = self.world.borrow_mut();
        inner.held_density.remove(&self.handle);
        inner.ghosts.remove(&self.handle);
        inner.world.remove_collider(self.handle);
    }

    /// The body the fixture is attached to, or `None` if the fixture was
    /// destroyed.
    pub fn body(&self) -> Option<Body> {
        let inner = self.world.borrow();
        let parent = inner.world.colliders.get(self.handle)?.parent()?;
        Some(Body::new(&self.world, parent))
    }

    /// How much the fixture grips what it slides along, from 0.0 (ice)
    /// upwards. 0.2 by default.
    pub fn friction(&self) -> f32 {
        self.read(|collider| collider.friction())
    }

    /// Sets how much the fixture grips what it slides along.
    pub fn set_friction(&self, friction: f32) -> &Fixture {
        self.change(|collider| collider.set_friction(friction));
        self
    }

    /// How bouncy the fixture is, from 0.0 (no bounce, the default) to
    /// 1.0 (bounces back as fast as it arrived). When two fixtures meet,
    /// the bouncier one decides.
    pub fn restitution(&self) -> f32 {
        self.read(|collider| collider.restitution())
    }

    /// Sets how bouncy the fixture is.
    pub fn set_restitution(&self, restitution: f32) -> &Fixture {
        self.change(|collider| collider.set_restitution(restitution));
        self
    }

    /// The fixture's density: its mass for each square meter of its area.
    /// 1.0 by default.
    pub fn density(&self) -> f32 {
        let inner = self.world.borrow();
        let meter = inner.meter;
        if let Some(&held) = inner.held_density.get(&self.handle) {
            return held * meter * meter;
        }
        drop(inner);
        self.read(|collider| collider.density()) * meter * meter
    }

    /// Sets the fixture's density, which changes its body's mass. If the
    /// body's mass was set with
    /// [`Body::set_mass_data`](super::Body::set_mass_data), it counts once
    /// that is undone with [`Body::reset_mass_data`].
    pub fn set_density(&self, density: f32) -> &Fixture {
        let mut inner = self.world.borrow_mut();
        let density = density / (inner.meter * inner.meter);
        if let Some(held) = inner.held_density.get_mut(&self.handle) {
            *held = density;
            return self;
        }
        let world = &mut inner.world;
        let collider = world
            .colliders
            .get_mut(self.handle)
            .expect("this fixture was destroyed");
        collider.set_density(density);
        // Update the body's mass now, not at the next step, so that
        // `Body::mass` is right straight away.
        if let Some(body) = collider
            .parent()
            .and_then(|parent| world.bodies.get_mut(parent))
        {
            body.recompute_mass_properties_from_colliders(&world.colliders);
        }
        drop(inner);
        self
    }

    /// Whether the fixture only detects overlaps without colliding.
    pub fn is_sensor(&self) -> bool {
        self.read(|collider| collider.is_sensor())
    }

    /// Makes the fixture a sensor: things pass through it, and it still
    /// shows up in [`World::contacts`](super::World::contacts). Useful
    /// for triggers and pickups.
    pub fn set_sensor(&self, sensor: bool) -> &Fixture {
        self.change(|collider| collider.set_sensor(sensor));
        self
    }

    /// Limits what the fixture collides with. `categories` are the groups
    /// it belongs to and `mask` the groups it collides with, each a set of
    /// 32 bits. Two fixtures collide only if each one's mask includes one
    /// of the other's categories. By default a fixture is in every group
    /// and collides with every group.
    pub fn set_filter(&self, categories: u32, mask: u32) -> &Fixture {
        self.change(|collider| {
            collider.set_collision_groups(InteractionGroups::new(
                Group::from_bits_truncate(categories),
                Group::from_bits_truncate(mask),
                InteractionTestMode::And,
            ));
        });
        self
    }

    /// The fixture's `(categories, mask)`. See [`Fixture::set_filter`].
    pub fn filter(&self) -> (u32, u32) {
        self.read(|collider| {
            let groups = collider.collision_groups();
            (groups.memberships.bits(), groups.filter.bits())
        })
    }

    /// Whether the fixture covers the world point `(x, y)`.
    pub fn test_point(&self, position: impl Into<Vec2>) -> bool {
        let Vec2 { x, y } = position.into();
        self.read(|collider| {
            collider
                .shape()
                .contains_point(collider.position(), Vector::new(x, y))
        })
    }

    /// The smallest box around the fixture, as `(left, top, right,
    /// bottom)` in world coordinates.
    pub fn bounding_box(&self) -> (f32, f32, f32, f32) {
        self.read(|collider| {
            let aabb = collider.compute_aabb();
            (aabb.mins.x, aabb.mins.y, aabb.maxs.x, aabb.maxs.y)
        })
    }

    /// A copy of the fixture's shape, measured from its body's origin.
    pub fn shape(&self) -> Shape {
        let shape = self.read(|collider| {
            Shape::new(
                collider.shared_shape().clone(),
                collider
                    .position_wrt_parent()
                    .copied()
                    .unwrap_or(Pose::IDENTITY),
            )
        });
        let ghosts = self.world.borrow().ghosts.get(&self.handle).copied();
        match ghosts {
            Some(ghosts) => shape.with_ghosts(ghosts),
            None => shape,
        }
    }

    /// Keeps the ghost vertices of `shape` for this fixture, or forgets
    /// them if it has none.
    pub(super) fn set_ghosts(&self, shape: &Shape) {
        let ghosts = shape.ghosts();
        let mut inner = self.world.borrow_mut();
        if ghosts == super::ghost::Ghosts::default() {
            inner.ghosts.remove(&self.handle);
        } else {
            inner.ghosts.insert(self.handle, ghosts);
        }
    }

    /// Gives the fixture a new shape, keeping everything else about it,
    /// and updates its body's mass. This is how a circle grows or moves:
    /// `fixture.set_shape(&fixture.shape().with_radius(20.0))`. LÖVE
    /// changes a fixture's shape in place with methods such as
    /// `CircleShape:setRadius`.
    pub fn set_shape(&self, shape: &Shape) -> &Fixture {
        let mut inner = self.world.borrow_mut();
        let world = &mut inner.world;
        let collider = world
            .colliders
            .get_mut(self.handle)
            .expect("this fixture was destroyed");
        collider.set_shape(shape.shape.clone());
        collider.set_position_wrt_parent(shape.offset);
        let parent = collider.parent();
        if let Some(body) = parent.and_then(|parent| world.bodies.get_mut(parent)) {
            body.recompute_mass_properties_from_colliders(&world.colliders);
            body.wake_up(true);
        }
        world
            .bodies
            .propagate_modified_body_positions_to_colliders(&mut world.colliders);
        drop(inner);
        self.set_ghosts(shape);
        self
    }

    /// Where the line from `(x1, y1)` to `(x2, y2)` first runs into this
    /// fixture, if it does. A line that starts inside it hits at once,
    /// with a fraction of 0. LÖVE's `Fixture:rayCast`, with the hit point
    /// worked out too.
    pub fn ray_cast(&self, anchor1: impl Into<Vec2>, anchor2: impl Into<Vec2>) -> Option<RayHit> {
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
        let direction = Vector::new(x2 - x1, y2 - y1);
        let ray = Ray::new(Vector::new(x1, y1), direction);
        let hit = self.read(|collider| {
            collider
                .shape()
                .cast_ray_and_get_normal(collider.position(), &ray, 1.0, true)
        })?;
        Some(RayHit {
            fixture: self.clone(),
            x: x1 + direction.x * hit.time_of_impact,
            y: y1 + direction.y * hit.time_of_impact,
            normal_x: hit.normal.x,
            normal_y: hit.normal.y,
            fraction: hit.time_of_impact,
        })
    }

    /// The fixture's share of its body's mass, as `(center_x, center_y,
    /// mass, inertia)`: its center of mass relative to the body's origin,
    /// its mass from its density and area, and its inertia around that
    /// center.
    pub fn mass_data(&self) -> (f32, f32, f32, f32) {
        let density = self.density();
        let meter = self.world.borrow().meter;
        self.read(|collider| {
            let props = collider
                .shape()
                .mass_properties(density / (meter * meter))
                .transform_by(
                    &collider
                        .position_wrt_parent()
                        .copied()
                        .unwrap_or(Pose::IDENTITY),
                );
            (
                props.local_com.x,
                props.local_com.y,
                props.mass(),
                props.principal_inertia(),
            )
        })
    }

    /// A number the game attached to the fixture, 0 until one is set.
    pub fn user_data(&self) -> u64 {
        self.read(|collider| collider.user_data as u64)
    }

    /// Attaches a number to the fixture, to recognize it in contacts.
    pub fn set_user_data(&self, data: u64) -> &Fixture {
        self.change(|collider| collider.user_data = u128::from(data));
        self
    }
}

impl std::fmt::Debug for Fixture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_destroyed() {
            return f.write_str("Fixture(destroyed)");
        }
        f.debug_struct("Fixture")
            .field("sensor", &self.is_sensor())
            .field("friction", &self.friction())
            .field("restitution", &self.restitution())
            .finish()
    }
}
