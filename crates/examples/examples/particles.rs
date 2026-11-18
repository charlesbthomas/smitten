//! Particle systems.
//!
//! - A fire follows the mouse, with smoke rising from it.
//! - Click for a burst of sparks that fall under gravity.
//! - A ring of embers swirls around a point, pulled in as it turns.
//! - Escape quits.

use std::f32::consts::{FRAC_PI_2, TAU};

use smitten::graphics::{AreaSpreadDistribution, Image, ParticleSystem};
use smitten::{BlendMode, Color, Config, Game, Key, MouseButton, event, graphics, mouse};

struct Particles {
    fire: ParticleSystem,
    smoke: ParticleSystem,
    sparks: ParticleSystem,
    swirl: ParticleSystem,
}

/// A soft white dot, bright in the middle and fading to nothing at the
/// edge. Particles are tinted, so one white picture serves every effect.
fn soft_dot(size: u32) -> Image {
    let center = (size as f32 - 1.0) / 2.0;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let distance = (x as f32 - center).hypot(y as f32 - center) / center;
            let alpha = (1.0 - distance).clamp(0.0, 1.0).powi(2);
            pixels.extend_from_slice(&[255, 255, 255, (alpha * 255.0) as u8]);
        }
    }
    Image::from_rgba(size, size, &pixels).expect("the dot is a valid image")
}

impl Game for Particles {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.05, 0.05, 0.08));
        let dot = soft_dot(32);

        let mut fire = ParticleSystem::new(&dot, 600);
        fire.set_emission_rate(220.0);
        fire.set_particle_lifetime(0.4, 0.9);
        fire.set_emission_area((10.0, 4.0));
        fire.set_direction(-FRAC_PI_2);
        fire.set_spread(0.6);
        fire.set_speed(60.0, 160.0);
        fire.set_linear_acceleration((0.0, -120.0), (0.0, -60.0));
        fire.set_sizes(&[1.2, 0.9, 0.2]);
        fire.set_colors(&[
            Color::rgba(1.0, 0.9, 0.4, 0.9),
            Color::rgba(1.0, 0.45, 0.1, 0.7),
            Color::rgba(0.6, 0.1, 0.05, 0.0),
        ]);

        let mut smoke = ParticleSystem::new(&dot, 200);
        smoke.set_emission_rate(25.0);
        smoke.set_particle_lifetime(2.0, 3.5);
        smoke.set_position((0.0, -40.0));
        smoke.set_emission_area((8.0, 4.0));
        smoke.set_direction(-FRAC_PI_2);
        smoke.set_spread(0.5);
        smoke.set_speed(30.0, 60.0);
        smoke.set_linear_damping(0.2, 0.5);
        smoke.set_spin(-1.0, 1.0);
        smoke.set_spin_variation(1.0);
        smoke.set_sizes(&[0.8, 3.0]);
        smoke.set_colors(&[
            Color::rgba(0.5, 0.5, 0.55, 0.0),
            Color::rgba(0.5, 0.5, 0.55, 0.25),
            Color::rgba(0.4, 0.4, 0.45, 0.0),
        ]);

        // Sparks have no emission rate: they only appear in bursts.
        let mut sparks = ParticleSystem::new(&dot, 1000);
        sparks.set_particle_lifetime(0.5, 1.4);
        sparks.set_spread(TAU);
        sparks.set_speed(120.0, 420.0);
        sparks.set_linear_acceleration((0.0, 500.0), (0.0, 700.0));
        sparks.set_sizes(&[0.35, 0.1]);
        sparks.set_colors(&[
            Color::rgba(1.0, 1.0, 0.8, 1.0),
            Color::rgba(1.0, 0.6, 0.2, 0.0),
        ]);

        // Born on a ring and pushed around it, while a pull towards the
        // middle draws them inwards and their spin changes as they age.
        let mut swirl = ParticleSystem::new(&dot, 800);
        swirl.set_position((600.0, 360.0));
        swirl.set_emission_rate(160.0);
        swirl.set_particle_lifetime(1.5, 2.5);
        swirl.set_emission_area_ex(
            AreaSpreadDistribution::BorderEllipse,
            (70.0, 70.0),
            0.0,
            true,
        );
        swirl.set_direction(FRAC_PI_2);
        swirl.set_speed(40.0, 60.0);
        swirl.set_radial_acceleration(-40.0, -20.0);
        swirl.set_tangential_acceleration(60.0, 90.0);
        swirl.set_sizes(&[0.1, 0.4, 0.05]);
        swirl.set_size_variation(0.5);
        swirl.set_colors(&[
            Color::rgba(0.4, 0.8, 1.0, 0.0),
            Color::rgba(0.5, 0.9, 1.0, 0.9),
            Color::rgba(0.2, 0.3, 1.0, 0.0),
        ]);

        Ok(Particles {
            fire,
            smoke,
            sparks,
            swirl,
        })
    }

    fn update(&mut self, dt: f32) {
        self.fire.update(dt);
        self.smoke.update(dt);
        self.sparks.update(dt);
        self.swirl.update(dt);
    }

    fn draw(&mut self) {
        let smitten::Vec2 { x, y } = mouse::position();
        graphics::set_color(Color::WHITE);
        graphics::draw_particles(&self.smoke, (x, y));

        // Adding the particles together makes the middle of the fire and
        // the sparks glow where they overlap.
        graphics::set_blend_mode(BlendMode::Add);
        graphics::draw_particles(&self.fire, (x, y));
        graphics::draw_particles(&self.sparks, (0.0, 0.0));
        graphics::draw_particles(&self.swirl, (0.0, 0.0));
        graphics::set_blend_mode(BlendMode::Alpha);

        let count =
            self.fire.count() + self.smoke.count() + self.sparks.count() + self.swirl.count();
        graphics::print(
            &format!("{count} particles. Click for sparks."),
            (16.0, 16.0),
        );
    }

    fn mouse_pressed(&mut self, x: f32, y: f32, button: MouseButton, _presses: u32) {
        if button == MouseButton::Left {
            self.sparks.set_position((x, y));
            self.sparks.emit(150);
        }
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        if key == Key::Escape {
            event::quit();
        }
    }
}

fn main() {
    smitten::run::<Particles>(Config {
        title: "Smitten particles".into(),
        width: 720,
        height: 480,
        ..Config::default()
    });
}
