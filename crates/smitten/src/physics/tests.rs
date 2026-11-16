use super::*;

const STEP: f32 = 1.0 / 60.0;

fn run(world: &World, seconds: f32) {
    for _ in 0..(seconds / STEP).round() as usize {
        world.update(STEP);
    }
}

/// A world with a wide floor whose top is at y = 500.
fn world_with_floor() -> (World, Body) {
    let world = World::new(0.0, 600.0);
    let floor = world.new_body(400.0, 520.0, BodyType::Static);
    floor.add_rectangle(2000.0, 40.0);
    (world, floor)
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

#[test]
fn a_body_falls_and_comes_to_rest_on_the_floor() {
    let (world, floor) = world_with_floor();
    let crate_ = world.new_body(400.0, 100.0, BodyType::Dynamic);
    crate_.add_rectangle(40.0, 40.0);

    run(&world, 0.25);
    assert!(crate_.y() > 100.0 && crate_.linear_velocity().1 > 0.0);

    run(&world, 4.0);
    // Its center rests half its height above the floor's top.
    assert!((crate_.y() - 480.0).abs() < 2.0, "rested at {}", crate_.y());
    assert!(crate_.linear_velocity().1.abs() < 1.0);
    assert!(crate_.is_touching(&floor));
    assert_eq!(floor.position(), (400.0, 520.0));
}

#[test]
fn gravity_matches_the_numbers_given() {
    let world = World::new(0.0, 600.0);
    let body = world.new_body(0.0, 0.0, BodyType::Dynamic);
    body.add_circle(10.0);
    run(&world, 1.0);
    // After a second of free fall: speed g, distance about g / 2.
    assert!((body.linear_velocity().1 - 600.0).abs() < 1.0);
    assert!((body.y() - 300.0).abs() < 10.0, "fell {}", body.y());

    world.set_gravity(0.0, 0.0);
    assert_eq!(world.gravity(), (0.0, 0.0));
}

#[test]
fn restitution_makes_things_bounce() {
    let bounce_height = |restitution: f32| {
        let (world, _floor) = world_with_floor();
        let ball = world.new_body(400.0, 300.0, BodyType::Dynamic);
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
    let ball = world.new_body(400.0, 440.0, BodyType::Dynamic);
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
    ball.set_position(400.0, 100.0);
    ball.set_linear_velocity(0.0, 0.0);
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
    let world = World::new(0.0, 600.0);
    let zone = world.new_body(0.0, 200.0, BodyType::Static);
    zone.add_rectangle(200.0, 20.0).set_sensor(true);
    let ball = world.new_body(0.0, 0.0, BodyType::Dynamic);
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
    let ghost = world.new_body(300.0, 400.0, BodyType::Dynamic);
    ghost.add_circle(10.0).set_filter(0b10, 0b10);
    let solid = world.new_body(500.0, 400.0, BodyType::Dynamic);
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
    let world = World::new(0.0, 0.0);
    let near = world.new_body(100.0, 0.0, BodyType::Static);
    let near_shape = near.add_rectangle(20.0, 20.0);
    let far = world.new_body(200.0, 0.0, BodyType::Static);
    far.add_rectangle(20.0, 20.0);
    world.update(STEP);

    let hit = world.ray_cast(0.0, 0.0, 400.0, 0.0).expect("the ray hits");
    assert_eq!(hit.fixture, near_shape);
    assert!((hit.x - 90.0).abs() < 0.01 && hit.y.abs() < 0.01);
    assert!((hit.normal_x + 1.0).abs() < 0.01);
    assert!((hit.fraction - 0.225).abs() < 0.001);

    assert!(world.ray_cast(0.0, 100.0, 400.0, 100.0).is_none());
    assert!(world.ray_cast(0.0, 0.0, 50.0, 0.0).is_none());
}

#[test]
fn points_can_be_tested_against_fixtures() {
    let world = World::new(0.0, 0.0);
    let body = world.new_body(100.0, 100.0, BodyType::Static);
    let shape = body.add_circle(20.0);
    world.update(STEP);

    assert_eq!(
        world.fixtures_at(110.0, 100.0),
        std::slice::from_ref(&shape)
    );
    assert!(world.fixtures_at(130.0, 100.0).is_empty());
    assert!(shape.test_point(100.0, 115.0));
    assert!(!shape.test_point(100.0, 125.0));
}

#[test]
fn mass_comes_from_area_density_and_the_meter() {
    let world = World::new(0.0, 0.0);
    assert_eq!(world.meter(), 30.0);
    let body = world.new_body(0.0, 0.0, BodyType::Dynamic);
    // Two meters by one meter at the default density of 1.
    let shape = body.add_rectangle(60.0, 30.0);
    assert!((body.mass() - 2.0).abs() < 1e-3, "mass was {}", body.mass());
    assert!((shape.density() - 1.0).abs() < 1e-4);

    shape.set_density(3.0);
    assert!((body.mass() - 6.0).abs() < 1e-3);
    assert_eq!(shape.friction(), 0.2);
    assert_eq!(shape.restitution(), 0.0);
}

#[test]
fn impulses_change_velocity_by_impulse_over_mass() {
    let world = World::new(0.0, 0.0);
    let body = world.new_body(0.0, 0.0, BodyType::Dynamic);
    body.add_rectangle(60.0, 30.0);
    body.apply_linear_impulse(100.0, 0.0);
    world.update(STEP);
    assert!((body.linear_velocity().0 - 50.0).abs() < 0.5);
}

#[test]
fn a_force_lasts_for_one_update_only() {
    let world = World::new(0.0, 0.0);
    let pushed = world.new_body(0.0, 0.0, BodyType::Dynamic);
    pushed.add_rectangle(30.0, 30.0);
    // Out of the way of the first, which will move along x.
    let twisted = world.new_body(0.0, 500.0, BodyType::Dynamic);
    twisted.add_rectangle(30.0, 30.0);

    // Hard enough that neither is slow enough to be put to sleep.
    pushed.apply_force(20_000.0, 0.0);
    twisted.apply_torque(200_000.0);
    world.update(STEP);
    let speed = pushed.linear_velocity().0;
    let spin = twisted.angular_velocity();
    assert!(speed > 0.0 && spin > 0.0);

    // With nothing pushing any more, both carry on as they were.
    run(&world, 0.5);
    assert!((pushed.linear_velocity().0 - speed).abs() < speed * 1e-3);
    assert!((twisted.angular_velocity() - spin).abs() < spin * 1e-3);

    // A force applied every update is a steady push.
    for _ in 0..10 {
        pushed.apply_force(20_000.0, 0.0);
        world.update(STEP);
    }
    assert!((pushed.linear_velocity().0 - speed * 11.0).abs() < speed * 0.01);
}

#[test]
fn a_kinematic_body_moves_at_its_velocity_and_carries_things() {
    let world = World::new(0.0, 600.0);
    let platform = world.new_body(0.0, 300.0, BodyType::Kinematic);
    platform.add_rectangle(200.0, 20.0).set_friction(1.0);
    platform.set_linear_velocity(60.0, 0.0);
    let rider = world.new_body(0.0, 270.0, BodyType::Dynamic);
    rider.add_rectangle(20.0, 20.0).set_friction(1.0);

    run(&world, 2.0);
    assert!((platform.x() - 120.0).abs() < 1.0);
    assert!((platform.y() - 300.0).abs() < 0.01);
    assert!(rider.x() > 80.0, "the rider only got to {}", rider.x());
}

#[test]
fn fixed_rotation_keeps_a_body_upright() {
    let world = World::new(0.0, 0.0);
    let body = world.new_body(0.0, 0.0, BodyType::Dynamic);
    body.add_rectangle(20.0, 40.0);
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
    let world = World::new(0.0, 600.0);
    let pivot = world.new_body(0.0, 0.0, BodyType::Static);
    let bob = world.new_body(100.0, 0.0, BodyType::Dynamic);
    bob.add_circle(10.0);
    world.new_revolute_joint(&pivot, &bob, 0.0, 0.0, false);

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
    let world = World::new(0.0, 0.0);
    let a = world.new_body(0.0, 0.0, BodyType::Dynamic);
    a.add_rectangle(20.0, 20.0);
    let b = world.new_body(30.0, 0.0, BodyType::Dynamic);
    b.add_rectangle(20.0, 20.0);
    world.new_weld_joint(&a, &b, false);
    a.set_angular_velocity(2.0);
    a.set_linear_velocity(50.0, 0.0);

    run(&world, 1.0);
    assert!((distance(a.position(), b.position()) - 30.0).abs() < 1.0);
    assert!((a.angle() - b.angle()).abs() < 0.05);
    assert!(b.x() > 30.0);
}

#[test]
fn a_rope_joint_limits_distance_and_can_be_destroyed() {
    let world = World::new(0.0, 600.0);
    let hook = world.new_body(0.0, 0.0, BodyType::Static);
    let weight = world.new_body(0.0, 10.0, BodyType::Dynamic);
    weight.add_circle(5.0);
    let rope = world.new_rope_joint(&hook, &weight, 0.0, 0.0, 0.0, 10.0, 80.0, false);

    run(&world, 3.0);
    assert!((weight.y() - 80.0).abs() < 3.0, "hung at {}", weight.y());

    rope.destroy();
    assert!(rope.is_destroyed());
    run(&world, 1.0);
    assert!(weight.y() > 200.0);
}

#[test]
fn a_spring_joint_pulls_back_toward_its_rest_length() {
    let world = World::new(0.0, 0.0);
    let anchor = world.new_body(0.0, 0.0, BodyType::Static);
    let weight = world.new_body(100.0, 0.0, BodyType::Dynamic);
    weight.add_circle(10.0);
    world.new_spring_joint(&anchor, &weight, 0.0, 0.0, 100.0, 0.0, 50.0, 5.0);

    weight.set_position(160.0, 0.0);
    run(&world, 5.0);
    assert!(
        (weight.x() - 100.0).abs() < 5.0,
        "settled at {}",
        weight.x()
    );
}

#[test]
fn a_prismatic_joint_only_lets_a_body_slide_along_its_axis() {
    let world = World::new(0.0, 600.0);
    let rail = world.new_body(0.0, 0.0, BodyType::Static);
    let slider = world.new_body(0.0, 0.0, BodyType::Dynamic);
    slider.add_rectangle(20.0, 20.0);
    // Slides sideways only, so gravity cannot move it.
    world.new_prismatic_joint(&rail, &slider, 0.0, 0.0, 1.0, 0.0, false);
    slider.apply_linear_impulse(60.0 * slider.mass(), 0.0);

    run(&world, 1.0);
    assert!(slider.x() > 40.0, "it only slid to {}", slider.x());
    assert!(slider.y().abs() < 2.0, "it dropped to {}", slider.y());
    assert!(slider.angle().abs() < 0.05);
}

#[test]
fn destroying_a_body_removes_its_fixtures_and_joints() {
    let world = World::new(0.0, 0.0);
    let a = world.new_body(0.0, 0.0, BodyType::Dynamic);
    let shape = a.add_circle(10.0);
    let b = world.new_body(50.0, 0.0, BodyType::Dynamic);
    b.add_circle(10.0);
    let joint = world.new_revolute_joint(&a, &b, 25.0, 0.0, false);
    assert_eq!(world.body_count(), 2);

    a.destroy();
    a.destroy();
    assert!(a.is_destroyed() && shape.is_destroyed() && joint.is_destroyed());
    assert_eq!(shape.body(), None);
    assert_eq!(world.bodies(), std::slice::from_ref(&b));
    world.update(STEP);

    let other = b.add_rectangle(5.0, 5.0);
    other.destroy();
    assert_eq!(b.fixtures().len(), 1);
}

#[test]
#[should_panic(expected = "this body was destroyed")]
fn using_a_destroyed_body_panics() {
    let world = World::new(0.0, 0.0);
    let body = world.new_body(0.0, 0.0, BodyType::Dynamic);
    body.destroy();
    body.position();
}

#[test]
fn local_and_world_points_convert_both_ways() {
    let world = World::new(0.0, 0.0);
    let body = world.new_body(100.0, 50.0, BodyType::Dynamic);
    body.set_angle(std::f32::consts::FRAC_PI_2);
    // A quarter turn clockwise on screen sends the body's +x down.
    let (x, y) = body.world_point(10.0, 0.0);
    assert!((x - 100.0).abs() < 1e-3 && (y - 60.0).abs() < 1e-3);
    let (x, y) = body.local_point(100.0, 60.0);
    assert!((x - 10.0).abs() < 1e-3 && y.abs() < 1e-3);
}

#[test]
fn polygons_edges_and_chains_collide() {
    let world = World::new(0.0, 600.0);
    let ground = world.new_body(0.0, 0.0, BodyType::Static);
    ground.add_chain(&[(-300.0, 300.0), (0.0, 320.0), (300.0, 300.0)], false);
    ground.add_edge(-300.0, 0.0, -300.0, 300.0);
    let wedge = world.new_body(-100.0, 100.0, BodyType::Dynamic);
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
    let world = World::new(0.0, 600.0);
    let floor = world.new_body(0.0, 300.0, BodyType::Static);
    floor.add_rectangle(400.0, 20.0);
    let falling = world.new_body(40.0, 100.0, BodyType::Dynamic);
    falling.add_rectangle(20.0, 20.0);

    let mut landing = None;
    let mut fall_speed = 0.0;
    for _ in 0..240 {
        let before = falling.linear_velocity().1;
        world.update(STEP);
        if let Some(contact) = world.contacts().into_iter().find(|c| c.began) {
            landing = Some(contact);
            fall_speed = before;
            break;
        }
    }
    let contact = landing.expect("the box never landed");
    // Between the bottom of the box and the top of the floor, under the box.
    assert!((contact.x - 40.0).abs() < 12.0, "x was {}", contact.x);
    assert!((contact.y - 290.0).abs() < 6.0, "y was {}", contact.y);
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
    assert!(contact.normal.1 * towards > 0.0);
    assert!(contact.normal.0.abs() < 0.1);
}

#[test]
fn bodies_fixtures_and_joints_work_as_map_keys() {
    use std::collections::HashMap;
    let world = World::new(0.0, 0.0);
    let one = world.new_body(0.0, 0.0, BodyType::Dynamic);
    let two = world.new_body(100.0, 0.0, BodyType::Dynamic);
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
    let elsewhere = World::new(0.0, 0.0);
    let other = elsewhere.new_body(0.0, 0.0, BodyType::Dynamic);
    assert!(!names.contains_key(&other));
}

#[test]
fn an_acceleration_does_not_depend_on_mass_or_the_meter() {
    let mut speeds = Vec::new();
    let mut spins = Vec::new();
    for (meter, size) in [(30.0, 20.0), (100.0, 20.0), (100.0, 60.0)] {
        let world = World::new(0.0, 0.0);
        world.set_meter(meter);
        let body = world.new_body(0.0, 0.0, BodyType::Dynamic);
        body.add_rectangle(size, size);
        for _ in 0..30 {
            body.apply_acceleration(400.0, 0.0);
            body.apply_angular_acceleration(3.0);
            world.update(STEP);
        }
        speeds.push(body.linear_velocity().0);
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
