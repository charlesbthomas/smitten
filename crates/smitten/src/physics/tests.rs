use super::*;
use glam::Vec2;

const STEP: f32 = 1.0 / 60.0;

fn run(world: &World, seconds: f32) {
    for _ in 0..(seconds / STEP).round() as usize {
        world.update(STEP);
    }
}

/// A world with a wide floor whose top is at y = 500.
fn world_with_floor() -> (World, Body) {
    let world = World::new((0.0, 600.0));
    let floor = world.new_body((400.0, 520.0), BodyType::Static);
    floor.add_rectangle((2000.0, 40.0));
    (world, floor)
}

fn distance(a: impl Into<Vec2>, b: impl Into<Vec2>) -> f32 {
    a.into().distance(b.into())
}

#[test]
fn a_body_falls_and_comes_to_rest_on_the_floor() {
    let (world, floor) = world_with_floor();
    let crate_ = world.new_body((400.0, 100.0), BodyType::Dynamic);
    crate_.add_rectangle((40.0, 40.0));

    run(&world, 0.25);
    assert!(crate_.y() > 100.0 && crate_.linear_velocity().y > 0.0);

    run(&world, 4.0);
    // Its center rests half its height above the floor's top.
    assert!((crate_.y() - 480.0).abs() < 2.0, "rested at {}", crate_.y());
    assert!(crate_.linear_velocity().y.abs() < 1.0);
    assert!(crate_.is_touching(&floor));
    assert_eq!(floor.position(), Vec2::new(400.0, 520.0));
}

#[test]
fn gravity_matches_the_numbers_given() {
    let world = World::new((0.0, 600.0));
    let body = world.new_body((0.0, 0.0), BodyType::Dynamic);
    body.add_circle(10.0);
    run(&world, 1.0);
    // After a second of free fall: speed g, distance about g / 2.
    assert!((body.linear_velocity().y - 600.0).abs() < 1.0);
    assert!((body.y() - 300.0).abs() < 10.0, "fell {}", body.y());

    world.set_gravity((0.0, 0.0));
    assert_eq!(world.gravity(), Vec2::new(0.0, 0.0));
}

#[test]
fn restitution_makes_things_bounce() {
    let bounce_height = |restitution: f32| {
        let (world, _floor) = world_with_floor();
        let ball = world.new_body((400.0, 300.0), BodyType::Dynamic);
        ball.add_circle(10.0).set_restitution(restitution);
        let mut highest_after_landing = f32::MAX;
        let mut landed = false;
        for _ in 0..240 {
            world.update(STEP);
            landed |= ball.y() > 485.0;
            if landed {
                highest_after_landing = highest_after_landing.min(ball.y());
            }
        }
        490.0 - highest_after_landing
    };
    assert!(bounce_height(0.0) < 5.0);
    assert!(bounce_height(0.8) > 60.0);
}

#[test]
fn contacts_report_touching_starting_and_stopping() {
    let (world, floor) = world_with_floor();
    let ball = world.new_body((400.0, 440.0), BodyType::Dynamic);
    let shape = ball.add_circle(10.0);

    let mut began = Vec::new();
    for _ in 0..60 {
        world.update(STEP);
        began.extend(world.contacts().into_iter().filter(|contact| contact.began));
    }
    assert_eq!(began.len(), 1);
    assert!(began[0].involves(&ball) && began[0].involves(&floor));
    assert!(began[0].a == shape || began[0].b == shape);

    // Lifting it away ends the contact.
    ball.set_position((400.0, 100.0));
    ball.set_linear_velocity((0.0, 0.0));
    world.update(STEP);
    let ended: Vec<_> = world.contacts();
    assert!(
        ended
            .iter()
            .any(|contact| !contact.began && contact.involves(&ball))
    );
    assert!(!ball.is_touching(&floor));
}

#[test]
fn sensors_detect_without_blocking() {
    let world = World::new((0.0, 600.0));
    let zone = world.new_body((0.0, 200.0), BodyType::Static);
    zone.add_rectangle((200.0, 20.0)).set_sensor(true);
    let ball = world.new_body((0.0, 0.0), BodyType::Dynamic);
    ball.add_circle(10.0);

    let mut entered = false;
    for _ in 0..120 {
        world.update(STEP);
        entered |= world
            .contacts()
            .iter()
            .any(|contact| contact.began && contact.involves(&zone));
    }
    assert!(entered);
    // It fell straight through.
    assert!(ball.y() > 400.0);
}

#[test]
fn filters_decide_what_collides() {
    let (world, floor) = world_with_floor();
    floor.fixtures()[0].set_filter(0b01, 0b01);
    let ghost = world.new_body((300.0, 400.0), BodyType::Dynamic);
    ghost.add_circle(10.0).set_filter(0b10, 0b10);
    let solid = world.new_body((500.0, 400.0), BodyType::Dynamic);
    let shape = solid.add_circle(10.0);
    shape.set_filter(0b01, 0b11);
    assert_eq!(shape.filter(), (0b01, 0b11));

    run(&world, 2.0);
    assert!(ghost.y() > 600.0, "the ghost should fall through");
    assert!(
        (solid.y() - 490.0).abs() < 2.0,
        "the solid ball should land"
    );
}

#[test]
fn ray_casts_find_the_nearest_fixture() {
    let world = World::new((0.0, 0.0));
    let near = world.new_body((100.0, 0.0), BodyType::Static);
    let near_shape = near.add_rectangle((20.0, 20.0));
    let far = world.new_body((200.0, 0.0), BodyType::Static);
    far.add_rectangle((20.0, 20.0));
    world.update(STEP);

    let hit = world
        .ray_cast((0.0, 0.0), (400.0, 0.0))
        .expect("the ray hits");
    assert_eq!(hit.fixture, near_shape);
    assert!((hit.point.x - 90.0).abs() < 0.01 && hit.point.y.abs() < 0.01);
    assert!((hit.normal.x + 1.0).abs() < 0.01);
    assert!((hit.fraction - 0.225).abs() < 0.001);

    assert!(world.ray_cast((0.0, 100.0), (400.0, 100.0)).is_none());
    assert!(world.ray_cast((0.0, 0.0), (50.0, 0.0)).is_none());
}

#[test]
fn points_can_be_tested_against_fixtures() {
    let world = World::new((0.0, 0.0));
    let body = world.new_body((100.0, 100.0), BodyType::Static);
    let shape = body.add_circle(20.0);
    world.update(STEP);

    assert_eq!(
        world.fixtures_at((110.0, 100.0)),
        std::slice::from_ref(&shape)
    );
    assert!(world.fixtures_at((130.0, 100.0)).is_empty());
    assert!(shape.test_point((100.0, 115.0)));
    assert!(!shape.test_point((100.0, 125.0)));
}

#[test]
fn mass_comes_from_area_density_and_the_meter() {
    let world = World::new((0.0, 0.0));
    assert_eq!(world.meter(), 30.0);
    let body = world.new_body((0.0, 0.0), BodyType::Dynamic);
    // Two meters by one meter at the default density of 1.
    let shape = body.add_rectangle((60.0, 30.0));
    assert!((body.mass() - 2.0).abs() < 1e-3, "mass was {}", body.mass());
    assert!((shape.density() - 1.0).abs() < 1e-4);

    shape.set_density(3.0);
    assert!((body.mass() - 6.0).abs() < 1e-3);
    assert_eq!(shape.friction(), 0.2);
    assert_eq!(shape.restitution(), 0.0);
}

#[test]
fn impulses_change_velocity_by_impulse_over_mass() {
    let world = World::new((0.0, 0.0));
    let body = world.new_body((0.0, 0.0), BodyType::Dynamic);
    body.add_rectangle((60.0, 30.0));
    body.apply_linear_impulse((100.0, 0.0));
    world.update(STEP);
    assert!((body.linear_velocity().x - 50.0).abs() < 0.5);
}

#[test]
fn a_force_lasts_for_one_update_only() {
    let world = World::new((0.0, 0.0));
    let pushed = world.new_body((0.0, 0.0), BodyType::Dynamic);
    pushed.add_rectangle((30.0, 30.0));
    // Out of the way of the first, which will move along x.
    let twisted = world.new_body((0.0, 500.0), BodyType::Dynamic);
    twisted.add_rectangle((30.0, 30.0));

    // Hard enough that neither is slow enough to be put to sleep.
    pushed.apply_force((20_000.0, 0.0));
    twisted.apply_torque(200_000.0);
    world.update(STEP);
    let speed = pushed.linear_velocity().x;
    let spin = twisted.angular_velocity();
    assert!(speed > 0.0 && spin > 0.0);

    // With nothing pushing any more, both carry on as they were.
    run(&world, 0.5);
    assert!((pushed.linear_velocity().x - speed).abs() < speed * 1e-3);
    assert!((twisted.angular_velocity() - spin).abs() < spin * 1e-3);

    // A force applied every update is a steady push.
    for _ in 0..10 {
        pushed.apply_force((20_000.0, 0.0));
        world.update(STEP);
    }
    assert!((pushed.linear_velocity().x - speed * 11.0).abs() < speed * 0.01);
}

#[test]
fn a_kinematic_body_moves_at_its_velocity_and_carries_things() {
    let world = World::new((0.0, 600.0));
    let platform = world.new_body((0.0, 300.0), BodyType::Kinematic);
    platform.add_rectangle((200.0, 20.0)).set_friction(1.0);
    platform.set_linear_velocity((60.0, 0.0));
    let rider = world.new_body((0.0, 270.0), BodyType::Dynamic);
    rider.add_rectangle((20.0, 20.0)).set_friction(1.0);

    run(&world, 2.0);
    assert!((platform.x() - 120.0).abs() < 1.0);
    assert!((platform.y() - 300.0).abs() < 0.01);
    assert!(rider.x() > 80.0, "the rider only got to {}", rider.x());
}

#[test]
fn fixed_rotation_keeps_a_body_upright() {
    let world = World::new((0.0, 0.0));
    let body = world.new_body((0.0, 0.0), BodyType::Dynamic);
    body.add_rectangle((20.0, 40.0));
    body.set_fixed_rotation(true);
    assert!(body.is_fixed_rotation());
    body.apply_angular_impulse(1000.0);
    run(&world, 0.5);
    assert!(body.angle().abs() < 1e-4);

    body.set_angle(1.0);
    assert!((body.angle() - 1.0).abs() < 1e-5);
}

#[test]
fn a_revolute_joint_makes_a_pendulum() {
    let world = World::new((0.0, 600.0));
    let pivot = world.new_body((0.0, 0.0), BodyType::Static);
    let bob = world.new_body((100.0, 0.0), BodyType::Dynamic);
    bob.add_circle(10.0);
    world.new_revolute_joint(&pivot, &bob, (0.0, 0.0), false);

    let mut lowest: f32 = 0.0;
    for _ in 0..120 {
        world.update(STEP);
        lowest = lowest.max(bob.y());
        let length = distance(bob.position(), (0.0, 0.0));
        assert!(
            (length - 100.0).abs() < 3.0,
            "the arm stretched to {length}"
        );
    }
    assert!(lowest > 90.0, "it only swung down to {lowest}");
}

#[test]
fn a_weld_joint_moves_two_bodies_as_one() {
    let world = World::new((0.0, 0.0));
    let a = world.new_body((0.0, 0.0), BodyType::Dynamic);
    a.add_rectangle((20.0, 20.0));
    let b = world.new_body((30.0, 0.0), BodyType::Dynamic);
    b.add_rectangle((20.0, 20.0));
    world.new_weld_joint(&a, &b, false);
    a.set_angular_velocity(2.0);
    a.set_linear_velocity((50.0, 0.0));

    run(&world, 1.0);
    assert!((distance(a.position(), b.position()) - 30.0).abs() < 1.0);
    assert!((a.angle() - b.angle()).abs() < 0.05);
    assert!(b.x() > 30.0);
}

#[test]
fn a_rope_joint_limits_distance_and_can_be_destroyed() {
    let world = World::new((0.0, 600.0));
    let hook = world.new_body((0.0, 0.0), BodyType::Static);
    let weight = world.new_body((0.0, 10.0), BodyType::Dynamic);
    weight.add_circle(5.0);
    let rope = world.new_rope_joint(&hook, &weight, (0.0, 0.0), (0.0, 10.0), 80.0, false);

    run(&world, 3.0);
    assert!((weight.y() - 80.0).abs() < 3.0, "hung at {}", weight.y());

    rope.destroy();
    assert!(rope.is_destroyed());
    run(&world, 1.0);
    assert!(weight.y() > 200.0);
}

#[test]
fn a_spring_joint_pulls_back_toward_its_rest_length() {
    let world = World::new((0.0, 0.0));
    let anchor = world.new_body((0.0, 0.0), BodyType::Static);
    let weight = world.new_body((100.0, 0.0), BodyType::Dynamic);
    weight.add_circle(10.0);
    world.new_spring_joint(&anchor, &weight, (0.0, 0.0), (100.0, 0.0), 50.0, 5.0);

    weight.set_position((160.0, 0.0));
    run(&world, 5.0);
    assert!(
        (weight.x() - 100.0).abs() < 5.0,
        "settled at {}",
        weight.x()
    );
}

#[test]
fn a_prismatic_joint_only_lets_a_body_slide_along_its_axis() {
    let world = World::new((0.0, 600.0));
    let rail = world.new_body((0.0, 0.0), BodyType::Static);
    let slider = world.new_body((0.0, 0.0), BodyType::Dynamic);
    slider.add_rectangle((20.0, 20.0));
    // Slides sideways only, so gravity cannot move it.
    world.new_prismatic_joint(&rail, &slider, (0.0, 0.0), (1.0, 0.0), false);
    slider.apply_linear_impulse((60.0 * slider.mass(), 0.0));

    run(&world, 1.0);
    assert!(slider.x() > 40.0, "it only slid to {}", slider.x());
    assert!(slider.y().abs() < 2.0, "it dropped to {}", slider.y());
    assert!(slider.angle().abs() < 0.05);
}

#[test]
fn destroying_a_body_removes_its_fixtures_and_joints() {
    let world = World::new((0.0, 0.0));
    let a = world.new_body((0.0, 0.0), BodyType::Dynamic);
    let shape = a.add_circle(10.0);
    let b = world.new_body((50.0, 0.0), BodyType::Dynamic);
    b.add_circle(10.0);
    let joint = world.new_revolute_joint(&a, &b, (25.0, 0.0), false);
    assert_eq!(world.body_count(), 2);

    a.destroy();
    a.destroy();
    assert!(a.is_destroyed() && shape.is_destroyed() && joint.is_destroyed());
    assert_eq!(shape.body(), None);
    assert_eq!(world.bodies(), std::slice::from_ref(&b));
    world.update(STEP);

    let other = b.add_rectangle((5.0, 5.0));
    other.destroy();
    assert_eq!(b.fixtures().len(), 1);
}

#[test]
#[should_panic(expected = "this body was destroyed")]
fn using_a_destroyed_body_panics() {
    let world = World::new((0.0, 0.0));
    let body = world.new_body((0.0, 0.0), BodyType::Dynamic);
    body.destroy();
    body.position();
}

#[test]
fn local_and_world_points_convert_both_ways() {
    let world = World::new((0.0, 0.0));
    let body = world.new_body((100.0, 50.0), BodyType::Dynamic);
    body.set_angle(std::f32::consts::FRAC_PI_2);
    // A quarter turn clockwise on screen sends the body's +x down.
    let Vec2 { x, y } = body.world_point((10.0, 0.0));
    assert!((x - 100.0).abs() < 1e-3 && (y - 60.0).abs() < 1e-3);
    let Vec2 { x, y } = body.local_point((100.0, 60.0));
    assert!((x - 10.0).abs() < 1e-3 && y.abs() < 1e-3);
}

#[test]
fn polygons_edges_and_chains_collide() {
    let world = World::new((0.0, 600.0));
    let ground = world.new_body((0.0, 0.0), BodyType::Static);
    ground.add_chain(&[(-300.0, 300.0), (0.0, 320.0), (300.0, 300.0)], false);
    ground.add_edge((-300.0, 0.0), (-300.0, 300.0));
    let wedge = world.new_body((-100.0, 100.0), BodyType::Dynamic);
    wedge.add_polygon(&[(-15.0, 10.0), (15.0, 10.0), (0.0, -15.0)]);
    wedge.set_user_data(7);

    run(&world, 3.0);
    assert!(
        wedge.y() > 250.0 && wedge.y() < 330.0,
        "ended at {}",
        wedge.y()
    );
    assert_eq!(wedge.user_data(), 7);
}

#[test]
fn a_contact_says_where_and_how_hard() {
    let world = World::new((0.0, 600.0));
    let floor = world.new_body((0.0, 300.0), BodyType::Static);
    floor.add_rectangle((400.0, 20.0));
    let falling = world.new_body((40.0, 100.0), BodyType::Dynamic);
    falling.add_rectangle((20.0, 20.0));

    let mut landing = None;
    let mut fall_speed = 0.0;
    for _ in 0..240 {
        let before = falling.linear_velocity().y;
        world.update(STEP);
        if let Some(contact) = world.contacts().into_iter().find(|c| c.began) {
            landing = Some(contact);
            fall_speed = before;
            break;
        }
    }
    let contact = landing.expect("the box never landed");
    // Between the bottom of the box and the top of the floor, under the box.
    assert!(
        (contact.point.x - 40.0).abs() < 12.0,
        "x was {}",
        contact.point.x
    );
    assert!(
        (contact.point.y - 290.0).abs() < 6.0,
        "y was {}",
        contact.point.y
    );
    // As hard as it was falling.
    assert!(fall_speed > 100.0);
    assert!(
        (contact.speed - fall_speed).abs() < fall_speed * 0.1,
        "speed was {} after falling at {fall_speed}",
        contact.speed
    );
    // The normal points from the first fixture's body towards the second's.
    let (first, second) = (contact.a.body().unwrap(), contact.b.body().unwrap());
    let towards = second.y() - first.y();
    assert!(contact.normal.y * towards > 0.0);
    assert!(contact.normal.x.abs() < 0.1);
}

#[test]
fn bodies_fixtures_and_joints_work_as_map_keys() {
    use std::collections::HashMap;
    let world = World::new((0.0, 0.0));
    let one = world.new_body((0.0, 0.0), BodyType::Dynamic);
    let two = world.new_body((100.0, 0.0), BodyType::Dynamic);
    let fixture = one.add_circle(10.0);
    two.add_circle(10.0);

    let mut names = HashMap::new();
    names.insert(one.clone(), "one");
    names.insert(two.clone(), "two");
    assert_eq!(names[&one], "one");
    assert_eq!(names[&fixture.body().unwrap()], "one");
    assert_eq!(names[&two], "two");

    let mut fixtures = HashMap::new();
    fixtures.insert(fixture.clone(), 7);
    assert_eq!(fixtures[&one.fixtures()[0]], 7);

    // The same handle in another world is a different body.
    let elsewhere = World::new((0.0, 0.0));
    let other = elsewhere.new_body((0.0, 0.0), BodyType::Dynamic);
    assert!(!names.contains_key(&other));
}

#[test]
fn an_acceleration_does_not_depend_on_mass_or_the_meter() {
    let mut speeds = Vec::new();
    let mut spins = Vec::new();
    for (meter, size) in [(30.0, 20.0), (100.0, 20.0), (100.0, 60.0)] {
        let world = World::new((0.0, 0.0));
        world.set_meter(meter);
        let body = world.new_body((0.0, 0.0), BodyType::Dynamic);
        body.add_rectangle((size, size));
        for _ in 0..30 {
            body.apply_acceleration((400.0, 0.0));
            body.apply_angular_acceleration(3.0);
            world.update(STEP);
        }
        speeds.push(body.linear_velocity().x);
        spins.push(body.angular_velocity());
    }
    // 400 pixels a second squared and 3 radians a second squared, for
    // half a second, whatever the body weighs.
    for speed in speeds {
        assert!((speed - 200.0).abs() < 2.0, "speed was {speed}");
    }
    for spin in spins {
        assert!((spin - 1.5).abs() < 0.02, "spin was {spin}");
    }
}

#[test]
fn a_revolute_motor_turns_at_its_speed_within_its_torque() {
    let world = World::new((0.0, 0.0));
    let base = world.new_body((400.0, 300.0), BodyType::Static);
    let wheel = world.new_body((400.0, 300.0), BodyType::Dynamic);
    wheel.add_circle(30.0);
    let joint = world.new_revolute_joint(&base, &wheel, (400.0, 300.0), false);
    assert_eq!(joint.kind(), JointType::Revolute);
    assert!(!joint.is_motor_enabled());

    // On, but with no torque, the motor does nothing.
    joint.set_motor_enabled(true);
    joint.set_motor_speed(3.0);
    run(&world, 0.5);
    assert!(wheel.angular_velocity().abs() < 0.01);

    joint.set_max_motor_torque(1.0e6);
    run(&world, 1.0);
    assert!(
        (wheel.angular_velocity() - 3.0).abs() < 0.05,
        "turned at {}",
        wheel.angular_velocity()
    );
    assert!((joint.joint_speed() - 3.0).abs() < 0.05);

    // A weak motor speeds up slowly: torque over inertia.
    let world2 = World::new((0.0, 0.0));
    let base2 = world2.new_body((0.0, 0.0), BodyType::Static);
    let wheel2 = world2.new_body((0.0, 0.0), BodyType::Dynamic);
    wheel2.add_circle(30.0);
    let weak = world2.new_revolute_joint(&base2, &wheel2, (0.0, 0.0), false);
    weak.set_motor_enabled(true);
    weak.set_motor_speed(100.0);
    let torque = wheel2.inertia() * 2.0;
    weak.set_max_motor_torque(torque);
    run(&world2, 1.0);
    assert!((wheel2.angular_velocity() - 2.0).abs() < 0.2);
    assert!((weak.motor_torque() - torque).abs() < torque * 0.1);
}

#[test]
fn revolute_limits_stop_the_turn_measured_from_the_start() {
    let world = World::new((0.0, 600.0));
    let base = world.new_body((400.0, 300.0), BodyType::Static);
    base.set_angle(1.0);
    let arm = world.new_body((450.0, 300.0), BodyType::Dynamic);
    arm.add_rectangle((100.0, 10.0));
    arm.set_angle(0.5);
    let joint = world.new_revolute_joint(&base, &arm, (400.0, 300.0), false);
    assert!(joint.joint_angle().abs() < 1e-4);
    joint.set_limits(-0.25, 0.25);
    joint.set_limits_enabled(true);
    assert!(joint.are_limits_enabled());
    assert_eq!(joint.limits(), (-0.25, 0.25));
    run(&world, 2.0);
    assert!((joint.joint_angle() - 0.25).abs() < 0.03);
    assert!((arm.angle() - 0.75).abs() < 0.03);

    joint.set_limits_enabled(false);
    run(&world, 2.0);
    assert!(joint.joint_angle() > 0.5);
}

#[test]
fn a_prismatic_motor_and_limits_drive_and_stop_a_slider() {
    let world = World::new((0.0, 0.0));
    let base = world.new_body((100.0, 300.0), BodyType::Static);
    let slider = world.new_body((100.0, 300.0), BodyType::Dynamic);
    slider.add_rectangle((20.0, 20.0));
    let joint = world.new_prismatic_joint(&base, &slider, (100.0, 300.0), (1.0, 0.0), false);
    joint.set_motor_enabled(true);
    joint.set_motor_speed(50.0);
    joint.set_max_motor_force(1.0e6);
    run(&world, 1.0);
    assert!((joint.joint_speed() - 50.0).abs() < 1.0);
    assert!((joint.joint_translation() - 50.0).abs() < 3.0);

    joint.set_limits(-10.0, 80.0);
    joint.set_limits_enabled(true);
    run(&world, 2.0);
    assert!(
        (joint.joint_translation() - 80.0).abs() < 1.0,
        "{}",
        joint.joint_translation()
    );
}

#[test]
fn reaction_force_holds_up_a_hanging_body() {
    let world = World::new((0.0, 600.0));
    let ceiling = world.new_body((400.0, 100.0), BodyType::Static);
    let weight = world.new_body((400.0, 200.0), BodyType::Dynamic);
    weight.add_circle(10.0);
    let revolute = world.new_revolute_joint(&ceiling, &weight, (400.0, 100.0), false);
    run(&world, 1.0);
    let Vec2 { x: fx, y: fy } = revolute.reaction_force();
    assert!(fx.abs() < 1.0);
    assert!((fy + weight.mass() * 600.0).abs() < weight.mass() * 30.0);
    revolute.destroy();

    let rope = world.new_rope_joint(
        &ceiling,
        &weight,
        (400.0, 100.0),
        (400.0, 200.0),
        100.0,
        false,
    );
    run(&world, 1.0);
    let Vec2 { x: fx, y: fy } = rope.reaction_force();
    assert!(fx.abs() < 1.0);
    assert!((fy + weight.mass() * 600.0).abs() < weight.mass() * 60.0);
    assert!(rope.max_length() == 100.0);
}

#[test]
fn a_distance_joint_holds_its_length_both_ways() {
    let world = World::new((0.0, 0.0));
    let a = world.new_body((100.0, 100.0), BodyType::Dynamic);
    a.add_circle(5.0);
    let b = world.new_body((200.0, 100.0), BodyType::Dynamic);
    b.add_circle(5.0);
    let joint = world.new_distance_joint(&a, &b, (100.0, 100.0), (200.0, 100.0), false);
    assert_eq!(joint.kind(), JointType::Distance);
    assert!((joint.length() - 100.0).abs() < 1e-3);
    b.set_linear_velocity((-200.0, 0.0));
    run(&world, 0.5);
    assert!((distance(a.position(), b.position()) - 100.0).abs() < 2.0);
    b.set_linear_velocity((200.0, 50.0));
    run(&world, 0.5);
    assert!((distance(a.position(), b.position()) - 100.0).abs() < 2.0);
    joint.set_length(60.0);
    run(&world, 1.0);
    assert!((distance(a.position(), b.position()) - 60.0).abs() < 2.0);
}

#[test]
fn a_wheel_joint_rides_on_a_spring_and_can_be_driven() {
    let (world, _floor) = world_with_floor();
    let car = world.new_body((400.0, 440.0), BodyType::Dynamic);
    car.add_rectangle((120.0, 20.0));
    let wheel = world.new_body((400.0, 470.0), BodyType::Dynamic);
    wheel.add_circle(20.0).set_friction(1.0);
    let joint = world.new_wheel_joint(&car, &wheel, (400.0, 470.0), (0.0, 1.0), false);
    assert!((joint.frequency() - 2.0).abs() < 1e-3);
    assert!((joint.damping_ratio() - 0.7).abs() < 1e-3);
    run(&world, 2.0);
    // The suspension squashes a little under the car's weight, and the
    // wheel turns freely, staying under the car.
    assert!(joint.joint_translation() < 0.0);
    assert!((wheel.x() - car.x()).abs() < 1.0);

    joint.set_motor_enabled(true);
    joint.set_motor_speed(5.0);
    joint.set_max_motor_torque(1.0e7);
    let start = car.x();
    run(&world, 1.0);
    assert!(car.x() - start > 20.0);
}

#[test]
fn a_mouse_joint_drags_a_body_to_its_target() {
    let (world, _floor) = world_with_floor();
    let crate_ = world.new_body((300.0, 480.0), BodyType::Dynamic);
    crate_.add_rectangle((40.0, 40.0));
    let count = world.body_count();
    let joint = world.new_mouse_joint(&crate_, (300.0, 480.0));
    assert_eq!(world.body_count(), count, "the ground body stays hidden");
    assert_eq!(world.bodies().len(), count);
    assert_eq!(joint.bodies(), vec![crate_.clone()]);
    joint.set_target((500.0, 200.0));
    assert_eq!(joint.target(), Vec2::new(500.0, 200.0));
    run(&world, 2.0);
    assert!(distance(crate_.position(), (500.0, 200.0)) < 10.0);
    joint.destroy();
    run(&world, 3.0);
    assert!(crate_.y() > 470.0);
}

#[test]
fn a_friction_joint_slows_things_seen_from_above() {
    let world = World::new((0.0, 0.0));
    let ground = world.new_body((0.0, 0.0), BodyType::Static);
    let puck = world.new_body((0.0, 0.0), BodyType::Dynamic);
    puck.add_circle(10.0);
    let joint = world.new_friction_joint(&ground, &puck, (0.0, 0.0), false);
    puck.set_linear_velocity((100.0, 0.0));
    run(&world, 0.5);
    // No force yet, so no friction.
    assert!((puck.linear_velocity().x - 100.0).abs() < 0.5);
    // A force of half its weight-per-meter slows it at a known rate.
    joint.set_max_force(puck.mass() * 100.0);
    joint.set_max_torque(1.0);
    run(&world, 0.5);
    assert!((puck.linear_velocity().x - 50.0).abs() < 5.0);
    run(&world, 1.0);
    assert!(puck.linear_velocity().x.abs() < 1.0);
}

#[test]
fn a_motor_joint_steers_a_body_to_an_offset() {
    let world = World::new((0.0, 0.0));
    let anchor = world.new_body((100.0, 100.0), BodyType::Static);
    let follower = world.new_body((150.0, 100.0), BodyType::Dynamic);
    follower.add_rectangle((10.0, 10.0));
    let joint = world.new_motor_joint(&anchor, &follower, 0.3, false);
    assert_eq!(joint.linear_offset(), Vec2::new(50.0, 0.0));
    joint.set_linear_offset((0.0, 80.0));
    joint.set_angular_offset(1.0);
    run(&world, 2.0);
    assert!(distance(follower.position(), (100.0, 180.0)) < 2.0);
    assert!((follower.angle() - 1.0).abs() < 0.05);
    assert!((joint.correction_factor() - 0.3).abs() < 1e-4);
}

#[test]
fn joints_know_their_bodies_anchors_and_data() {
    let world = World::new((0.0, 0.0));
    let a = world.new_body((0.0, 0.0), BodyType::Dynamic);
    let b = world.new_body((10.0, 0.0), BodyType::Dynamic);
    let joint = world.new_spring_joint(&a, &b, (0.0, 0.0), (10.0, 0.0), 5.0, 1.0);
    assert_eq!(joint.kind(), JointType::Spring);
    assert_eq!(joint.bodies(), vec![a.clone(), b.clone()]);
    assert_eq!(joint.anchors(), (Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)));
    assert!(joint.collide_connected());
    assert_eq!(
        (joint.length(), joint.stiffness(), joint.damping()),
        (10.0, 5.0, 1.0)
    );
    joint.set_user_data(42);
    assert_eq!(joint.user_data(), 42);
    assert_eq!(joint.kind(), JointType::Spring);
    assert_eq!(world.joints(), vec![joint.clone()]);
    assert_eq!(a.joints(), vec![joint.clone()]);
    assert_eq!(world.joint_count(), 1);
}

#[test]
fn a_joint_turns_back_into_its_own_type() {
    let world = World::new((0.0, 0.0));
    let a = world.new_body((0.0, 0.0), BodyType::Dynamic);
    let b = world.new_body((10.0, 0.0), BodyType::Dynamic);
    let weld = world.new_weld_joint(&a, &b, false);
    let joint: Joint = weld.clone().into();
    assert!(joint.downcast::<joint_kind::Revolute>().is_none());
    assert_eq!(joint.downcast::<joint_kind::Weld>(), Some(weld.clone()));
    assert!(matches!(joint.typed(), AnyJoint::Weld(found) if found == weld));
}

#[test]
fn pre_solve_can_make_a_one_way_platform() {
    let world = World::new((0.0, 600.0));
    let platform = world.new_body((400.0, 300.0), BodyType::Static);
    let top = platform.add_rectangle((400.0, 20.0));
    let seen = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = seen.clone();
    world.set_pre_solve(move |contact| {
        counter.set(counter.get() + 1);
        let up = if contact.a == top {
            contact.normal.y
        } else {
            -contact.normal.y
        };
        // The normal points from the platform into the other: up, out of
        // its top, is negative y.
        if up > -0.5 {
            contact.enabled = false;
        }
    });
    assert!(world.has_pre_solve());
    // Thrown up from below, the ball passes through and lands on top.
    let ball = world.new_body((400.0, 400.0), BodyType::Dynamic);
    ball.add_circle(10.0);
    ball.set_linear_velocity((0.0, -700.0));
    run(&world, 3.0);
    assert!(seen.get() > 0);
    assert!((ball.y() - 280.0).abs() < 2.0, "ball at {}", ball.y());
}

#[test]
fn pre_solve_can_change_friction_for_one_contact() {
    let (world, _floor) = world_with_floor();
    let block = world.new_body((400.0, 480.0), BodyType::Dynamic);
    block
        .add_rectangle((40.0, 40.0))
        .set_friction(1.0)
        .set_user_data(7);
    run(&world, 0.5);
    world.set_pre_solve(|contact| {
        if contact.a_data == 7 || contact.b_data == 7 {
            contact.friction = 0.0;
            contact.restitution = 0.9;
            contact.reset_restitution();
            assert_eq!(contact.restitution, 0.0);
        }
    });
    block.set_linear_velocity((100.0, 0.0));
    run(&world, 0.5);
    assert!(
        block.linear_velocity().x > 95.0,
        "{:?}",
        block.linear_velocity()
    );
    world.set_pre_solve(|contact| {
        contact.friction = 0.0;
        contact.reset_friction();
    });
    block.set_linear_velocity((100.0, 0.0));
    run(&world, 0.5);
    assert!(block.linear_velocity().x < 1.0);
    world.clear_pre_solve();
}

#[test]
fn touching_reports_the_push_that_holds_things_up() {
    let (world, floor) = world_with_floor();
    let block = world.new_body((400.0, 480.0), BodyType::Dynamic);
    block.add_rectangle((40.0, 40.0));
    run(&world, 1.0);
    let touches = world.touching();
    assert_eq!(touches.len(), 1);
    let touch = &touches[0];
    assert!(touch.involves(&block) && touch.involves(&floor));
    let weight = block.mass() * 600.0;
    assert!((touch.normal_impulse / STEP - weight).abs() < weight * 0.05);
    assert!(touch.tangent_impulse.abs() < 1e-3);
}

#[test]
fn box_queries_find_fixtures_by_their_bounding_boxes() {
    let world = World::new((0.0, 0.0));
    let a = world.new_body((100.0, 100.0), BodyType::Dynamic);
    let ball = a.add_circle(10.0);
    let b = world.new_body((300.0, 100.0), BodyType::Static);
    let wall = b.add_rectangle((20.0, 200.0));
    // Found before any update.
    assert_eq!(
        world.fixtures_in_box(80.0, 80.0, 120.0, 120.0),
        vec![ball.clone()]
    );
    let mut both = world.fixtures_in_box(0.0, 0.0, 400.0, 95.0);
    both.sort_by_key(|fixture| fixture.bounding_box().0 as i32);
    assert_eq!(both, vec![ball.clone(), wall.clone()]);
    assert!(world.fixtures_in_box(150.0, 150.0, 250.0, 250.0).is_empty());
    assert_eq!(wall.bounding_box(), (290.0, 0.0, 310.0, 200.0));
    // The corners may come in either order.
    assert_eq!(world.fixtures_in_box(120.0, 120.0, 80.0, 80.0), vec![ball]);
}

#[test]
fn ray_cast_all_finds_every_fixture_nearest_first() {
    let world = World::new((0.0, 0.0));
    let far = world
        .new_body((300.0, 0.0), BodyType::Static)
        .add_rectangle((10.0, 10.0));
    let near = world
        .new_body((100.0, 0.0), BodyType::Static)
        .add_rectangle((10.0, 10.0));
    world.update(STEP);
    let hits = world.ray_cast_all((0.0, 0.0), (400.0, 0.0));
    assert_eq!(hits.len(), 2);
    assert_eq!((&hits[0].fixture, &hits[1].fixture), (&near, &far));
    assert!((hits[0].point.x - 95.0).abs() < 0.01);
}

fn close(a: impl Into<Vec2>, b: impl Into<Vec2>) -> bool {
    a.into().distance(b.into()) < 1e-3
}

#[test]
fn bodies_turn_vectors_and_find_their_center() {
    let world = World::new((0.0, 0.0));
    let body = world.new_body((100.0, 50.0), BodyType::Dynamic);
    // A circle off to the right puts the center of mass there.
    body.add_circle_at((10.0, 0.0), 5.0);
    body.set_angle(std::f32::consts::FRAC_PI_2);

    assert!(close(body.local_center(), (10.0, 0.0)));
    assert!(close(body.world_center(), (100.0, 60.0)));
    assert!(close(body.world_vector((1.0, 0.0)), (0.0, 1.0)));
    assert!(close(body.local_vector((0.0, 1.0)), (1.0, 0.0)));
    let points = body.world_points(&[(0.0, 0.0), (10.0, 0.0)]);
    assert!(close(points[0], (100.0, 50.0)) && close(points[1], (100.0, 60.0)));
    let back = body.local_points(&points);
    assert!(close(back[1], (10.0, 0.0)));
}

#[test]
fn a_spinning_body_moves_faster_at_its_rim() {
    let world = World::new((0.0, 0.0));
    let wheel = world.new_body((0.0, 0.0), BodyType::Dynamic);
    wheel.add_circle(10.0);
    wheel.set_linear_velocity((5.0, 0.0));
    wheel.set_angular_velocity(2.0);
    // Clockwise on screen: the point below the hub moves left.
    let Vec2 { x, y } = wheel.linear_velocity_from_world_point((0.0, 10.0));
    assert!(
        (x - (5.0 - 20.0)).abs() < 1e-3 && y.abs() < 1e-3,
        "{x}, {y}"
    );
    let Vec2 { x, y: _ } = wheel.linear_velocity_from_local_point((0.0, -10.0));
    assert!((x - 25.0).abs() < 1e-3);
}

#[test]
fn transforms_set_position_and_angle_together() {
    let world = World::new((0.0, 0.0));
    let body = world.new_body((0.0, 0.0), BodyType::Dynamic);
    body.set_transform((10.0, 20.0), 0.5);
    let (x, y, angle) = body.transform();
    assert!(close((x, y), (10.0, 20.0)) && (angle - 0.5).abs() < 1e-5);
    body.set_x(3.0);
    body.set_y(4.0);
    assert!(close(body.position(), (3.0, 4.0)));
    assert_eq!(body.world().body_count(), 1);
}

#[test]
fn mass_data_can_be_set_and_reset() {
    let world = World::new((0.0, 0.0));
    let body = world.new_body((0.0, 0.0), BodyType::Dynamic);
    let fixture = body.add_rectangle((30.0, 30.0));
    let (_, _, mass, inertia) = body.mass_data();
    assert!((mass - 1.0).abs() < 1e-3, "one square meter, got {mass}");
    assert!((inertia - body.inertia()).abs() < 1e-6);

    body.set_mass_data((5.0, 0.0), 10.0, 3.0);
    let (x, y, mass, inertia) = body.mass_data();
    assert!(close((x, y), (5.0, 0.0)));
    assert!((mass - 10.0).abs() < 1e-3 && (inertia - 3.0).abs() < 1e-3);
    assert!((body.mass() - 10.0).abs() < 1e-3);
    // The fixture keeps its density while it counts for nothing.
    assert!((fixture.density() - 1.0).abs() < 1e-5);

    body.set_mass(2.0);
    assert!((body.mass() - 2.0).abs() < 1e-3 && (body.inertia() - 3.0).abs() < 1e-3);
    body.set_inertia(7.0);
    assert!((body.mass() - 2.0).abs() < 1e-3 && (body.inertia() - 7.0).abs() < 1e-3);

    // Pushing it shows the mass is real.
    body.apply_linear_impulse((4.0, 0.0));
    assert!((body.linear_velocity().x - 2.0).abs() < 1e-3);

    fixture.set_density(2.0);
    assert!((body.mass() - 2.0).abs() < 1e-3, "not yet");
    body.reset_mass_data();
    assert!((body.mass() - 2.0).abs() < 1e-3, "two square meters' worth");
    assert!(close(body.local_center(), (0.0, 0.0)));

    // A new fixture also puts back what the fixtures give.
    body.set_mass(50.0);
    body.add_circle_at((100.0, 0.0), 1.0);
    assert!(body.mass() < 3.0, "got {}", body.mass());
}

#[test]
fn fixtures_ray_cast_and_report_their_mass() {
    let world = World::new((0.0, 0.0));
    let body = world.new_body((100.0, 0.0), BodyType::Dynamic);
    let fixture = body.add_rectangle_at((30.0, 0.0), (30.0, 60.0), 0.0);
    let hit = fixture.ray_cast((0.0, 0.0), (200.0, 0.0)).expect("a hit");
    assert!((hit.point.x - 115.0).abs() < 1e-3 && (hit.normal.x + 1.0).abs() < 1e-3);
    assert!(fixture.ray_cast((0.0, 100.0), (200.0, 100.0)).is_none());

    let (x, y, mass, inertia) = fixture.mass_data();
    assert!(close((x, y), (30.0, 0.0)));
    assert!((mass - 2.0).abs() < 1e-3 && inertia > 0.0);
}

#[test]
fn an_inactive_body_stays_put_and_lets_things_through() {
    let (world, floor) = world_with_floor();
    let ball = world.new_body((400.0, 100.0), BodyType::Dynamic);
    ball.add_circle(10.0);
    ball.set_active(false);
    assert!(!ball.is_active());
    run(&world, 0.5);
    assert_eq!(ball.position(), Vec2::new(400.0, 100.0));

    ball.set_active(true);
    floor.set_active(false);
    run(&world, 2.0);
    assert!(ball.y() > 520.0, "fell through to {}", ball.y());
}

#[test]
fn sleeping_can_be_turned_off_for_a_body_or_the_world() {
    let (world, _) = world_with_floor();
    let a = world.new_body((300.0, 480.0), BodyType::Dynamic);
    a.add_rectangle((40.0, 40.0));
    let b = world.new_body((500.0, 480.0), BodyType::Dynamic);
    b.add_rectangle((40.0, 40.0));
    b.set_sleeping_allowed(false);
    assert!(!b.is_sleeping_allowed() && a.is_sleeping_allowed());
    run(&world, 3.0);
    assert!(!a.is_awake(), "a rests and sleeps");
    assert!(b.is_awake(), "b is kept awake");

    world.set_sleeping_allowed(false);
    assert!(!world.is_sleeping_allowed());
    assert!(a.is_awake());
    run(&world, 3.0);
    assert!(a.is_awake());

    world.set_sleeping_allowed(true);
    run(&world, 3.0);
    assert!(!a.is_awake() && b.is_awake());
}

#[test]
fn a_contact_filter_decides_what_collides() {
    let (world, floor) = world_with_floor();
    floor.fixtures()[0].set_user_data(7);
    let ghost = world.new_body((300.0, 400.0), BodyType::Dynamic);
    ghost.add_circle(10.0).set_user_data(1);
    let rock = world.new_body((500.0, 400.0), BodyType::Dynamic);
    rock.add_circle(10.0).set_user_data(2);
    let sensor = world.new_body((300.0, 600.0), BodyType::Static);
    sensor.add_rectangle((100.0, 20.0)).set_sensor(true);

    world.set_contact_filter(|pair| pair.a_data != 1 && pair.b_data != 1);
    assert!(world.has_contact_filter());
    let mut began = Vec::new();
    for _ in 0..60 {
        world.update(STEP);
        began.extend(world.contacts().into_iter().filter(|c| c.began));
    }
    assert!(ghost.y() > 520.0, "the ghost fell through to {}", ghost.y());
    assert!(
        (rock.y() - 490.0).abs() < 2.0,
        "the rock rests at {}",
        rock.y()
    );
    // Not even the sensor saw the ghost.
    assert!(began.iter().all(|c| !c.involves(&ghost)));
    assert!(began.iter().any(|c| c.involves(&rock)));

    world.clear_contact_filter();
    assert!(!world.has_contact_filter());
}

#[test]
fn the_world_is_locked_while_it_updates() {
    let (world, _) = world_with_floor();
    let ball = world.new_body((400.0, 485.0), BodyType::Dynamic);
    ball.add_circle(10.0);
    assert!(!world.is_locked());
    let inside = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = inside.clone();
    let probe = world.clone();
    world.set_pre_solve(move |_| seen.set(probe.is_locked()));
    run(&world, 0.2);
    assert!(inside.get());
    assert!(world.contact_count() >= 1);
}

#[test]
fn translating_the_origin_moves_everything_by_the_opposite() {
    let (world, _) = world_with_floor();
    let ball = world.new_body((400.0, 300.0), BodyType::Dynamic);
    let fixture = ball.add_circle(10.0);
    ball.set_linear_velocity((5.0, 0.0));
    let mouse = world.new_mouse_joint(&ball, (400.0, 300.0));

    world.translate_origin((100.0, 50.0));
    assert!(close(ball.position(), (300.0, 250.0)));
    assert_eq!(ball.linear_velocity(), Vec2::new(5.0, 0.0));
    assert!(close(mouse.target(), (300.0, 250.0)));
    let (left, top, _, _) = fixture.bounding_box();
    assert!((left - 290.0).abs() < 0.1 && (top - 240.0).abs() < 0.1);
    world.update(STEP);
    let hit = world
        .ray_cast((300.0, 0.0), (300.0, 1000.0))
        .expect("a hit");
    assert_eq!(hit.fixture, fixture);
}

#[test]
fn body_contacts_and_two_point_contacts() {
    let (world, floor) = world_with_floor();
    let crate_ = world.new_body((400.0, 470.0), BodyType::Dynamic);
    crate_.add_rectangle((40.0, 40.0));
    let mut began = None;
    for _ in 0..60 {
        world.update(STEP);
        if let Some(contact) = world.contacts().into_iter().find(|c| c.began) {
            began.get_or_insert(contact);
        }
    }
    let began = began.expect("the crate landed");
    assert_eq!(began.children, (0, 0));
    assert!(!began.points.is_empty());

    let touches = crate_.contacts();
    assert_eq!(touches.len(), 1);
    assert!(touches[0].involves(&floor));
    // A box flat on the floor touches at both bottom corners.
    let points = &touches[0].points;
    assert_eq!(points.len(), 2, "{points:?}");
    for &Vec2 { x, y } in points {
        assert!(
            (y - 500.0).abs() < 1.5 && (x - 400.0).abs() > 15.0,
            "({x}, {y})"
        );
    }
}

#[test]
fn a_chain_contact_says_which_segment_was_hit() {
    let world = World::new((0.0, 600.0));
    let ground = world.new_body((0.0, 0.0), BodyType::Static);
    ground.add_chain(
        &[(0.0, 500.0), (100.0, 500.0), (200.0, 500.0), (300.0, 500.0)],
        false,
    );
    let ball = world.new_body((250.0, 400.0), BodyType::Dynamic);
    ball.add_circle(10.0);
    run(&world, 1.0);
    let touches = ball.contacts();
    assert_eq!(touches.len(), 1);
    let touch = &touches[0];
    let piece = if touch.a.body() == Some(ground.clone()) {
        touch.children.0
    } else {
        touch.children.1
    };
    assert_eq!(piece, 2);
}

#[test]
fn joints_know_their_reference_angle_axis_and_each_limit() {
    let world = World::new((0.0, 0.0));
    let a = world.new_body((0.0, 0.0), BodyType::Dynamic);
    a.add_circle(5.0);
    let b = world.new_body((10.0, 0.0), BodyType::Dynamic);
    b.add_circle(5.0);
    a.set_angle(0.25);
    b.set_angle(1.0);

    let hinge = world.new_revolute_joint(&a, &b, (5.0, 0.0), false);
    assert!((hinge.reference_angle() - 0.75).abs() < 1e-5);
    let weld = world.new_weld_joint(&a, &b, false);
    assert!((weld.reference_angle() - 0.75).abs() < 1e-5);
    weld.destroy();

    let slider = world.new_prismatic_joint(&a, &b, (5.0, 0.0), (0.0, 1.0), false);
    assert!((slider.reference_angle() - 0.75).abs() < 1e-5);
    assert!(close(slider.axis(), (0.0, 1.0)));
    let wheel = world.new_wheel_joint(&a, &b, (5.0, 0.0), (1.0, 0.0), false);
    assert!(close(wheel.axis(), (1.0, 0.0)));

    hinge.set_upper_limit(1.0);
    hinge.set_lower_limit(-0.5);
    assert_eq!((hinge.lower_limit(), hinge.upper_limit()), (-0.5, 1.0));
    assert_eq!(hinge.limits(), (-0.5, 1.0));
}

#[test]
fn shapes_describe_themselves() {
    let circle = Shape::circle_at((5.0, 0.0), 10.0);
    assert_eq!(circle.kind(), ShapeType::Circle);
    assert_eq!(
        (circle.radius(), circle.point()),
        (10.0, Vec2::new(5.0, 0.0))
    );
    assert_eq!(circle.child_count(), 1);
    assert!(circle.points().is_empty());

    let rectangle = Shape::rectangle_at((10.0, 0.0), (20.0, 10.0), 0.0);
    assert_eq!(rectangle.kind(), ShapeType::Polygon);
    assert_eq!(rectangle.radius(), 0.0);
    let corners = rectangle.points();
    assert_eq!(corners.len(), 4);
    assert!(corners.iter().any(|&p| close(p, (0.0, -5.0))));
    assert!(corners.iter().any(|&p| close(p, (20.0, 5.0))));

    let triangle = Shape::polygon(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]);
    assert_eq!(
        (triangle.kind(), triangle.vertex_count()),
        (ShapeType::Polygon, 3)
    );

    let edge = Shape::edge((0.0, 0.0), (10.0, 0.0));
    assert_eq!(edge.kind(), ShapeType::Edge);
    assert_eq!(
        edge.points(),
        vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)]
    );

    let chain = Shape::chain(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], true);
    assert_eq!(chain.kind(), ShapeType::Chain);
    assert_eq!((chain.vertex_count(), chain.child_count()), (3, 3));
    assert_eq!(
        chain.child_edge(2).points(),
        vec![Vec2::new(10.0, 10.0), Vec2::ZERO]
    );
    let open = Shape::chain(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], false);
    assert_eq!(open.child_count(), 2);
}

#[test]
fn shapes_can_be_queried_where_a_body_would_put_them() {
    let rectangle = Shape::rectangle((20.0, 10.0));
    // Turned a quarter, on a body at (100, 100).
    let quarter = std::f32::consts::FRAC_PI_2;
    let (left, top, right, bottom) = rectangle.bounding_box((100.0, 100.0), quarter);
    assert!((left - 95.0).abs() < 1e-3 && (right - 105.0).abs() < 1e-3);
    assert!((top - 90.0).abs() < 1e-3 && (bottom - 110.0).abs() < 1e-3);
    assert!(rectangle.test_point((100.0, 100.0), quarter, (100.0, 108.0)));
    assert!(!rectangle.test_point((100.0, 100.0), quarter, (108.0, 100.0)));

    let (nx, ny, fraction) = rectangle
        .ray_cast((0.0, 100.0), (200.0, 100.0), (100.0, 100.0), quarter)
        .expect("a hit");
    assert!((nx + 1.0).abs() < 1e-3 && ny.abs() < 1e-3);
    assert!((fraction - 95.0 / 200.0).abs() < 1e-3);
    assert!(
        rectangle
            .ray_cast((0.0, 0.0), (200.0, 0.0), (100.0, 100.0), 0.0)
            .is_none()
    );

    // Thirty units to a meter: a 30 by 60 rectangle is two square
    // meters.
    let (x, y, mass, inertia) =
        Shape::rectangle_at((15.0, 0.0), (30.0, 60.0), 0.0).compute_mass(1.0, 30.0);
    assert!(close((x, y), (15.0, 0.0)));
    assert!((mass - 2.0).abs() < 1e-3 && inertia > 0.0);
    assert_eq!(
        Shape::edge((0.0, 0.0), (1.0, 0.0))
            .compute_mass(1.0, 30.0)
            .2,
        0.0
    );
}

#[test]
fn fixtures_are_made_from_shapes_and_can_change_shape() {
    let (world, _) = world_with_floor();
    let ball = world.new_body((400.0, 300.0), BodyType::Dynamic);
    let shape = Shape::circle(10.0);
    let fixture = ball.add_shape(&shape);
    assert_eq!(fixture.shape(), shape);
    let small = ball.mass();

    let circle = || {
        fixture
            .shape()
            .downcast::<shape_kind::Circle>()
            .expect("a circle")
    };
    fixture.set_shape(&circle().with_radius(20.0));
    assert_eq!(fixture.shape().radius(), 20.0);
    assert!((ball.mass() / small - 4.0).abs() < 1e-3);
    run(&world, 3.0);
    assert!((ball.y() - 480.0).abs() < 2.0, "rests at {}", ball.y());

    fixture.set_shape(&circle().with_point((0.0, -10.0)));
    assert_eq!(circle().point(), Vec2::new(0.0, -10.0));
    run(&world, 3.0);
    assert!((ball.y() - 490.0).abs() < 2.0, "rests at {}", ball.y());

    let box_ = world.new_body((200.0, 300.0), BodyType::Dynamic);
    let fixture = box_.add_rectangle_at((5.0, 0.0), (20.0, 10.0), 0.0);
    assert_eq!(
        fixture.shape(),
        Shape::rectangle_at((5.0, 0.0), (20.0, 10.0), 0.0)
    );
}

#[test]
fn a_shape_turns_back_into_its_own_type() {
    let shape: Shape = Shape::rectangle((10.0, 10.0)).into();
    assert!(shape.downcast::<shape_kind::Circle>().is_none());
    assert!(matches!(shape.typed(), AnyShape::Polygon(_)));
}

#[test]
fn a_pulley_lifts_one_side_as_the_other_falls() {
    let world = World::new((0.0, 600.0));
    let light = world.new_body((100.0, 300.0), BodyType::Dynamic);
    light.add_rectangle((20.0, 20.0));
    let heavy = world.new_body((300.0, 300.0), BodyType::Dynamic);
    heavy.add_rectangle((20.0, 20.0)).set_density(3.0);
    let pulley = world.new_pulley_joint(
        &light,
        &heavy,
        (100.0, 100.0),
        (300.0, 100.0),
        (100.0, 300.0),
        (300.0, 300.0),
        1.0,
        false,
    );
    assert_eq!(pulley.kind(), JointType::Pulley);
    assert_eq!(
        pulley.ground_anchors(),
        (Vec2::new(100.0, 100.0), Vec2::new(300.0, 100.0))
    );
    assert!((pulley.constant() - 400.0).abs() < 1e-3);
    assert_eq!(pulley.ratio(), 1.0);

    for _ in 0..30 {
        world.update(STEP);
        let total = pulley.length_a() + pulley.length_b();
        assert!((total - 400.0).abs() < 0.5, "the rope stretched to {total}");
    }
    assert!(light.y() < 290.0, "the light one rose to {}", light.y());
    assert!(heavy.y() > 310.0, "the heavy one fell to {}", heavy.y());
    // Half the difference in weight over the total: g / 2.
    let Vec2 { x: _, y: rising } = light.linear_velocity();
    assert!((rising + 150.0).abs() < 20.0, "rising at {rising}");
    let Vec2 { x: _, y: pull } = pulley.reaction_force();
    assert!(pull < 0.0, "the rope pulls the heavy one up, got {pull}");

    // More rope lets both hang lower.
    pulley.set_constant(500.0);
    world.update(STEP);
    assert!((pulley.length_a() + pulley.length_b() - 500.0).abs() < 1.0);
}

#[test]
fn a_pulley_ratio_trades_distance_for_strength() {
    let world = World::new((0.0, 600.0));
    let a = world.new_body((100.0, 300.0), BodyType::Dynamic);
    a.add_rectangle((20.0, 20.0));
    let b = world.new_body((300.0, 300.0), BodyType::Dynamic);
    b.add_rectangle((20.0, 20.0));
    // With a ratio of 2, b holds up twice its weight: a, as heavy, rises.
    let pulley = world.new_pulley_joint(
        &a,
        &b,
        (100.0, 100.0),
        (300.0, 100.0),
        (100.0, 300.0),
        (300.0, 300.0),
        2.0,
        true,
    );
    run(&world, 0.5);
    let (start_a, start_b) = (200.0, 200.0);
    let (length_a, length_b) = (pulley.length_a(), pulley.length_b());
    assert!(length_a > start_a, "a went down, to a rope of {length_a}");
    assert!(
        ((length_a - start_a) + 2.0 * (length_b - start_b)).abs() < 1.0,
        "a moved twice as far as b"
    );
}

/// A wheel pinned at `(x, y)` to a static body, by a revolute joint.
fn pinned_wheel(
    world: &World,
    ground: &Body,
    x: f32,
    y: f32,
    radius: f32,
) -> (Body, RevoluteJoint) {
    let wheel = world.new_body((x, y), BodyType::Dynamic);
    wheel.add_circle(radius);
    let pin = world.new_revolute_joint(ground, &wheel, (x, y), false);
    (wheel, pin)
}

#[test]
fn a_gear_turns_two_wheels_together() {
    let world = World::new((0.0, 0.0));
    let ground = world.new_body((0.0, 0.0), BodyType::Static);
    let (big, big_pin) = pinned_wheel(&world, &ground, 100.0, 100.0, 40.0);
    let (small, small_pin) = pinned_wheel(&world, &ground, 160.0, 100.0, 20.0);
    let gear = world.new_gear_joint(&big_pin, &small_pin, 0.5, false);
    assert_eq!(gear.kind(), JointType::Gear);
    assert_eq!(gear.joints(), vec![big_pin.clone(), small_pin.clone()]);
    assert_eq!(gear.bodies(), vec![big.clone(), small.clone()]);

    big_pin.set_motor_enabled(true);
    big_pin.set_max_motor_torque(1.0e7);
    big_pin.set_motor_speed(1.0);
    let mut hardest: f32 = 0.0;
    for _ in 0..60 {
        world.update(STEP);
        hardest = hardest.max(gear.reaction_torque().abs());
        let sum = big_pin.joint_angle() + 0.5 * small_pin.joint_angle();
        assert!(sum.abs() < 0.01, "the gears slipped by {sum}");
    }
    assert!(
        big_pin.joint_angle() > 0.5,
        "turned {}",
        big_pin.joint_angle()
    );
    // The small wheel turns twice as fast, the other way.
    let ratio = small.angular_velocity() / big.angular_velocity();
    assert!(
        (ratio + 2.0).abs() < 0.05,
        "turned at {ratio} times the speed"
    );
    // It pushed on the small wheel to spin it up.
    assert!(hardest > 0.0);

    // A new ratio applies from where the wheels are now.
    gear.set_ratio(1.0);
    assert_eq!(gear.ratio(), 1.0);
    let before = big_pin.joint_angle() + small_pin.joint_angle();
    run(&world, 0.5);
    let after = big_pin.joint_angle() + small_pin.joint_angle();
    assert!(
        (after - before).abs() < 0.01,
        "slipped from {before} to {after}"
    );

    // Once a joint is gone the gear does nothing, and does not panic.
    gear.destroy();
    small_pin.destroy();
    run(&world, 0.1);
}

#[test]
fn a_gear_between_a_wheel_and_a_slider_makes_a_rack_and_pinion() {
    let world = World::new((0.0, 0.0));
    let ground = world.new_body((0.0, 0.0), BodyType::Static);
    let (_, pin) = pinned_wheel(&world, &ground, 100.0, 100.0, 20.0);
    let rack = world.new_body((100.0, 130.0), BodyType::Dynamic);
    rack.add_rectangle((200.0, 10.0));
    let slider = world.new_prismatic_joint(&ground, &rack, (100.0, 130.0), (1.0, 0.0), false);
    // The rack moves the pinion's radius for every radian it turns.
    let gear = world.new_gear_joint(&pin, &slider, 1.0 / 20.0, false);

    rack.set_linear_velocity((40.0, 0.0));
    run(&world, 1.0);
    let travel = slider.joint_translation();
    assert!(travel > 10.0, "the rack slid {travel}");
    let turn = pin.joint_angle();
    assert!(
        (turn + travel / 20.0).abs() < 0.02,
        "turned {turn} for {travel}"
    );
    gear.destroy();
}

#[test]
#[should_panic(expected = "ties revolute and prismatic joints")]
fn a_gear_needs_revolute_or_prismatic_joints() {
    let world = World::new((0.0, 0.0));
    let a = world.new_body((0.0, 0.0), BodyType::Dynamic);
    a.add_circle(5.0);
    let b = world.new_body((10.0, 0.0), BodyType::Dynamic);
    b.add_circle(5.0);
    let weld = world.new_weld_joint(&a, &b, false);
    let pin = world.new_revolute_joint(&a, &b, (5.0, 0.0), false);
    world.new_gear_joint(&weld, &pin, 1.0, false);
}

#[test]
fn a_pulley_comes_to_rest_when_one_side_lands() {
    let (world, _) = world_with_floor();
    let light = world.new_body((100.0, 300.0), BodyType::Dynamic);
    light.add_rectangle((20.0, 20.0));
    let heavy = world.new_body((300.0, 400.0), BodyType::Dynamic);
    heavy.add_rectangle((20.0, 20.0)).set_density(3.0);
    let pulley = world.new_pulley_joint(
        &light,
        &heavy,
        (100.0, 100.0),
        (300.0, 100.0),
        (100.0, 300.0),
        (300.0, 400.0),
        1.0,
        false,
    );
    run(&world, 4.0);
    // The heavy one rests on the floor, holding the light one up.
    assert!((heavy.y() - 490.0).abs() < 2.0, "heavy at {}", heavy.y());
    assert!((light.y() - 210.0).abs() < 3.0, "light at {}", light.y());
    // It is solved after the engine's step, not with the floor's
    // contact, so it keeps a little speed, but does not move.
    let still = light.y();
    run(&world, 1.0);
    assert!((light.y() - still).abs() < 0.5, "crept to {}", light.y());
    assert!(light.linear_velocity().y.abs() < 5.0);
    assert!((pulley.length_a() + pulley.length_b() - 500.0).abs() < 1.0);
}

#[test]
fn the_distance_between_fixtures_and_their_closest_points() {
    let world = World::new((0.0, 0.0));
    let a = world
        .new_body((0.0, 0.0), BodyType::Static)
        .add_circle(10.0);
    let b = world
        .new_body((50.0, 0.0), BodyType::Dynamic)
        .add_rectangle((20.0, 20.0));
    let (gap, one, two) = super::distance(&a, &b);
    assert!((gap - 30.0).abs() < 1e-3, "{gap}");
    assert!(
        close(one, (10.0, 0.0)) && close(two, (40.0, 0.0)),
        "{one:?} {two:?}"
    );

    let c = world
        .new_body((15.0, 0.0), BodyType::Dynamic)
        .add_circle(10.0);
    let (gap, one, two) = super::distance(&a, &c);
    assert_eq!(gap, 0.0);
    assert_eq!(one, two);
    assert!(one.x > 4.0 && one.x < 11.0, "{one:?}");
}

/// Slides a frictionless 20 by 20 block to the right at 300 units a
/// second, resting on whatever `ground` gives the ground body, with
/// gravity `gravity`. Returns where it got to after a second and a half.
fn slide(gravity: f32, start_y: f32, ground: impl FnOnce(&Body)) -> Vec2 {
    let world = World::new((0.0, gravity));
    let floor = world.new_body((0.0, 0.0), BodyType::Static);
    ground(&floor);
    let block = world.new_body((35.0, start_y), BodyType::Dynamic);
    block.add_rectangle((20.0, 20.0)).set_friction(0.0);
    block.set_fixed_rotation(true);
    run(&world, 1.0);
    block.set_awake(true);
    block.set_linear_velocity((300.0, 0.0));
    run(&world, 1.5);
    block.position()
}

/// Points 20 apart along y = 500, from x = `from` to `to`.
fn flat_points(from: u32, to: u32) -> Vec<(f32, f32)> {
    (from / 20..=to / 20)
        .map(|i| (i as f32 * 20.0, 500.0))
        .collect()
}

#[test]
fn a_block_slides_over_the_joins_of_a_chain() {
    // Plain friction of the chain, 0.2 averaged with the block's 0, slows
    // it from 300 to 210 over the slide; catching on a join would stop it.
    let Vec2 { x, y } = slide(600.0, 470.0, |ground| {
        ground.add_chain(&flat_points(0, 800), false);
    });
    assert!(x > 400.0, "stopped at {x}");
    assert!((y - 490.0).abs() < 1.0, "y {y}");
}

#[test]
fn a_chain_is_solid_from_both_sides() {
    // Pressed up against the chain from below, the block slides under it.
    let Vec2 { x, y } = slide(-600.0, 530.0, |ground| {
        ground.add_chain(&flat_points(0, 800), false);
    });
    assert!(x > 400.0, "stopped at {x}");
    assert!((y - 510.0).abs() < 1.0, "went through to {y}");
}

#[test]
fn ghost_vertices_join_edges_and_chains_smoothly() {
    let Vec2 { x, y: _ } = slide(600.0, 470.0, |ground| {
        let points = flat_points(0, 800);
        for (index, pair) in points.windows(2).enumerate() {
            let mut edge = Shape::edge((pair[0].0, pair[0].1), (pair[1].0, pair[1].1));
            if index > 0 {
                let (x, y) = points[index - 1];
                edge.set_previous_vertex((x, y));
            }
            if let Some(&(x, y)) = points.get(index + 2) {
                edge.set_next_vertex((x, y));
            }
            ground.add_shape(&edge);
        }
    });
    assert!(x > 400.0, "edges stopped it at {x}");

    let Vec2 { x, y: _ } = slide(600.0, 470.0, |ground| {
        // Two chains that meet at x = 60, just ahead of the block.
        let mut left = Shape::chain(&flat_points(0, 60), false);
        left.set_next_vertex((80.0, 500.0));
        let mut right = Shape::chain(&flat_points(60, 800), false);
        right.set_previous_vertex((40.0, 500.0));
        ground.add_shape(&left);
        ground.add_shape(&right);
    });
    assert!(x > 400.0, "chains stopped it at {x}");
}

#[test]
fn a_wall_at_the_end_of_a_chain_still_stops_things() {
    let Vec2 { x, y: _ } = slide(600.0, 470.0, |ground| {
        ground.add_chain(
            &[(0.0, 500.0), (100.0, 500.0), (200.0, 500.0), (200.0, 400.0)],
            false,
        );
    });
    assert!(
        (x - 190.0).abs() < 1.0,
        "the block's right side is at {}",
        x + 10.0
    );
}

#[test]
fn ghost_vertices_are_kept_with_the_shape() {
    let world = World::new((0.0, 0.0));
    let body = world.new_body((0.0, 0.0), BodyType::Static);
    let mut edge = Shape::edge((0.0, 0.0), (10.0, 0.0));
    assert_eq!(edge.previous_vertex(), None);
    edge.set_previous_vertex((-10.0, 1.0));
    edge.set_next_vertex((20.0, -1.0));
    let fixture = body.add_shape(&edge);
    let shape = fixture.shape();
    let close =
        |a: Option<Vec2>, b: (f32, f32)| a.expect("a ghost vertex").abs_diff_eq(b.into(), 1e-4);
    assert!(close(shape.previous_vertex(), (-10.0, 1.0)));
    assert!(close(shape.next_vertex(), (20.0, -1.0)));

    // Replacing the shape replaces its ghosts too.
    fixture.set_shape(&Shape::edge((0.0, 0.0), (5.0, 0.0)));
    assert_eq!(fixture.shape().next_vertex(), None);
}

#[test]
fn things_stay_inside_a_closed_chain() {
    let world = World::new((0.0, 600.0));
    let walls = world.new_body((0.0, 0.0), BodyType::Static);
    walls.add_chain(
        &[(0.0, 0.0), (200.0, 0.0), (200.0, 200.0), (0.0, 200.0)],
        true,
    );
    let ball = world.new_body((100.0, 100.0), BodyType::Dynamic);
    ball.add_circle(10.0).set_restitution(0.5);
    ball.set_linear_velocity((400.0, -300.0));
    run(&world, 3.0);
    let Vec2 { x, y } = ball.position();
    assert!(
        (10.0..=190.5).contains(&x) && (10.0..=190.5).contains(&y),
        "the ball got out to ({x}, {y})"
    );
}
