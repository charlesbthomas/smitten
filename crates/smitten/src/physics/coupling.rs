//! Gear and pulley joints, which Rapier has no constraint for.
//!
//! Each is held in the engine as a joint that locks nothing, so that it
//! has a handle, belongs to its two bodies, keeps their `collide` setting
//! and goes away with them. What it ties together is kept here and
//! enforced after each of the engine's steps: the bodies' velocities are
//! corrected so the tie holds, as Box2D's solver would, and then their
//! positions, so that it does not drift.

use rapier2d::prelude::*;

use super::joint::JointType;
use super::world::Inner;

/// What a gear or pulley joint ties together.
#[derive(Debug, Clone)]
pub(super) enum Coupling {
    /// The two bodies hang from ropes over two fixed points, and
    /// `length_a + ratio * length_b` stays `constant`.
    Pulley {
        ground_a: Vector,
        ground_b: Vector,
        ratio: f32,
        constant: f32,
        impulse: f32,
    },
    /// The two joints turn or slide together: `coordinate_a + ratio *
    /// coordinate_b` stays `constant`.
    Gear {
        joint_a: ImpulseJointHandle,
        joint_b: ImpulseJointHandle,
        ratio: f32,
        constant: f32,
        impulse: f32,
    },
}

/// How fast a coupling's value changes for each body involved: the rate
/// for its velocity and the rate for its spin.
type Rates = Vec<(RigidBodyHandle, Vector, f32)>;

fn cross(a: Vector, b: Vector) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Adds `scale` times a body's rates, merging with rates already listed
/// for the same body.
fn add(rates: &mut Rates, body: RigidBodyHandle, linear: Vector, angular: f32, scale: f32) {
    if let Some(entry) = rates.iter_mut().find(|entry| entry.0 == body) {
        entry.1 += linear * scale;
        entry.2 += angular * scale;
    } else {
        rates.push((body, linear * scale, angular * scale));
    }
}

fn pose(world: &PhysicsWorld, body: RigidBodyHandle) -> Pose {
    world
        .bodies
        .get(body)
        .map_or(Pose::IDENTITY, |body| *body.position())
}

fn center(world: &PhysicsWorld, body: RigidBodyHandle) -> Vector {
    world
        .bodies
        .get(body)
        .map_or(Vector::ZERO, |body| body.center_of_mass())
}

/// A revolute joint's angle or a prismatic joint's translation, and its
/// rates, added to `rates` scaled by `scale`.
pub(super) fn joint_coordinate(
    world: &PhysicsWorld,
    handle: ImpulseJointHandle,
    rates: &mut Rates,
    scale: f32,
) -> Option<f32> {
    let joint = world.impulse_joints.get(handle)?;
    let (body1, body2) = (joint.body1(), joint.body2());
    let one = pose(world, body1) * joint.data.local_frame1;
    let two = pose(world, body2) * joint.data.local_frame2;
    match super::joint::kind_of(&joint.data) {
        JointType::Revolute => {
            add(rates, body1, Vector::ZERO, -1.0, scale);
            add(rates, body2, Vector::ZERO, 1.0, scale);
            Some((one.rotation.inverse() * two.rotation).angle())
        }
        JointType::Prismatic => {
            let axis = one.rotation.transform_vector(Vector::X);
            let anchor = two.translation;
            add(
                rates,
                body1,
                -axis,
                -cross(anchor - center(world, body1), axis),
                scale,
            );
            add(
                rates,
                body2,
                axis,
                cross(anchor - center(world, body2), axis),
                scale,
            );
            Some((anchor - one.translation).dot(axis))
        }
        _ => None,
    }
}

/// The lengths of a pulley's two ropes.
pub(super) fn pulley_lengths(
    world: &PhysicsWorld,
    handle: ImpulseJointHandle,
    ground_a: Vector,
    ground_b: Vector,
) -> Option<(f32, f32)> {
    let joint = world.impulse_joints.get(handle)?;
    let a = pose(world, joint.body1()).transform_point(joint.data.local_anchor1());
    let b = pose(world, joint.body2()).transform_point(joint.data.local_anchor2());
    Some(((a - ground_a).length(), (b - ground_b).length()))
}

/// How far a coupling is from holding, and its rates.
fn measure(
    world: &PhysicsWorld,
    handle: ImpulseJointHandle,
    coupling: &Coupling,
) -> Option<(f32, Rates)> {
    let mut rates = Rates::new();
    match *coupling {
        Coupling::Pulley {
            ground_a,
            ground_b,
            ratio,
            constant,
            ..
        } => {
            let joint = world.impulse_joints.get(handle)?;
            let (body1, body2) = (joint.body1(), joint.body2());
            let a = pose(world, body1).transform_point(joint.data.local_anchor1());
            let b = pose(world, body2).transform_point(joint.data.local_anchor2());
            let unit = |rope: Vector| {
                let length = rope.length();
                (
                    if length > 1e-6 {
                        rope / length
                    } else {
                        Vector::ZERO
                    },
                    length,
                )
            };
            let (along_a, length_a) = unit(a - ground_a);
            let (along_b, length_b) = unit(b - ground_b);
            add(
                &mut rates,
                body1,
                along_a,
                cross(a - center(world, body1), along_a),
                1.0,
            );
            add(
                &mut rates,
                body2,
                along_b,
                cross(b - center(world, body2), along_b),
                ratio,
            );
            Some((length_a + ratio * length_b - constant, rates))
        }
        Coupling::Gear {
            joint_a,
            joint_b,
            ratio,
            constant,
            ..
        } => {
            let a = joint_coordinate(world, joint_a, &mut rates, 1.0)?;
            let b = joint_coordinate(world, joint_b, &mut rates, ratio)?;
            Some((a + ratio * b - constant, rates))
        }
    }
}

/// How much a unit of correction changes the coupling: one over its
/// effective mass.
fn softness(world: &PhysicsWorld, rates: &Rates) -> f32 {
    rates
        .iter()
        .filter_map(|&(handle, linear, angular)| {
            let body = world.bodies.get(handle)?;
            let props = body.mass_properties();
            Some(
                linear.dot(linear * props.effective_inv_mass)
                    + angular * angular * props.effective_world_inv_inertia,
            )
        })
        .sum()
}

/// How many times the positions are corrected after each step. The
/// pulley's rope directions change as the bodies move, so one is not
/// quite enough.
const POSITION_PASSES: usize = 3;

/// Makes every gear and pulley joint hold, after a step.
pub(super) fn solve(inner: &mut Inner) {
    if inner.couplings.is_empty() {
        return;
    }
    let world = &mut inner.world;
    inner
        .couplings
        .retain(|&handle, _| world.impulse_joints.get(handle).is_some());
    let handles: Vec<ImpulseJointHandle> = inner.couplings.keys().copied().collect();
    let mut moved = false;
    for handle in handles {
        let coupling = &inner.couplings[&handle];
        let Some((_, rates)) = measure(world, handle, coupling) else {
            continue;
        };
        let enabled = rates
            .iter()
            .all(|&(body, _, _)| world.bodies.get(body).is_some_and(|body| body.is_enabled()));
        let soft = softness(world, &rates);
        if !enabled || soft <= 1e-12 {
            continue;
        }

        // Velocities first: stop the coupling changing.
        let speed: f32 = rates
            .iter()
            .filter_map(|&(body, linear, angular)| {
                let body = world.bodies.get(body)?;
                Some(linear.dot(body.linvel()) + angular * body.angvel())
            })
            .sum();
        let impulse = -speed / soft;
        if impulse.abs() > 0.0 {
            for &(handle, linear, angular) in &rates {
                let Some(body) = world.bodies.get_mut(handle) else {
                    continue;
                };
                let props = body.mass_properties();
                let (inv_mass, inv_inertia) =
                    (props.effective_inv_mass, props.effective_world_inv_inertia);
                if inv_mass == Vector::ZERO && inv_inertia == 0.0 {
                    continue;
                }
                let velocity = body.linvel() + linear * inv_mass * impulse;
                let spin = body.angvel() + angular * inv_inertia * impulse;
                body.set_linvel(velocity, true);
                body.set_angvel(spin, true);
            }
        }
        if let Some(Coupling::Pulley { impulse: kept, .. } | Coupling::Gear { impulse: kept, .. }) =
            inner.couplings.get_mut(&handle)
        {
            *kept = impulse;
        }

        // Then positions: undo any drift.
        for _ in 0..POSITION_PASSES {
            let coupling = &inner.couplings[&handle];
            let Some((error, rates)) = measure(world, handle, coupling) else {
                break;
            };
            let soft = softness(world, &rates);
            if error.abs() < 1e-5 || soft <= 1e-12 {
                break;
            }
            let step = -error / soft;
            for &(handle, linear, angular) in &rates {
                let Some(body) = world.bodies.get_mut(handle) else {
                    continue;
                };
                let props = body.mass_properties();
                let (inv_mass, inv_inertia) =
                    (props.effective_inv_mass, props.effective_world_inv_inertia);
                if inv_mass == Vector::ZERO && inv_inertia == 0.0 {
                    continue;
                }
                let center = body.center_of_mass() + linear * inv_mass * step;
                let angle = body.rotation().angle() + angular * inv_inertia * step;
                let origin =
                    center - Rotation::new(angle).transform_vector(body.local_center_of_mass());
                body.set_position(Pose::new(origin, angle), true);
                // Keep the center of mass up to date for the next pass.
                body.recompute_mass_properties_from_colliders(&world.colliders);
                moved = true;
            }
        }
    }
    if moved {
        world
            .bodies
            .propagate_modified_body_positions_to_colliders(&mut world.colliders);
    }
}
