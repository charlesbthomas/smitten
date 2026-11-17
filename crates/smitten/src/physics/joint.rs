use std::rc::Rc;

use rapier2d::prelude::*;

use super::Body;
use super::world::{Inner, Shared};

/// What kind of connection a [`Joint`] is, the equivalent of LÖVE's
/// `JointType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JointType {
    /// Two bodies that turn around a shared point. See
    /// [`World::new_revolute_joint`](super::World::new_revolute_joint).
    Revolute,
    /// Two bodies glued together.
    Weld,
    /// Two points that can come closer but not get further apart.
    Rope,
    /// Two points held apart by a spring.
    Spring,
    /// Two points held at a fixed distance.
    Distance,
    /// A body that slides along a line relative to another.
    Prismatic,
    /// A wheel on a suspension, turning freely or driven by a motor.
    Wheel,
    /// A body pulled towards a target point, for dragging with the mouse.
    Mouse,
    /// Top-down friction between two bodies.
    Friction,
    /// A body driven towards an offset from another.
    Motor,
}

impl JointType {
    const ALL: [JointType; 10] = [
        JointType::Revolute,
        JointType::Weld,
        JointType::Rope,
        JointType::Spring,
        JointType::Distance,
        JointType::Prismatic,
        JointType::Wheel,
        JointType::Mouse,
        JointType::Friction,
        JointType::Motor,
    ];

    fn index(self) -> u128 {
        JointType::ALL
            .iter()
            .position(|&kind| kind == self)
            .expect("every joint type is listed") as u128
    }
}

/// The joint's type is kept in the engine's user data, above the 64 bits
/// the game can set.
const KIND_SHIFT: u32 = 64;
const GAME_DATA: u128 = u64::MAX as u128;

/// How hard a motor that drives a speed pushes towards it. The engine's
/// velocity motors are springs; this makes them stiff enough to act like
/// Box2D's, with the motor's maximum torque or force as the real limit.
pub(super) const MOTOR_DAMPING: f32 = 1.0e4;

pub(super) fn tag(joint: &mut GenericJoint, kind: JointType) {
    joint.user_data = (joint.user_data & GAME_DATA) | (kind.index() << KIND_SHIFT);
}

/// Spring coefficients for a frequency in hertz and a damping ratio, for
/// a motor whose stiffness is an acceleration.
pub(super) fn spring(frequency: f32, damping_ratio: f32) -> (f32, f32) {
    let omega = 2.0 * std::f32::consts::PI * frequency.max(0.0);
    (omega * omega, 2.0 * damping_ratio.max(0.0) * omega)
}

/// The frequency and damping ratio of a motor's spring.
fn unspring(motor: &JointMotor) -> (f32, f32) {
    let omega = motor.stiffness.max(0.0).sqrt();
    if omega <= 0.0 {
        return (0.0, 0.0);
    }
    (
        omega / (2.0 * std::f32::consts::PI),
        motor.damping / (2.0 * omega),
    )
}

/// How stiff a motor joint's correction is for a correction factor, as
/// the fraction of the error Box2D removes each step at 60 steps a second.
pub(super) fn correction_spring(factor: f32) -> (f32, f32) {
    let omega = 60.0 * factor.clamp(0.0, 1.0);
    (omega * omega, 2.0 * omega)
}

/// Turns a joint impulse the engine kept into a force. The engine splits
/// each update into substeps and keeps the impulse of the last one, as
/// applied to the first body.
fn per_second(inner: &Inner) -> f32 {
    let parameters = &inner.world.integration_parameters;
    parameters.num_solver_iterations.max(1) as f32 / parameters.dt
}

/// A connection between two bodies, the equivalent of LÖVE's `Joint`.
///
/// Make one with the `new_..._joint` methods of
/// [`World`](super::World). It stays in place until destroyed, or until
/// one of its bodies is.
///
/// One type stands for every kind of joint, as in LÖVE's dynamic API.
/// Methods that only some kinds have, such as
/// [`Joint::set_motor_speed`], say which kinds in their documentation and
/// panic when called on another kind. [`Joint::kind`] says which kind a
/// joint is. Using a destroyed joint panics, except for
/// [`Joint::is_destroyed`] and [`Joint::destroy`].
#[derive(Clone)]
pub struct Joint {
    world: Shared,
    handle: ImpulseJointHandle,
}

impl PartialEq for Joint {
    fn eq(&self, other: &Joint) -> bool {
        self.handle == other.handle && Rc::ptr_eq(&self.world, &other.world)
    }
}

impl Eq for Joint {}

impl std::hash::Hash for Joint {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.handle.hash(state);
        Rc::as_ptr(&self.world).hash(state);
    }
}

/// The joint's anchors and axis in the world, as the solver sees them.
struct Frames {
    /// The first body's frame and the second's.
    one: Pose,
    two: Pose,
}

impl Joint {
    pub(super) fn new(world: &Shared, handle: ImpulseJointHandle) -> Joint {
        Joint {
            world: world.clone(),
            handle,
        }
    }

    fn read<R>(&self, read: impl FnOnce(&ImpulseJoint, &Inner) -> R) -> R {
        let inner = self.world.borrow();
        let joint = inner
            .world
            .impulse_joints
            .get(self.handle)
            .expect("this joint was destroyed");
        read(joint, &inner)
    }

    /// Changes the joint and wakes its bodies, so the change is felt at
    /// once.
    fn change<R>(&self, change: impl FnOnce(&mut GenericJoint) -> R) -> R {
        let mut inner = self.world.borrow_mut();
        let joint = inner
            .world
            .impulse_joints
            .get_mut(self.handle, true)
            .expect("this joint was destroyed");
        change(&mut joint.data)
    }

    /// Panics unless the joint is one of `kinds`; returns its kind.
    fn expect(&self, kinds: &[JointType], what: &str) -> JointType {
        let kind = self.kind();
        assert!(
            kinds.contains(&kind),
            "a {kind:?} joint has no {what}; only {kinds:?} joints do"
        );
        kind
    }

    fn frames(joint: &ImpulseJoint, inner: &Inner) -> Frames {
        let pose = |body| {
            inner
                .world
                .bodies
                .get(body)
                .map_or(Pose::IDENTITY, |body: &RigidBody| *body.position())
        };
        Frames {
            one: pose(joint.body1()) * joint.data.local_frame1,
            two: pose(joint.body2()) * joint.data.local_frame2,
        }
    }

    /// Whether the joint, or one of its bodies, has been destroyed.
    pub fn is_destroyed(&self) -> bool {
        self.world
            .borrow()
            .world
            .impulse_joints
            .get(self.handle)
            .is_none()
    }

    /// Removes the joint, setting its bodies free of each other. Does
    /// nothing if it was already destroyed.
    pub fn destroy(&self) {
        self.world
            .borrow_mut()
            .world
            .remove_impulse_joint(self.handle);
    }

    /// What kind of joint this is.
    pub fn kind(&self) -> JointType {
        self.read(|joint, _| {
            let index = (joint.data.user_data >> KIND_SHIFT) as usize;
            JointType::ALL
                .get(index)
                .copied()
                .unwrap_or(JointType::Weld)
        })
    }

    /// The bodies the joint connects. A mouse joint has only the one it
    /// drags.
    pub fn bodies(&self) -> Vec<Body> {
        self.read(|joint, inner| {
            [joint.body1(), joint.body2()]
                .into_iter()
                .filter(|&body| Some(body) != inner.ground)
                .map(|body| Body::new(&self.world, body))
                .collect()
        })
    }

    /// Where the joint holds on to each body, in world coordinates, as
    /// `((x1, y1), (x2, y2))`. For a mouse joint the first is the target
    /// and the second the point being dragged.
    pub fn anchors(&self) -> ((f32, f32), (f32, f32)) {
        self.read(|joint, inner| {
            let frames = Joint::frames(joint, inner);
            let (one, two) = (frames.one.translation, frames.two.translation);
            ((one.x, one.y), (two.x, two.y))
        })
    }

    /// Whether the two bodies still collide with each other.
    pub fn collide_connected(&self) -> bool {
        self.read(|joint, _| joint.data.contacts_enabled)
    }

    /// The force the joint applied to its second body during the last
    /// update, as `(x, y)`. Useful for breaking a joint that is pulled
    /// too hard.
    pub fn reaction_force(&self) -> (f32, f32) {
        self.read(|joint, inner| {
            let data = &joint.data;
            let along = |axis: usize| {
                joint.impulses[axis] + data.limits[axis].impulse + data.motors[axis].impulse
            };
            let frames = Joint::frames(joint, inner);
            let impulse = if data.coupled_axes.contains(JointAxesMask::LIN_AXES) {
                // A rope or spring pulls along the line between its
                // anchors.
                let line = frames.two.translation - frames.one.translation;
                let length = line.length();
                if length > 1e-6 {
                    line / length * along(0)
                } else {
                    Vector::ZERO
                }
            } else {
                frames
                    .one
                    .rotation
                    .transform_vector(Vector::new(along(0), along(1)))
            };
            let force = -impulse * per_second(inner);
            (force.x, force.y)
        })
    }

    /// The torque the joint applied to its second body during the last
    /// update.
    pub fn reaction_torque(&self) -> f32 {
        self.read(|joint, inner| {
            let data = &joint.data;
            let axis = JointAxis::AngX as usize;
            let impulse =
                joint.impulses[axis] + data.limits[axis].impulse + data.motors[axis].impulse;
            -impulse * per_second(inner)
        })
    }

    /// A number the game attached to the joint, 0 until one is set.
    pub fn user_data(&self) -> u64 {
        self.read(|joint, _| (joint.data.user_data & GAME_DATA) as u64)
    }

    /// Attaches a number to the joint.
    pub fn set_user_data(&self, data: u64) {
        self.change(|joint| joint.user_data = (joint.user_data & !GAME_DATA) | u128::from(data));
    }

    // Motors.

    fn motor_axis(&self, what: &str) -> JointAxis {
        match self.expect(
            &[JointType::Revolute, JointType::Prismatic, JointType::Wheel],
            what,
        ) {
            JointType::Prismatic => JointAxis::LinX,
            _ => JointAxis::AngX,
        }
    }

    /// Whether the joint's motor is on. Revolute, prismatic and wheel
    /// joints have one, off at first.
    pub fn is_motor_enabled(&self) -> bool {
        let axis = self.motor_axis("motor");
        self.read(|joint, _| joint.data.motor_axes.contains(axis.into()))
    }

    /// Turns the joint's motor on or off. A motor turns the joint at
    /// [`Joint::motor_speed`], with no more than its maximum torque (or
    /// force, for a prismatic joint), which is 0 until set.
    pub fn set_motor_enabled(&self, enabled: bool) {
        let axis = self.motor_axis("motor");
        self.change(|joint| joint.motor_axes.set(axis.into(), enabled));
    }

    /// The speed the motor drives the joint at: radians per second for a
    /// revolute or wheel joint, units per second for a prismatic one.
    pub fn motor_speed(&self) -> f32 {
        let axis = self.motor_axis("motor");
        self.read(|joint, _| joint.data.motors[axis as usize].target_vel)
    }

    /// Sets the speed the motor drives the joint at. Positive turns the
    /// second body clockwise on screen, or slides it along the axis.
    pub fn set_motor_speed(&self, speed: f32) {
        let axis = self.motor_axis("motor");
        self.change(|joint| joint.motors[axis as usize].target_vel = speed);
    }

    /// The most torque the motor of a revolute or wheel joint can apply.
    pub fn max_motor_torque(&self) -> f32 {
        self.expect(&[JointType::Revolute, JointType::Wheel], "motor torque");
        self.read(|joint, _| joint.data.motors[JointAxis::AngX as usize].max_force)
    }

    /// Limits the torque the motor of a revolute or wheel joint can
    /// apply. The motor does nothing until this is above 0.
    pub fn set_max_motor_torque(&self, torque: f32) {
        self.expect(&[JointType::Revolute, JointType::Wheel], "motor torque");
        self.change(|joint| joint.motors[JointAxis::AngX as usize].max_force = torque.max(0.0));
    }

    /// The most force the motor of a prismatic joint can apply.
    pub fn max_motor_force(&self) -> f32 {
        self.expect(&[JointType::Prismatic], "motor force");
        self.read(|joint, _| joint.data.motors[JointAxis::LinX as usize].max_force)
    }

    /// Limits the force the motor of a prismatic joint can apply. The
    /// motor does nothing until this is above 0.
    pub fn set_max_motor_force(&self, force: f32) {
        self.expect(&[JointType::Prismatic], "motor force");
        self.change(|joint| joint.motors[JointAxis::LinX as usize].max_force = force.max(0.0));
    }

    /// The torque a revolute or wheel joint's motor applied during the
    /// last update.
    pub fn motor_torque(&self) -> f32 {
        self.expect(&[JointType::Revolute, JointType::Wheel], "motor torque");
        self.motor_output(JointAxis::AngX)
    }

    /// The force a prismatic joint's motor applied during the last
    /// update.
    pub fn motor_force(&self) -> f32 {
        self.expect(&[JointType::Prismatic], "motor force");
        self.motor_output(JointAxis::LinX)
    }

    fn motor_output(&self, axis: JointAxis) -> f32 {
        self.read(|joint, inner| {
            if !joint.data.motor_axes.contains(axis.into()) {
                return 0.0;
            }
            -joint.data.motors[axis as usize].impulse * per_second(inner)
        })
    }

    // Limits.

    fn limit_axis(&self) -> JointAxis {
        match self.expect(&[JointType::Revolute, JointType::Prismatic], "limits") {
            JointType::Prismatic => JointAxis::LinX,
            _ => JointAxis::AngX,
        }
    }

    /// Whether the joint's limits are on. Revolute and prismatic joints
    /// have them, off at first.
    pub fn are_limits_enabled(&self) -> bool {
        let axis = self.limit_axis();
        self.read(|joint, _| joint.data.limit_axes.contains(axis.into()))
    }

    /// Turns the joint's limits on or off.
    pub fn set_limits_enabled(&self, enabled: bool) {
        let axis = self.limit_axis();
        self.change(|joint| joint.limit_axes.set(axis.into(), enabled));
    }

    /// The joint's `(lower, upper)` limits: the angles, in radians, a
    /// revolute joint may turn to, or the distances along its axis a
    /// prismatic joint may slide to. Both start at 0.
    pub fn limits(&self) -> (f32, f32) {
        let axis = self.limit_axis();
        self.read(|joint, _| {
            let limits = &joint.data.limits[axis as usize];
            (limits.min, limits.max)
        })
    }

    /// Sets the joint's limits. They are measured from where the joint
    /// was made, and take effect once
    /// [enabled](Joint::set_limits_enabled).
    pub fn set_limits(&self, lower: f32, upper: f32) {
        assert!(
            lower <= upper,
            "a joint's lower limit must not be above its upper one, got {lower} and {upper}"
        );
        let axis = self.limit_axis();
        self.change(|joint| {
            let limits = &mut joint.limits[axis as usize];
            limits.min = lower;
            limits.max = upper;
        });
    }

    // Where the joint is.

    /// How far a revolute or wheel joint has turned since it was made, in
    /// radians, clockwise on screen.
    pub fn joint_angle(&self) -> f32 {
        self.expect(&[JointType::Revolute, JointType::Wheel], "angle");
        self.read(|joint, inner| {
            let frames = Joint::frames(joint, inner);
            (frames.one.rotation.inverse() * frames.two.rotation).angle()
        })
    }

    /// How far a prismatic or wheel joint has slid along its axis since
    /// it was made.
    pub fn joint_translation(&self) -> f32 {
        self.expect(&[JointType::Prismatic, JointType::Wheel], "translation");
        self.read(|joint, inner| {
            let frames = Joint::frames(joint, inner);
            let axis = frames.one.rotation.transform_vector(Vector::X);
            (frames.two.translation - frames.one.translation).dot(axis)
        })
    }

    /// How fast the joint is moving: in radians per second for a revolute
    /// or wheel joint, in units per second along the axis for a prismatic
    /// one.
    pub fn joint_speed(&self) -> f32 {
        let kind = self.expect(
            &[JointType::Revolute, JointType::Prismatic, JointType::Wheel],
            "speed",
        );
        self.read(|joint, inner| {
            let body = |handle| inner.world.bodies.get(handle);
            let (Some(one), Some(two)) = (body(joint.body1()), body(joint.body2())) else {
                return 0.0;
            };
            if kind != JointType::Prismatic {
                return two.angvel() - one.angvel();
            }
            let frames = Joint::frames(joint, inner);
            let axis = frames.one.rotation.transform_vector(Vector::X);
            let cross = |spin: f32, arm: Vector| Vector::new(-arm.y, arm.x) * spin;
            let at = |body: &RigidBody, point: Vector| {
                body.linvel() + cross(body.angvel(), point - body.center_of_mass())
            };
            let apart = frames.two.translation - frames.one.translation;
            // As Box2D works it out: the anchors' relative velocity along
            // the axis, plus how the axis itself turns with the first body.
            apart.dot(cross(one.angvel(), axis))
                + axis.dot(at(two, frames.two.translation) - at(one, frames.one.translation))
        })
    }

    // Ropes, distances and springs.

    /// The longest a rope joint can stretch.
    pub fn max_length(&self) -> f32 {
        self.expect(&[JointType::Rope], "maximum length");
        self.read(|joint, _| joint.data.limits[JointAxis::LinX as usize].max)
    }

    /// Changes the longest a rope joint can stretch.
    pub fn set_max_length(&self, length: f32) {
        self.expect(&[JointType::Rope], "maximum length");
        self.change(|joint| {
            let limits = &mut joint.limits[JointAxis::LinX as usize];
            limits.min = 0.0;
            limits.max = length.max(0.0);
        });
    }

    /// The distance a distance joint holds its anchors at, or that a
    /// spring joint's spring rests at.
    pub fn length(&self) -> f32 {
        self.expect(&[JointType::Distance, JointType::Spring], "length");
        self.read(|joint, _| joint.data.motors[JointAxis::LinX as usize].target_pos)
    }

    /// Changes the length of a distance or spring joint.
    pub fn set_length(&self, length: f32) {
        let kind = self.expect(&[JointType::Distance, JointType::Spring], "length");
        let length = length.max(0.0);
        self.change(|joint| {
            let axis = JointAxis::LinX as usize;
            match kind {
                JointType::Distance => {
                    joint.limits[axis].max = length;
                    joint.motors[axis].target_pos = length;
                }
                _ => joint.motors[axis].target_pos = length,
            }
        });
    }

    /// How hard a spring joint pulls back.
    pub fn stiffness(&self) -> f32 {
        self.expect(&[JointType::Spring], "stiffness");
        self.read(|joint, _| joint.data.motors[JointAxis::LinX as usize].stiffness)
    }

    /// Changes how hard a spring joint pulls back.
    pub fn set_stiffness(&self, stiffness: f32) {
        self.expect(&[JointType::Spring], "stiffness");
        self.change(|joint| joint.motors[JointAxis::LinX as usize].stiffness = stiffness.max(0.0));
    }

    /// How quickly a spring joint's bouncing dies away.
    pub fn damping(&self) -> f32 {
        self.expect(&[JointType::Spring], "damping");
        self.read(|joint, _| joint.data.motors[JointAxis::LinX as usize].damping)
    }

    /// Changes how quickly a spring joint's bouncing dies away.
    pub fn set_damping(&self, damping: f32) {
        self.expect(&[JointType::Spring], "damping");
        self.change(|joint| joint.motors[JointAxis::LinX as usize].damping = damping.max(0.0));
    }

    // Springy mouse and wheel joints.

    fn springs(&self) -> &'static [JointAxis] {
        match self.expect(&[JointType::Mouse, JointType::Wheel], "frequency") {
            JointType::Mouse => &[JointAxis::LinX, JointAxis::LinY],
            _ => &[JointAxis::LinX],
        }
    }

    /// How quickly a mouse joint pulls its body to the target, or a wheel
    /// joint's suspension bounces, in times a second.
    pub fn frequency(&self) -> f32 {
        let axis = self.springs()[0];
        self.read(|joint, _| unspring(&joint.data.motors[axis as usize]).0)
    }

    /// Changes how quickly a mouse joint pulls, or a wheel joint's
    /// suspension bounces. 5 for a mouse joint and 2 for a wheel at first.
    pub fn set_frequency(&self, frequency: f32) {
        let axes = self.springs();
        let ratio = self.damping_ratio();
        self.set_spring(axes, frequency, ratio);
    }

    /// How much a mouse joint's pull, or a wheel joint's suspension, is
    /// damped: 1.0 stops it overshooting at all, less lets it bounce.
    pub fn damping_ratio(&self) -> f32 {
        let axis = self.springs()[0];
        self.read(|joint, _| unspring(&joint.data.motors[axis as usize]).1)
    }

    /// Changes how much a mouse joint or wheel joint is damped. 0.7 at
    /// first.
    pub fn set_damping_ratio(&self, ratio: f32) {
        let axes = self.springs();
        let frequency = self.frequency();
        self.set_spring(axes, frequency, ratio);
    }

    fn set_spring(&self, axes: &[JointAxis], frequency: f32, ratio: f32) {
        let (stiffness, damping) = spring(frequency, ratio);
        self.change(|joint| {
            for &axis in axes {
                let motor = &mut joint.motors[axis as usize];
                motor.stiffness = stiffness;
                motor.damping = damping;
            }
        });
    }

    /// Where a mouse joint pulls its body to.
    pub fn target(&self) -> (f32, f32) {
        self.expect(&[JointType::Mouse], "target");
        self.anchors().0
    }

    /// Moves the point a mouse joint pulls its body to. Call it with the
    /// mouse position every frame while dragging.
    pub fn set_target(&self, x: f32, y: f32) {
        self.expect(&[JointType::Mouse], "target");
        // The target is held by the world's hidden ground body, which
        // sits at the origin.
        self.change(|joint| {
            joint.set_local_anchor1(Vector::new(x, y));
        });
    }

    // Mouse, friction and motor joints.

    fn force_axes(&self) -> &'static [JointAxis] {
        self.expect(
            &[JointType::Mouse, JointType::Friction, JointType::Motor],
            "maximum force",
        );
        &[JointAxis::LinX, JointAxis::LinY]
    }

    /// The most force a mouse, friction or motor joint applies.
    pub fn max_force(&self) -> f32 {
        let axis = self.force_axes()[0];
        self.read(|joint, _| joint.data.motors[axis as usize].max_force)
    }

    /// Limits the force a mouse, friction or motor joint applies. A mouse
    /// joint starts at a thousand times its body's weight in one meter's
    /// gravity, a friction joint at 0, and a motor joint unlimited.
    pub fn set_max_force(&self, force: f32) {
        let axes = self.force_axes();
        self.change(|joint| {
            for &axis in axes {
                joint.motors[axis as usize].max_force = force.max(0.0);
            }
        });
    }

    /// The most torque a friction or motor joint applies.
    pub fn max_torque(&self) -> f32 {
        self.expect(&[JointType::Friction, JointType::Motor], "maximum torque");
        self.read(|joint, _| joint.data.motors[JointAxis::AngX as usize].max_force)
    }

    /// Limits the torque a friction or motor joint applies. A friction
    /// joint starts at 0, and a motor joint unlimited.
    pub fn set_max_torque(&self, torque: f32) {
        self.expect(&[JointType::Friction, JointType::Motor], "maximum torque");
        self.change(|joint| joint.motors[JointAxis::AngX as usize].max_force = torque.max(0.0));
    }

    /// Where a motor joint holds its second body, relative to the first
    /// body's position and angle.
    pub fn linear_offset(&self) -> (f32, f32) {
        self.expect(&[JointType::Motor], "linear offset");
        self.read(|joint, _| {
            let offset = joint.data.local_frame1.translation;
            (offset.x, offset.y)
        })
    }

    /// Changes where a motor joint holds its second body.
    pub fn set_linear_offset(&self, x: f32, y: f32) {
        self.expect(&[JointType::Motor], "linear offset");
        self.change(|joint| joint.local_frame1.translation = Vector::new(x, y));
    }

    /// The angle a motor joint holds its second body at, relative to the
    /// first.
    pub fn angular_offset(&self) -> f32 {
        self.expect(&[JointType::Motor], "angular offset");
        self.read(|joint, _| joint.data.local_frame1.rotation.angle())
    }

    /// Changes the angle a motor joint holds its second body at.
    pub fn set_angular_offset(&self, angle: f32) {
        self.expect(&[JointType::Motor], "angular offset");
        self.change(|joint| joint.local_frame1.rotation = Rotation::new(angle));
    }

    /// How quickly a motor joint corrects its second body's position,
    /// from 0.0 to 1.0. 0.3 by default.
    pub fn correction_factor(&self) -> f32 {
        self.expect(&[JointType::Motor], "correction factor");
        self.read(|joint, _| {
            let stiffness = joint.data.motors[JointAxis::LinX as usize].stiffness;
            stiffness.max(0.0).sqrt() / 60.0
        })
    }

    /// Changes how quickly a motor joint corrects its second body's
    /// position.
    pub fn set_correction_factor(&self, factor: f32) {
        self.expect(&[JointType::Motor], "correction factor");
        let (stiffness, damping) = correction_spring(factor);
        self.change(|joint| {
            for motor in &mut joint.motors {
                motor.stiffness = stiffness;
                motor.damping = damping;
            }
        });
    }
}

impl std::fmt::Debug for Joint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_destroyed() {
            return f.write_str("Joint(destroyed)");
        }
        f.debug_struct("Joint").field("kind", &self.kind()).finish()
    }
}
