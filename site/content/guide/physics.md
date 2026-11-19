+++
title = "Physics"
description = "Worlds, bodies, fixtures, joints, contacts and ray casts."
weight = 8
+++

{{ api(path="physics/index.html", text="smitten::physics") }} simulates
rigid bodies. A {{ api(path="physics/struct.World.html", text="World") }}
holds {{ api(path="physics/struct.Body.html", text="Bodies") }}, each body has
one or more {{ api(path="physics/struct.Fixture.html", text="Fixtures") }} (the
shapes that collide), and joints tie bodies together.

The simulation is done by [Rapier](https://rapier.rs).

## A world with things in it

```rust
use smitten::physics::{BodyType, World};

// Gravity pulls down at 900 units per second, every second.
let world = World::new((0.0, 900.0));

let ground = world.new_body((400.0, 580.0), BodyType::Static);
ground.add_rectangle((800.0, 40.0));

let ball = world.new_body((400.0, 100.0), BodyType::Dynamic);
ball.add_circle(20.0).set_restitution(0.5);

let crate_ = world.new_body((420.0, 0.0), BodyType::Dynamic);
crate_.add_rectangle((40.0, 40.0)).set_friction(0.6);
```

Call `world.update(dt)` once a frame, in `update`. Then draw each body at
its position and angle. Positions are in the units you draw in, with y
down, and angles are in radians, clockwise on screen, so they can go
straight into a `DrawParams`:

```rust
fn update(&mut self, dt: f32) {
    // A long stall shouldn't make the simulation jump.
    self.world.update(dt.min(1.0 / 30.0));
}

fn draw(&mut self) {
    graphics::draw(
        &self.crate_image,
        DrawParams::at(self.crate_.position())
            .rotation(self.crate_.angle())
            .origin(self.crate_image.size() / 2.0),
    );
}
```

`world.draw_debug()` draws every fixture and joint as outlines, which is
the quickest way to see what the simulation sees.

{{ figure(src="physics-example.png", alt="Crates and balls piled in a box, with a chain of planks hanging from the ceiling and a sliding platform.", caption="The physics example. Click to drop things, and drag them with the right button.") }}

## Bodies and fixtures

A body is `Static` (never moves), `Dynamic` (moved by forces and
collisions) or `Kinematic` (moved only by the velocity you give it, as for
moving platforms).

Fixtures are added to a body by shape: `add_circle`, `add_rectangle`,
`add_polygon`, `add_edge` and `add_chain`, plus `add_circle_at` and
`add_rectangle_at` for shapes away from the body's center. Each returns the
`Fixture`, whose `set_friction`, `set_restitution`, `set_density`,
`set_sensor` and `set_filter` tune how it collides.

Move bodies with `apply_force`, `apply_linear_impulse`, `apply_torque` and
`set_linear_velocity`. `set_bullet(true)` keeps fast bodies from passing
through thin ones, and `set_user_data` stores a number of your own, such as
an index into your game's list of things.

The simulation behaves best when bodies are between a tenth of a meter and
ten meters across. `world.set_meter(units)` says how many units make a
meter; it is 30 by default.

## Contacts

`world.contacts()` reports what touched during the last update. Read
it after each `world.update`. Each
{{ api(path="physics/struct.Contact.html", text="Contact") }} says whether
the two fixtures started or stopped touching, and where:

```rust
self.world.update(dt);
for contact in self.world.contacts() {
    if contact.began && contact.speed > 200.0 {
        self.thud.play_once();
    }
}
```

`world.set_pre_solve` runs a closure for each contact before it is
resolved, for one-way platforms and the like:

```rust
let platform = world.new_body((400.0, 300.0), BodyType::Static);
let top = platform.add_rectangle((200.0, 10.0));
world.set_pre_solve(move |contact| {
    // Things below the platform pass up through it.
    let up = if contact.a == top { contact.normal.y } else { -contact.normal.y };
    if up > -0.5 {
        contact.enabled = false;
    }
});
```

## Joints

Each `new_..._joint` method on `World` returns that kind of joint, with only
the methods it has:

```rust
// A plank hanging from a hinge, with a motor.
let hinge = world.new_revolute_joint(&wall, &plank, (250.0, 60.0), false);
hinge.set_motor_enabled(true);
hinge.set_motor_speed(2.0);
hinge.set_max_motor_torque(5000.0);
```

The kinds are revolute, weld, rope, distance, spring, prismatic, wheel,
mouse, friction, motor, pulley and gear joints. `Joint` holds what every
joint has, and `joint.typed()` gives an `AnyJoint` to match on.

## Queries

```rust
// The first thing a ray hits.
if let Some(hit) = world.ray_cast((x, y), (x + 500.0, y)) {
    println!("hit at {} with normal {}", hit.point, hit.normal);
}

// Everything under the mouse.
for fixture in world.fixtures_at(mouse::position()) {
    // ...
}
```

`ray_cast_all` returns every hit, and `fixtures_in_box` finds fixtures
whose bounds overlap a rectangle.

## Leaving physics out

A game that doesn't use physics can leave the engine out of its build by
turning off the `physics` feature:

```toml
smitten = { git = "https://github.com/charlesbthomas/smitten", default-features = false, features = ["sdl-from-source", "video"] }
```

The [Topple](@/examples/_index.md#stack-and-topple) game is a complete
example built on physics.
