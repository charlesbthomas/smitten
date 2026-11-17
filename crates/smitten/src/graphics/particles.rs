use smitten_core::backend::Vertex;

use super::{Image, Transform};
use crate::Color;

/// Many small copies of one image that are born, move and fade on their
/// own, the equivalent of LÖVE's `ParticleSystem`. Use it for smoke, fire,
/// sparks, dust and the like.
///
/// Set up how particles behave with the `set_` methods, call
/// [`update`](ParticleSystem::update) every frame, and draw it with
/// [`graphics::draw_particles`](super::draw_particles).
///
/// A new system emits nothing until it is given an emission rate or
/// [`emit`](ParticleSystem::emit) is called. Particles are positioned
/// relative to the point the system is drawn at.
#[derive(Debug, Clone)]
pub struct ParticleSystem {
    image: Image,
    max: usize,
    particles: Vec<Particle>,
    rng: Rng,

    active: bool,
    /// Seconds the emitter has been running since it was started.
    emitter_age: f32,
    /// How long the emitter runs before stopping, or `None` for forever.
    emitter_lifetime: Option<f32>,
    /// Particles owed but not yet emitted, as a fraction of one.
    owed: f32,

    emission_rate: f32,
    position: (f32, f32),
    /// Half the size of the box particles appear in.
    area: (f32, f32),
    lifetime: (f32, f32),
    speed: (f32, f32),
    direction: f32,
    spread: f32,
    acceleration: ((f32, f32), (f32, f32)),
    damping: (f32, f32),
    rotation: (f32, f32),
    spin: (f32, f32),
    sizes: Vec<f32>,
    colors: Vec<Color>,
}

#[derive(Debug, Clone, Copy)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    ax: f32,
    ay: f32,
    damping: f32,
    rotation: f32,
    spin: f32,
    age: f32,
    lifetime: f32,
}

impl Particle {
    /// How far through its life the particle is, from 0.0 to 1.0.
    fn progress(&self) -> f32 {
        if self.lifetime > 0.0 {
            (self.age / self.lifetime).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
}

impl ParticleSystem {
    /// Creates a system that draws each particle as `image` and holds at
    /// most `max_particles` at once.
    pub fn new(image: &Image, max_particles: usize) -> ParticleSystem {
        ParticleSystem {
            image: image.clone(),
            max: max_particles,
            particles: Vec::with_capacity(max_particles.min(4096)),
            rng: Rng::new(0x5EED_1E55),
            active: true,
            emitter_age: 0.0,
            emitter_lifetime: None,
            owed: 0.0,
            emission_rate: 0.0,
            position: (0.0, 0.0),
            area: (0.0, 0.0),
            lifetime: (1.0, 1.0),
            speed: (0.0, 0.0),
            direction: 0.0,
            spread: 0.0,
            acceleration: ((0.0, 0.0), (0.0, 0.0)),
            damping: (0.0, 0.0),
            rotation: (0.0, 0.0),
            spin: (0.0, 0.0),
            sizes: vec![1.0],
            colors: vec![Color::WHITE],
        }
    }

    // --- How particles are emitted ---

    /// Sets how many particles are emitted per second while the system is
    /// active.
    pub fn set_emission_rate(&mut self, per_second: f32) {
        self.emission_rate = per_second.max(0.0);
    }

    /// Sets how long the system keeps emitting after [`start`], in
    /// seconds, or `None` to emit until stopped. Particles already emitted
    /// live on after it ends.
    ///
    /// [`start`]: ParticleSystem::start
    pub fn set_emitter_lifetime(&mut self, seconds: Option<f32>) {
        self.emitter_lifetime = seconds;
    }

    /// Sets where new particles appear, relative to where the system is
    /// drawn. Move this to leave a trail behind a moving object.
    pub fn set_position(&mut self, x: f32, y: f32) {
        self.position = (x, y);
    }

    /// Returns where new particles appear.
    pub fn position(&self) -> (f32, f32) {
        self.position
    }

    /// Spreads new particles evenly over a box reaching `dx` to each side
    /// of the position and `dy` above and below it.
    pub fn set_emission_area(&mut self, dx: f32, dy: f32) {
        self.area = (dx.abs(), dy.abs());
    }

    // --- How each particle behaves ---

    /// Sets how long each particle lives, in seconds. Each gets a random
    /// time between `min` and `max`.
    pub fn set_particle_lifetime(&mut self, min: f32, max: f32) {
        self.lifetime = (min.max(0.0), max.max(0.0));
    }

    /// Sets the speed particles start with, in pixels per second. Each
    /// gets a random speed between `min` and `max`.
    pub fn set_speed(&mut self, min: f32, max: f32) {
        self.speed = (min, max);
    }

    /// Sets the direction particles start out in, in radians. 0.0 is to
    /// the right and positive angles turn clockwise on screen.
    pub fn set_direction(&mut self, radians: f32) {
        self.direction = radians;
    }

    /// Sets the width, in radians, of the fan of directions around
    /// [`set_direction`](ParticleSystem::set_direction). A full turn sends
    /// particles every way.
    pub fn set_spread(&mut self, radians: f32) {
        self.spread = radians;
    }

    /// Sets a constant acceleration in pixels per second squared, such as
    /// gravity. Each particle gets a random one between `min` and `max`.
    pub fn set_linear_acceleration(&mut self, min: (f32, f32), max: (f32, f32)) {
        self.acceleration = (min, max);
    }

    /// Sets how strongly particles slow down over time. 0.0 is not at
    /// all. Each gets a random amount between `min` and `max`.
    pub fn set_linear_damping(&mut self, min: f32, max: f32) {
        self.damping = (min.max(0.0), max.max(0.0));
    }

    /// Sets the angle particles start at, in radians. Each gets a random
    /// angle between `min` and `max`.
    pub fn set_rotation(&mut self, min: f32, max: f32) {
        self.rotation = (min, max);
    }

    /// Sets how fast particles turn, in radians per second. Each gets a
    /// random rate between `min` and `max`.
    pub fn set_spin(&mut self, min: f32, max: f32) {
        self.spin = (min, max);
    }

    /// Sets the sizes a particle passes through over its life, as scale
    /// factors of the image. One value keeps a constant size; several are
    /// blended between evenly, so `&[0.0, 1.0, 0.0]` grows then shrinks.
    ///
    /// Panics if `sizes` is empty.
    pub fn set_sizes(&mut self, sizes: &[f32]) {
        assert!(!sizes.is_empty(), "a particle system needs a size");
        self.sizes = sizes.to_vec();
    }

    /// Sets the colors a particle passes through over its life, blended
    /// between evenly. End on a transparent color to fade out.
    ///
    /// Panics if `colors` is empty.
    pub fn set_colors(&mut self, colors: &[Color]) {
        assert!(!colors.is_empty(), "a particle system needs a color");
        self.colors = colors.to_vec();
    }

    /// Makes the random choices repeat exactly for the same seed.
    pub fn set_seed(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
    }

    // --- Running the system ---

    /// Emits `count` particles right now, as far as there is room. Works
    /// whether or not the system is active.
    pub fn emit(&mut self, count: usize) {
        for _ in 0..count {
            if self.particles.len() >= self.max {
                break;
            }
            let particle = self.spawn();
            self.particles.push(particle);
        }
    }

    /// Starts emitting again from the beginning of the emitter lifetime.
    /// A new system is already started.
    pub fn start(&mut self) {
        self.active = true;
        self.emitter_age = 0.0;
    }

    /// Stops emitting. Particles already emitted live on.
    pub fn stop(&mut self) {
        self.active = false;
        self.owed = 0.0;
    }

    /// Stops emitting, like [`stop`](ParticleSystem::stop), but
    /// [`resume`](ParticleSystem::resume) carries on where it left off.
    pub fn pause(&mut self) {
        self.active = false;
    }

    /// Carries on emitting after a [`pause`](ParticleSystem::pause).
    pub fn resume(&mut self) {
        self.active = true;
    }

    /// Removes every particle and starts the emitter over.
    pub fn reset(&mut self) {
        self.particles.clear();
        self.owed = 0.0;
        self.start();
    }

    /// Returns whether the system is emitting.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Returns how many particles are alive.
    pub fn count(&self) -> usize {
        self.particles.len()
    }

    /// Returns the most particles the system holds at once.
    pub fn max_particles(&self) -> usize {
        self.max
    }

    /// Moves every particle forward by `dt` seconds, removes the ones
    /// whose life is over, and emits new ones.
    pub fn update(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        for particle in &mut self.particles {
            particle.age += dt;
            particle.vx += particle.ax * dt;
            particle.vy += particle.ay * dt;
            // Damping removes a fraction of the speed per second.
            let keep = 1.0 / (1.0 + particle.damping * dt);
            particle.vx *= keep;
            particle.vy *= keep;
            particle.x += particle.vx * dt;
            particle.y += particle.vy * dt;
            particle.rotation += particle.spin * dt;
        }
        self.particles
            .retain(|particle| particle.age < particle.lifetime);

        if !self.active {
            return;
        }
        // Only emit for the part of this step the emitter was alive for.
        let mut emitting = dt;
        if let Some(lifetime) = self.emitter_lifetime {
            emitting = emitting.min((lifetime - self.emitter_age).max(0.0));
        }
        self.emitter_age += dt;
        self.owed += self.emission_rate * emitting;
        let count = self.owed.floor();
        self.owed -= count;
        self.emit(count as usize);
        if self
            .emitter_lifetime
            .is_some_and(|lifetime| self.emitter_age >= lifetime)
        {
            self.stop();
        }
    }

    fn spawn(&mut self) -> Particle {
        let rng = &mut self.rng;
        let angle = self.direction + rng.between(-0.5, 0.5) * self.spread;
        let speed = rng.between(self.speed.0, self.speed.1);
        let (min, max) = self.acceleration;
        Particle {
            x: self.position.0 + rng.between(-self.area.0, self.area.0),
            y: self.position.1 + rng.between(-self.area.1, self.area.1),
            vx: angle.cos() * speed,
            vy: angle.sin() * speed,
            ax: rng.between(min.0, max.0),
            ay: rng.between(min.1, max.1),
            damping: rng.between(self.damping.0, self.damping.1),
            rotation: rng.between(self.rotation.0, self.rotation.1),
            spin: rng.between(self.spin.0, self.spin.1),
            age: 0.0,
            lifetime: rng.between(self.lifetime.0, self.lifetime.1),
        }
    }

    pub(crate) fn image(&self) -> &Image {
        &self.image
    }

    /// Builds two triangles per particle, oldest first so newer particles
    /// draw on top. `tint` multiplies every particle's color.
    pub(crate) fn vertices(&self, transform: &Transform, tint: Color) -> Vec<Vertex> {
        let half_width = self.image.width() as f32 / 2.0;
        let half_height = self.image.height() as f32 / 2.0;
        let mut vertices = Vec::with_capacity(self.particles.len() * 6);

        for particle in &self.particles {
            let progress = particle.progress();
            let size = sample(&self.sizes, progress, |a, b, t| a + (b - a) * t);
            let color = sample(&self.colors, progress, mix);
            let color = [
                color.r * tint.r,
                color.g * tint.g,
                color.b * tint.b,
                color.a * tint.a,
            ];
            let (sin, cos) = particle.rotation.sin_cos();
            let corner = |dx: f32, dy: f32, u: f32, v: f32| {
                let (dx, dy) = (dx * half_width * size, dy * half_height * size);
                let (x, y) = transform.apply(
                    particle.x + dx * cos - dy * sin,
                    particle.y + dx * sin + dy * cos,
                );
                Vertex {
                    position: [x, y],
                    uv: [u, v],
                    color,
                }
            };
            let top_left = corner(-1.0, -1.0, 0.0, 0.0);
            let top_right = corner(1.0, -1.0, 1.0, 0.0);
            let bottom_right = corner(1.0, 1.0, 1.0, 1.0);
            let bottom_left = corner(-1.0, 1.0, 0.0, 1.0);
            vertices.extend_from_slice(&[
                top_left,
                top_right,
                bottom_right,
                top_left,
                bottom_right,
                bottom_left,
            ]);
        }
        vertices
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

/// Picks the value `progress` of the way through `stops`, which are spaced
/// evenly from 0.0 to 1.0, blending between the two nearest.
fn sample<T: Copy>(stops: &[T], progress: f32, blend: impl Fn(T, T, f32) -> T) -> T {
    let last = stops.len() - 1;
    let position = progress.clamp(0.0, 1.0) * last as f32;
    let index = (position.floor() as usize).min(last);
    let next = (index + 1).min(last);
    blend(stops[index], stops[next], position - index as f32)
}

/// A small random number generator (xorshift64*). Particle effects need
/// variety, not unpredictability, so this avoids a dependency.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // The state must never be zero.
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// A number from 0.0 up to but not including 1.0.
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let bits = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
        bits as f32 / (1u64 << 24) as f32
    }

    /// A number from `low` to `high`, whichever order they are in.
    fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A system drawing an 8x8 image. No GPU is involved: without a
    /// running game, dropping the image frees nothing.
    fn system(max: usize) -> ParticleSystem {
        let image = Image::from_texture(
            smitten_core::backend::TextureId(1),
            8,
            8,
            crate::Filter::Linear,
            super::super::stats::Kind::Image,
        );
        ParticleSystem::new(&image, max)
    }

    #[test]
    fn nothing_is_emitted_without_a_rate() {
        let mut particles = system(100);
        particles.update(1.0);
        assert_eq!(particles.count(), 0);
    }

    #[test]
    fn the_rate_is_met_however_time_is_sliced() {
        let mut particles = system(1000);
        particles.set_emission_rate(30.0);
        particles.set_particle_lifetime(10.0, 10.0);
        for _ in 0..120 {
            particles.update(1.0 / 60.0);
        }
        // Rounding of the many small steps may hold one particle back.
        assert!(
            (59..=60).contains(&particles.count()),
            "{}",
            particles.count()
        );
    }

    #[test]
    fn particles_are_removed_when_their_life_is_over() {
        let mut particles = system(100);
        particles.set_particle_lifetime(1.0, 1.0);
        particles.emit(5);
        particles.update(0.5);
        assert_eq!(particles.count(), 5);
        particles.update(0.6);
        assert_eq!(particles.count(), 0);
    }

    #[test]
    fn the_system_never_holds_more_than_its_maximum() {
        let mut particles = system(8);
        particles.set_particle_lifetime(10.0, 10.0);
        particles.emit(20);
        assert_eq!(particles.count(), 8);
        particles.set_emission_rate(100.0);
        particles.update(1.0);
        assert_eq!(particles.count(), 8);
    }

    #[test]
    fn the_emitter_stops_after_its_lifetime_and_can_be_started_again() {
        let mut particles = system(1000);
        particles.set_emission_rate(10.0);
        particles.set_particle_lifetime(100.0, 100.0);
        particles.set_emitter_lifetime(Some(1.0));
        particles.update(0.5);
        particles.update(2.0);
        // Half a second, then only the half second that was left.
        assert_eq!(particles.count(), 10);
        assert!(!particles.is_active());

        particles.update(1.0);
        assert_eq!(particles.count(), 10);
        particles.start();
        particles.update(0.5);
        assert_eq!(particles.count(), 15);
    }

    #[test]
    fn stopped_systems_keep_moving_their_particles() {
        let mut particles = system(10);
        particles.set_particle_lifetime(10.0, 10.0);
        particles.set_speed(100.0, 100.0);
        particles.emit(1);
        particles.stop();
        particles.update(0.5);

        let vertices = particles.vertices(&Transform::IDENTITY, Color::WHITE);
        // Moving right at 100 pixels a second: the 8x8 quad is centered
        // on x = 50.
        assert_eq!(vertices.len(), 6);
        assert!((vertices[0].position[0] - 46.0).abs() < 1e-3);
        assert!((vertices[1].position[0] - 54.0).abs() < 1e-3);
    }

    #[test]
    fn size_color_and_tint_follow_the_particles_age() {
        let mut particles = system(10);
        particles.set_particle_lifetime(2.0, 2.0);
        particles.set_sizes(&[1.0, 3.0]);
        particles.set_colors(&[Color::WHITE, Color::rgba(1.0, 1.0, 1.0, 0.0)]);
        particles.emit(1);
        particles.update(1.0);

        let tint = Color::rgb(0.5, 1.0, 1.0);
        let vertices = particles.vertices(&Transform::IDENTITY, tint);
        // Halfway through: twice the size and half faded.
        let width = vertices[1].position[0] - vertices[0].position[0];
        assert!((width - 16.0).abs() < 1e-3);
        assert_eq!(vertices[0].color, [0.5, 1.0, 1.0, 0.5]);
        assert_eq!(vertices[2].uv, [1.0, 1.0]);
    }

    #[test]
    fn gravity_and_damping_change_the_speed() {
        let mut falling = system(1);
        falling.set_particle_lifetime(10.0, 10.0);
        falling.set_linear_acceleration((0.0, 100.0), (0.0, 100.0));
        falling.emit(1);
        falling.update(1.0);
        let top = falling.vertices(&Transform::IDENTITY, Color::WHITE)[0].position[1];
        assert!((top - 96.0).abs() < 1e-3, "{top}");

        let mut slowing = system(1);
        slowing.set_particle_lifetime(10.0, 10.0);
        slowing.set_speed(100.0, 100.0);
        slowing.set_linear_damping(1.0, 1.0);
        slowing.emit(1);
        slowing.update(1.0);
        let left = slowing.vertices(&Transform::IDENTITY, Color::WHITE)[0].position[0];
        // Slower than the undamped 100 pixels, but still moving.
        assert!(left > 0.0 && left < 96.0, "{left}");
    }

    #[test]
    fn stops_are_blended_evenly_across_a_life() {
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        assert_eq!(sample(&[2.0], 0.7, lerp), 2.0);
        assert_eq!(sample(&[0.0, 10.0], 0.25, lerp), 2.5);
        assert_eq!(sample(&[0.0, 1.0, 0.0], 0.5, lerp), 1.0);
        assert_eq!(sample(&[0.0, 1.0, 0.0], 0.75, lerp), 0.5);
        assert_eq!(sample(&[0.0, 1.0, 0.0], 1.0, lerp), 0.0);
        // Out of range values are clamped rather than indexing past the end.
        assert_eq!(sample(&[1.0, 3.0], 7.0, lerp), 3.0);
    }

    #[test]
    fn random_numbers_stay_in_range_and_vary() {
        let mut rng = Rng::new(1);
        let values: Vec<f32> = (0..1000).map(|_| rng.between(-2.0, 3.0)).collect();
        assert!(values.iter().all(|&value| (-2.0..3.0).contains(&value)));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.3, "mean was {mean}");
    }

    #[test]
    fn the_same_seed_gives_the_same_numbers() {
        let (mut a, mut b) = (Rng::new(42), Rng::new(42));
        assert!((0..10).all(|_| a.next() == b.next()));
    }
}
