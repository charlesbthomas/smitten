use glam::Vec2;
use std::f32::consts::TAU;

use smitten_core::backend::Vertex;

use super::{Image, Quad, Transform};
use crate::Color;

/// How new particles are spread over the emission area, the equivalent of
/// LÖVE's `AreaSpreadDistribution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AreaSpreadDistribution {
    /// Every particle starts at the emitter's position.
    #[default]
    None,
    /// Evenly over a rectangle reaching `dx` to each side and `dy` above
    /// and below.
    Uniform,
    /// Around the position in a bell curve whose standard deviation is
    /// `dx` across and `dy` down.
    Normal,
    /// Evenly over an ellipse whose radii are `dx` and `dy`.
    Ellipse,
    /// On the edge of an ellipse whose radii are `dx` and `dy`.
    BorderEllipse,
    /// On the edge of the rectangle that [`Uniform`] fills.
    ///
    /// [`Uniform`]: AreaSpreadDistribution::Uniform
    BorderRectangle,
}

/// Where new particles appear around the emitter, as set by
/// [`ParticleSystem::set_emission_area_ex`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EmissionArea {
    pub distribution: AreaSpreadDistribution,
    /// Half the width of the area, or the spread across for
    /// [`AreaSpreadDistribution::Normal`].
    pub dx: f32,
    /// Half the height of the area, or the spread down for
    /// [`AreaSpreadDistribution::Normal`].
    pub dy: f32,
    /// How far the area is turned around the emitter, in radians.
    pub angle: f32,
    /// Whether each particle's direction is turned to point away from the
    /// emitter, so particles burst outwards from the area's center.
    pub relative_direction: bool,
}

/// Where a new particle goes in the drawing order, the equivalent of
/// LÖVE's `ParticleInsertMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ParticleInsertMode {
    /// Drawn on top of the particles already there.
    #[default]
    Top,
    /// Drawn underneath the particles already there.
    Bottom,
    /// Drawn at a random place among the particles already there.
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Active,
    Paused,
    Stopped,
}

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
    texture: Image,
    max: usize,
    particles: Vec<Particle>,
    rng: Rng,

    state: State,
    /// Seconds the emitter has been running since it was started.
    emitter_age: f32,
    /// How long the emitter runs before stopping, or `None` for forever.
    emitter_lifetime: Option<f32>,
    /// Particles owed but not yet emitted, as a fraction of one.
    owed: f32,

    emission_rate: f32,
    position: (f32, f32),
    /// Where the emitter was at the last update, so particles emitted
    /// while it moves are spread along the way.
    previous_position: (f32, f32),
    area: EmissionArea,
    insert_mode: ParticleInsertMode,
    lifetime: (f32, f32),
    speed: (f32, f32),
    direction: f32,
    spread: f32,
    acceleration: ((f32, f32), (f32, f32)),
    radial: (f32, f32),
    tangential: (f32, f32),
    damping: (f32, f32),
    rotation: (f32, f32),
    relative_rotation: bool,
    spin: (f32, f32),
    spin_variation: f32,
    sizes: Vec<f32>,
    size_variation: f32,
    colors: Vec<Color>,
    quads: Vec<Quad>,
    /// The point of each particle's image it turns and scales around, or
    /// `None` for its center.
    offset: Option<(f32, f32)>,
}

#[derive(Debug, Clone, Copy)]
struct Particle {
    x: f32,
    y: f32,
    /// Where the emitter was when the particle was born, which radial and
    /// tangential acceleration push away from and around.
    origin_x: f32,
    origin_y: f32,
    vx: f32,
    vy: f32,
    ax: f32,
    ay: f32,
    radial: f32,
    tangential: f32,
    damping: f32,
    rotation: f32,
    spin_start: f32,
    spin_end: f32,
    /// Which part of the size stops this particle passes through: it
    /// starts `size_offset` of the way along and covers `size_interval`.
    size_offset: f32,
    size_interval: f32,
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
            texture: image.clone(),
            max: max_particles,
            particles: Vec::with_capacity(max_particles.min(4096)),
            rng: Rng::new(0x5EED_1E55),
            state: State::Active,
            emitter_age: 0.0,
            emitter_lifetime: None,
            owed: 0.0,
            emission_rate: 0.0,
            position: (0.0, 0.0),
            previous_position: (0.0, 0.0),
            area: EmissionArea::default(),
            insert_mode: ParticleInsertMode::Top,
            lifetime: (1.0, 1.0),
            speed: (0.0, 0.0),
            direction: 0.0,
            spread: 0.0,
            acceleration: ((0.0, 0.0), (0.0, 0.0)),
            radial: (0.0, 0.0),
            tangential: (0.0, 0.0),
            damping: (0.0, 0.0),
            rotation: (0.0, 0.0),
            relative_rotation: false,
            spin: (0.0, 0.0),
            spin_variation: 0.0,
            sizes: vec![1.0],
            size_variation: 0.0,
            colors: vec![Color::WHITE],
            quads: Vec::new(),
            offset: None,
        }
    }

    /// Makes a system with the same settings and no particles, in the
    /// stopped state, the equivalent of LÖVE's `ParticleSystem:clone`.
    /// [`Clone`] copies the particles and state as well.
    pub fn clone_stopped(&self) -> ParticleSystem {
        let mut copy = self.clone();
        copy.particles = Vec::with_capacity(self.max.min(4096));
        copy.stop();
        copy.emitter_age = 0.0;
        copy
    }

    // --- What particles look like ---

    /// Returns the image every particle is drawn as.
    pub fn texture(&self) -> &Image {
        &self.texture
    }

    /// Changes the image every particle is drawn as. A canvas works too,
    /// through its image.
    pub fn set_texture(&mut self, image: &Image) {
        self.texture = image.clone();
    }

    /// Sets the parts of the image particles show over their life, such
    /// as the frames of an animation in a sprite sheet. They are shown in
    /// turn, each for an equal share of a particle's life. An empty list
    /// draws the whole image.
    pub fn set_quads(&mut self, quads: &[Quad]) {
        self.quads = quads.to_vec();
    }

    /// Returns the parts of the image particles show over their life.
    pub fn quads(&self) -> &[Quad] {
        &self.quads
    }

    /// Sets the point of a particle's image, in pixels from its top-left
    /// corner, that the particle turns and scales around and that sits on
    /// the particle's position. The default is the image's center, or
    /// each quad's center when quads are set.
    pub fn set_offset(&mut self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.offset = Some((x, y));
    }

    /// Returns the point particles turn and scale around. Until one is
    /// set, this is the center of the image.
    pub fn offset(&self) -> Vec2 {
        self.offset
            .unwrap_or((
                self.texture.width() as f32 / 2.0,
                self.texture.height() as f32 / 2.0,
            ))
            .into()
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

    /// Returns the sizes a particle passes through over its life.
    pub fn sizes(&self) -> &[f32] {
        &self.sizes
    }

    /// Sets how much the sizes vary between particles, from 0.0 to 1.0.
    /// At 0.0 every particle passes through all of
    /// [`set_sizes`](ParticleSystem::set_sizes). Higher values let each
    /// particle start later and end earlier in the list, chosen at random.
    pub fn set_size_variation(&mut self, variation: f32) {
        self.size_variation = variation.clamp(0.0, 1.0);
    }

    /// Returns how much the sizes vary between particles.
    pub fn size_variation(&self) -> f32 {
        self.size_variation
    }

    /// Sets the colors a particle passes through over its life, blended
    /// between evenly. End on a transparent color to fade out.
    ///
    /// Panics if `colors` is empty.
    pub fn set_colors(&mut self, colors: &[Color]) {
        assert!(!colors.is_empty(), "a particle system needs a color");
        self.colors = colors.to_vec();
    }

    /// Returns the colors a particle passes through over its life.
    pub fn colors(&self) -> &[Color] {
        &self.colors
    }

    /// Sets where new particles go in the drawing order.
    pub fn set_insert_mode(&mut self, mode: ParticleInsertMode) {
        self.insert_mode = mode;
    }

    /// Returns where new particles go in the drawing order.
    pub fn insert_mode(&self) -> ParticleInsertMode {
        self.insert_mode
    }

    // --- How particles are emitted ---

    /// Sets how many particles are emitted per second while the system is
    /// active.
    pub fn set_emission_rate(&mut self, per_second: f32) {
        self.emission_rate = per_second.max(0.0);
    }

    /// Returns how many particles are emitted per second.
    pub fn emission_rate(&self) -> f32 {
        self.emission_rate
    }

    /// Sets how long the system keeps emitting after [`start`], in
    /// seconds, or `None` to emit until stopped. Particles already emitted
    /// live on after it ends.
    ///
    /// [`start`]: ParticleSystem::start
    pub fn set_emitter_lifetime(&mut self, seconds: Option<f32>) {
        self.emitter_lifetime = seconds;
    }

    /// Returns how long the system keeps emitting, or `None` for until it
    /// is stopped.
    pub fn emitter_lifetime(&self) -> Option<f32> {
        self.emitter_lifetime
    }

    /// Sets where new particles appear, relative to where the system is
    /// drawn. Move this to leave a trail behind a moving object; see also
    /// [`move_to`](ParticleSystem::move_to).
    pub fn set_position(&mut self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.position = (x, y);
        self.previous_position = (x, y);
    }

    /// Moves where new particles appear, and spreads the particles the
    /// next [`update`](ParticleSystem::update) emits along the way from
    /// the old position, so a fast-moving emitter leaves a smooth trail
    /// rather than clumps.
    pub fn move_to(&mut self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.position = (x, y);
    }

    /// Returns where new particles appear.
    pub fn position(&self) -> Vec2 {
        self.position.into()
    }

    /// Spreads new particles evenly over a box reaching `dx` to each side
    /// of the position and `dy` above and below it.
    pub fn set_emission_area(&mut self, delta: impl Into<Vec2>) {
        let Vec2 { x: dx, y: dy } = delta.into();
        self.set_emission_area_ex(AreaSpreadDistribution::Uniform, (dx, dy), 0.0, false);
    }

    /// Sets the area new particles appear in with the full set of LÖVE's
    /// `setEmissionArea` arguments: its shape, its size, the angle it is
    /// turned by in radians, and whether particles head away from the
    /// emitter (see [`EmissionArea`]).
    pub fn set_emission_area_ex(
        &mut self,
        distribution: AreaSpreadDistribution,
        delta: impl Into<Vec2>,
        angle: f32,
        relative_direction: bool,
    ) {
        let Vec2 { x: dx, y: dy } = delta.into();
        self.area = EmissionArea {
            distribution,
            dx: dx.abs(),
            dy: dy.abs(),
            angle,
            relative_direction,
        };
    }

    /// Returns the area new particles appear in.
    pub fn emission_area(&self) -> EmissionArea {
        self.area
    }

    // --- How each particle behaves ---

    /// Sets how long each particle lives, in seconds. Each gets a random
    /// time between `min` and `max`.
    pub fn set_particle_lifetime(&mut self, min: f32, max: f32) {
        self.lifetime = (min.max(0.0), max.max(0.0));
    }

    /// Returns the shortest and longest life a particle gets.
    pub fn particle_lifetime(&self) -> (f32, f32) {
        self.lifetime
    }

    /// Sets the speed particles start with, in pixels per second. Each
    /// gets a random speed between `min` and `max`.
    pub fn set_speed(&mut self, min: f32, max: f32) {
        self.speed = (min, max);
    }

    /// Returns the slowest and fastest speed particles start with.
    pub fn speed(&self) -> (f32, f32) {
        self.speed
    }

    /// Sets the direction particles start out in, in radians. 0.0 is to
    /// the right and positive angles turn clockwise on screen.
    pub fn set_direction(&mut self, radians: f32) {
        self.direction = radians;
    }

    /// Returns the direction particles start out in.
    pub fn direction(&self) -> f32 {
        self.direction
    }

    /// Sets the width, in radians, of the fan of directions around
    /// [`set_direction`](ParticleSystem::set_direction). A full turn sends
    /// particles every way.
    pub fn set_spread(&mut self, radians: f32) {
        self.spread = radians;
    }

    /// Returns the width of the fan of directions particles start in.
    pub fn spread(&self) -> f32 {
        self.spread
    }

    /// Sets a constant acceleration in pixels per second squared, such as
    /// gravity. Each particle gets a random one between `min` and `max`.
    pub fn set_linear_acceleration(&mut self, min: (f32, f32), max: (f32, f32)) {
        self.acceleration = (min, max);
    }

    /// Returns the range of constant accelerations particles get.
    pub fn linear_acceleration(&self) -> ((f32, f32), (f32, f32)) {
        self.acceleration
    }

    /// Sets an acceleration away from where the emitter was when each
    /// particle was born, in pixels per second squared. Negative values
    /// pull particles in. Each gets a random one between `min` and `max`.
    pub fn set_radial_acceleration(&mut self, min: f32, max: f32) {
        self.radial = (min, max);
    }

    /// Returns the range of accelerations away from the emitter.
    pub fn radial_acceleration(&self) -> (f32, f32) {
        self.radial
    }

    /// Sets an acceleration at right angles to the line from where the
    /// emitter was when each particle was born, which makes particles
    /// swirl around it. Each gets a random one between `min` and `max`.
    pub fn set_tangential_acceleration(&mut self, min: f32, max: f32) {
        self.tangential = (min, max);
    }

    /// Returns the range of accelerations around the emitter.
    pub fn tangential_acceleration(&self) -> (f32, f32) {
        self.tangential
    }

    /// Sets how strongly particles slow down over time. 0.0 is not at
    /// all. Each gets a random amount between `min` and `max`.
    pub fn set_linear_damping(&mut self, min: f32, max: f32) {
        self.damping = (min.max(0.0), max.max(0.0));
    }

    /// Returns the range of damping particles get.
    pub fn linear_damping(&self) -> (f32, f32) {
        self.damping
    }

    /// Sets the angle particles start at, in radians. Each gets a random
    /// angle between `min` and `max`.
    pub fn set_rotation(&mut self, min: f32, max: f32) {
        self.rotation = (min, max);
    }

    /// Returns the range of angles particles start at.
    pub fn rotation(&self) -> (f32, f32) {
        self.rotation
    }

    /// Sets whether particles are also turned to face the way they are
    /// moving, on top of their own rotation.
    pub fn set_relative_rotation(&mut self, enabled: bool) {
        self.relative_rotation = enabled;
    }

    /// Returns whether particles are turned to face the way they move.
    pub fn has_relative_rotation(&self) -> bool {
        self.relative_rotation
    }

    /// Sets how fast particles turn, in radians per second: `start` when
    /// a particle is born, changing evenly to `end` when it dies, as in
    /// LÖVE. Use [`set_spin_variation`](ParticleSystem::set_spin_variation)
    /// to make particles turn at different rates.
    pub fn set_spin(&mut self, start: f32, end: f32) {
        self.spin = (start, end);
    }

    /// Returns how fast particles turn when born and when they die.
    pub fn spin(&self) -> (f32, f32) {
        self.spin
    }

    /// Sets how much the spin varies between particles, from 0.0 (not at
    /// all) to 1.0. At 1.0 a particle's starting spin is anywhere within
    /// half the distance from `start` to `end` of `start`, and its ending
    /// spin likewise around `end`.
    pub fn set_spin_variation(&mut self, variation: f32) {
        self.spin_variation = variation.clamp(0.0, 1.0);
    }

    /// Returns how much the spin varies between particles.
    pub fn spin_variation(&self) -> f32 {
        self.spin_variation
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
            if !self.add(1.0) {
                break;
            }
        }
    }

    /// Starts emitting again from the beginning of the emitter lifetime.
    /// A new system is already started. After a
    /// [`pause`](ParticleSystem::pause), this carries on where it left
    /// off, as in LÖVE.
    pub fn start(&mut self) {
        if self.state != State::Paused {
            self.emitter_age = 0.0;
        }
        self.state = State::Active;
    }

    /// Stops emitting. Particles already emitted live on.
    pub fn stop(&mut self) {
        self.state = State::Stopped;
        self.owed = 0.0;
    }

    /// Stops emitting, like [`stop`](ParticleSystem::stop), but
    /// [`resume`](ParticleSystem::resume) carries on where it left off.
    pub fn pause(&mut self) {
        if self.state == State::Active {
            self.state = State::Paused;
        }
    }

    /// Carries on emitting after a [`pause`](ParticleSystem::pause).
    pub fn resume(&mut self) {
        self.state = State::Active;
    }

    /// Removes every particle and starts the emitter over.
    pub fn reset(&mut self) {
        self.particles.clear();
        self.owed = 0.0;
        self.state = State::Stopped;
        self.start();
    }

    /// Returns whether the system is emitting.
    pub fn is_active(&self) -> bool {
        self.state == State::Active
    }

    /// Returns whether the system is paused, so that
    /// [`resume`](ParticleSystem::resume) carries on emitting.
    pub fn is_paused(&self) -> bool {
        self.state == State::Paused
    }

    /// Returns whether the system is stopped, by
    /// [`stop`](ParticleSystem::stop) or by its emitter lifetime running
    /// out.
    pub fn is_stopped(&self) -> bool {
        self.state == State::Stopped
    }

    /// Returns whether no particles are alive.
    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }

    /// Returns whether the system holds as many particles as it can.
    pub fn is_full(&self) -> bool {
        self.particles.len() >= self.max
    }

    /// Returns how many particles are alive.
    pub fn count(&self) -> usize {
        self.particles.len()
    }

    /// Returns the most particles the system holds at once.
    pub fn max_particles(&self) -> usize {
        self.max
    }

    /// Changes the most particles the system holds at once, the
    /// equivalent of LÖVE's `setBufferSize`. As in LÖVE, this removes
    /// every particle and starts the emitter lifetime over.
    pub fn set_buffer_size(&mut self, max_particles: usize) {
        self.max = max_particles;
        self.particles = Vec::with_capacity(max_particles.min(4096));
        self.owed = 0.0;
        self.emitter_age = 0.0;
    }

    /// Returns the most particles the system holds at once, the
    /// equivalent of LÖVE's `getBufferSize`.
    pub fn buffer_size(&self) -> usize {
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
            let progress = particle.progress();

            // Radial acceleration points away from the particle's origin,
            // and tangential acceleration at right angles to that.
            let (mut rx, mut ry) = (
                particle.x - particle.origin_x,
                particle.y - particle.origin_y,
            );
            let length = (rx * rx + ry * ry).sqrt();
            if length > 0.0 {
                rx /= length;
                ry /= length;
            }
            let ax = particle.ax + rx * particle.radial - ry * particle.tangential;
            let ay = particle.ay + ry * particle.radial + rx * particle.tangential;
            particle.vx += ax * dt;
            particle.vy += ay * dt;
            // Damping removes a fraction of the speed per second.
            let keep = 1.0 / (1.0 + particle.damping * dt);
            particle.vx *= keep;
            particle.vy *= keep;
            particle.x += particle.vx * dt;
            particle.y += particle.vy * dt;

            let spin = particle.spin_start + (particle.spin_end - particle.spin_start) * progress;
            particle.rotation += spin * dt;
        }
        self.particles
            .retain(|particle| particle.age < particle.lifetime);

        if self.state == State::Active {
            // Only emit for the part of this step the emitter was alive for.
            let mut emitting = dt;
            if let Some(lifetime) = self.emitter_lifetime {
                emitting = emitting.min((lifetime - self.emitter_age).max(0.0));
            }
            self.emitter_age += dt;
            self.owed += self.emission_rate * emitting;
            let count = self.owed.floor();
            self.owed -= count;
            // Spread the new particles along the way the emitter moved.
            let count = count as usize;
            for index in 0..count {
                if !self.add((index + 1) as f32 / count as f32) {
                    break;
                }
            }
            if self
                .emitter_lifetime
                .is_some_and(|lifetime| self.emitter_age >= lifetime)
            {
                self.stop();
            }
        }
        self.previous_position = self.position;
    }

    /// Adds one particle, born `along` of the way from where the emitter
    /// was at the last update to where it is now. Returns false if there
    /// is no room.
    fn add(&mut self, along: f32) -> bool {
        if self.particles.len() >= self.max {
            return false;
        }
        let particle = self.spawn(along);
        match self.insert_mode {
            ParticleInsertMode::Top => self.particles.push(particle),
            ParticleInsertMode::Bottom => self.particles.insert(0, particle),
            ParticleInsertMode::Random => {
                let len = self.particles.len();
                let index = ((self.rng.next() * (len + 1) as f32) as usize).min(len);
                self.particles.insert(index, particle);
            }
        }
        true
    }

    fn spawn(&mut self, along: f32) -> Particle {
        let (from, to) = (self.previous_position, self.position);
        let emitter = (
            from.0 + (to.0 - from.0) * along,
            from.1 + (to.1 - from.1) * along,
        );
        let rng = &mut self.rng;
        let (dx, dy) = area_offset(rng, &self.area);
        let (x, y) = (emitter.0 + dx, emitter.1 + dy);

        let mut angle = self.direction + rng.between(-0.5, 0.5) * self.spread;
        if self.area.relative_direction {
            angle += dy.atan2(dx);
        }
        let speed = rng.between(self.speed.0, self.speed.1);
        let (min, max) = self.acceleration;

        let size_offset = rng.between(0.0, self.size_variation);
        let size_interval = (1.0 - rng.between(0.0, self.size_variation)) - size_offset;
        let (spin_start, spin_end) = self.spin;
        Particle {
            x,
            y,
            origin_x: emitter.0,
            origin_y: emitter.1,
            vx: angle.cos() * speed,
            vy: angle.sin() * speed,
            ax: rng.between(min.0, max.0),
            ay: rng.between(min.1, max.1),
            radial: rng.between(self.radial.0, self.radial.1),
            tangential: rng.between(self.tangential.0, self.tangential.1),
            damping: rng.between(self.damping.0, self.damping.1),
            rotation: rng.between(self.rotation.0, self.rotation.1),
            spin_start: vary(rng, spin_start, spin_end, self.spin_variation),
            spin_end: vary(rng, spin_end, spin_start, self.spin_variation),
            size_offset,
            size_interval,
            age: 0.0,
            lifetime: rng.between(self.lifetime.0, self.lifetime.1),
        }
    }

    /// Builds two triangles per particle in drawing order, so later ones
    /// draw on top. `tint` multiplies every particle's color.
    pub(crate) fn vertices(&self, transform: &Transform, tint: Color) -> Vec<Vertex> {
        let (image_width, image_height) =
            (self.texture.width() as f32, self.texture.height() as f32);
        let mut vertices = Vec::with_capacity(self.particles.len() * 6);

        for particle in &self.particles {
            let progress = particle.progress();
            let size_progress = particle.size_offset + progress * particle.size_interval;
            let size = sample(&self.sizes, size_progress, |a, b, t| a + (b - a) * t);
            let color = sample(&self.colors, progress, mix);
            let color = [
                color.r * tint.r,
                color.g * tint.g,
                color.b * tint.b,
                color.a * tint.a,
            ];

            // The frame to show, and the part of the image it covers.
            let (width, height, uv) = match self.quads.len() {
                0 => (image_width, image_height, [0.0, 0.0, 1.0, 1.0]),
                frames => {
                    let index = ((progress * frames as f32) as usize).min(frames - 1);
                    let quad = &self.quads[index];
                    (
                        quad.width,
                        quad.height,
                        [
                            quad.x / quad.sheet_width,
                            quad.y / quad.sheet_height,
                            (quad.x + quad.width) / quad.sheet_width,
                            (quad.y + quad.height) / quad.sheet_height,
                        ],
                    )
                }
            };
            let (offset_x, offset_y) = self.offset.unwrap_or((width / 2.0, height / 2.0));

            let mut angle = particle.rotation;
            if self.relative_rotation {
                angle += particle.vy.atan2(particle.vx);
            }
            let (sin, cos) = angle.sin_cos();
            let corner = |px: f32, py: f32, u: f32, v: f32| {
                let (dx, dy) = ((px - offset_x) * size, (py - offset_y) * size);
                let Vec2 { x, y } = transform.apply((
                    particle.x + dx * cos - dy * sin,
                    particle.y + dx * sin + dy * cos,
                ));
                Vertex {
                    position: [x, y],
                    uv: [u, v],
                    color,
                }
            };
            let [u0, v0, u1, v1] = uv;
            let top_left = corner(0.0, 0.0, u0, v0);
            let top_right = corner(width, 0.0, u1, v0);
            let bottom_right = corner(width, height, u1, v1);
            let bottom_left = corner(0.0, height, u0, v1);
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

/// Picks where in the emission area a particle starts, relative to the
/// emitter.
fn area_offset(rng: &mut Rng, area: &EmissionArea) -> (f32, f32) {
    let (dx, dy) = (area.dx, area.dy);
    let (x, y) = match area.distribution {
        AreaSpreadDistribution::None => return (0.0, 0.0),
        AreaSpreadDistribution::Uniform => (rng.between(-dx, dx), rng.between(-dy, dy)),
        AreaSpreadDistribution::Normal => (rng.normal() * dx, rng.normal() * dy),
        AreaSpreadDistribution::Ellipse => {
            // Maps a point in the unit square onto the unit disc, as LÖVE
            // does.
            let (u, v) = (rng.between(-1.0, 1.0), rng.between(-1.0, 1.0));
            (
                dx * u * (1.0 - 0.5 * v * v).sqrt(),
                dy * v * (1.0 - 0.5 * u * u).sqrt(),
            )
        }
        AreaSpreadDistribution::BorderEllipse => {
            let angle = rng.between(0.0, TAU);
            (dx * angle.cos(), dy * angle.sin())
        }
        AreaSpreadDistribution::BorderRectangle => {
            // A distance around the edge: top, bottom, left, then right.
            let along = rng.between(0.0, 4.0 * (dx + dy));
            if along < 2.0 * dx {
                (along - dx, -dy)
            } else if along < 4.0 * dx {
                (along - 3.0 * dx, dy)
            } else if along < 4.0 * dx + 2.0 * dy {
                (-dx, along - 4.0 * dx - dy)
            } else {
                (dx, along - 4.0 * dx - 3.0 * dy)
            }
        }
    };
    if area.angle == 0.0 {
        return (x, y);
    }
    let (sin, cos) = area.angle.sin_cos();
    (x * cos - y * sin, x * sin + y * cos)
}

/// A random value around `inner`, within `variation` of half the distance
/// to `outer` on either side, the way LÖVE varies spin.
fn vary(rng: &mut Rng, inner: f32, outer: f32, variation: f32) -> f32 {
    let reach = (outer - inner) / 2.0 * variation;
    rng.between(inner - reach, inner + reach)
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

    /// A number from the standard normal distribution (Box-Muller).
    fn normal(&mut self) -> f32 {
        // 1.0 - next() is never zero, so the logarithm is finite.
        let radius = (-2.0 * (1.0 - self.next()).ln()).sqrt();
        radius * (TAU * self.next()).cos()
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

    #[test]
    fn getters_return_what_was_set() {
        let mut particles = system(10);
        particles.set_emission_rate(5.0);
        particles.set_emitter_lifetime(Some(2.0));
        particles.set_particle_lifetime(1.0, 2.0);
        particles.set_speed(3.0, 4.0);
        particles.set_direction(0.5);
        particles.set_spread(0.25);
        particles.set_linear_acceleration((1.0, 2.0), (3.0, 4.0));
        particles.set_radial_acceleration(-1.0, 1.0);
        particles.set_tangential_acceleration(2.0, 3.0);
        particles.set_linear_damping(0.1, 0.2);
        particles.set_rotation(0.0, 1.0);
        particles.set_spin(1.0, 2.0);
        particles.set_spin_variation(0.5);
        particles.set_sizes(&[1.0, 2.0]);
        particles.set_size_variation(2.0);
        particles.set_colors(&[Color::WHITE]);
        particles.set_relative_rotation(true);
        particles.set_insert_mode(ParticleInsertMode::Random);
        particles.set_emission_area_ex(AreaSpreadDistribution::Ellipse, (-3.0, 4.0), 1.0, true);

        assert_eq!(particles.emission_rate(), 5.0);
        assert_eq!(particles.emitter_lifetime(), Some(2.0));
        assert_eq!(particles.particle_lifetime(), (1.0, 2.0));
        assert_eq!(particles.speed(), (3.0, 4.0));
        assert_eq!(particles.direction(), 0.5);
        assert_eq!(particles.spread(), 0.25);
        assert_eq!(particles.linear_acceleration(), ((1.0, 2.0), (3.0, 4.0)));
        assert_eq!(particles.radial_acceleration(), (-1.0, 1.0));
        assert_eq!(particles.tangential_acceleration(), (2.0, 3.0));
        assert_eq!(particles.linear_damping(), (0.1, 0.2));
        assert_eq!(particles.rotation(), (0.0, 1.0));
        assert_eq!(particles.spin(), (1.0, 2.0));
        assert_eq!(particles.spin_variation(), 0.5);
        assert_eq!(particles.sizes(), [1.0, 2.0]);
        // Variations are clamped to 0.0..=1.0.
        assert_eq!(particles.size_variation(), 1.0);
        assert_eq!(particles.colors(), [Color::WHITE]);
        assert!(particles.has_relative_rotation());
        assert_eq!(particles.insert_mode(), ParticleInsertMode::Random);
        assert_eq!(
            particles.emission_area(),
            EmissionArea {
                distribution: AreaSpreadDistribution::Ellipse,
                dx: 3.0,
                dy: 4.0,
                angle: 1.0,
                relative_direction: true,
            }
        );
        assert_eq!(particles.offset(), Vec2::new(4.0, 4.0));
        particles.set_offset((1.0, 2.0));
        assert_eq!(particles.offset(), Vec2::new(1.0, 2.0));
        assert_eq!(particles.buffer_size(), 10);
    }

    #[test]
    fn pausing_and_stopping_are_told_apart() {
        let mut particles = system(10);
        assert!(particles.is_active() && !particles.is_paused() && !particles.is_stopped());
        particles.pause();
        assert!(particles.is_paused() && !particles.is_active());
        particles.start();
        assert!(particles.is_active());
        particles.stop();
        assert!(particles.is_stopped() && !particles.is_paused());
        // Pausing a stopped system leaves it stopped.
        particles.pause();
        assert!(particles.is_stopped());
    }

    #[test]
    fn starting_after_a_pause_keeps_the_emitter_age() {
        let mut particles = system(1000);
        particles.set_emission_rate(10.0);
        particles.set_particle_lifetime(100.0, 100.0);
        particles.set_emitter_lifetime(Some(1.0));
        particles.update(0.5);
        particles.pause();
        particles.start();
        particles.update(1.0);
        // Only the half second left of the emitter's life emitted.
        assert_eq!(particles.count(), 10);
        assert!(particles.is_stopped());
    }

    #[test]
    fn a_stopped_clone_has_the_settings_but_no_particles() {
        let mut particles = system(10);
        particles.set_particle_lifetime(5.0, 5.0);
        particles.set_speed(7.0, 7.0);
        particles.emit(3);
        let copy = particles.clone_stopped();
        assert_eq!(copy.count(), 0);
        assert!(copy.is_stopped());
        assert_eq!(copy.speed(), (7.0, 7.0));
        assert_eq!(particles.count(), 3);
    }

    #[test]
    fn changing_the_buffer_size_empties_the_system() {
        let mut particles = system(10);
        particles.set_particle_lifetime(5.0, 5.0);
        particles.emit(5);
        particles.set_buffer_size(3);
        assert_eq!(particles.count(), 0);
        particles.emit(5);
        assert_eq!(particles.count(), 3);
        assert!(particles.is_full());
    }

    #[test]
    fn insert_mode_decides_the_drawing_order() {
        let mut particles = system(10);
        particles.set_particle_lifetime(5.0, 5.0);
        particles.set_insert_mode(ParticleInsertMode::Bottom);
        particles.set_position((0.0, 0.0));
        particles.emit(1);
        particles.set_position((100.0, 0.0));
        particles.emit(1);
        // The newer particle at x = 100 is drawn first, underneath.
        let vertices = particles.vertices(&Transform::IDENTITY, Color::WHITE);
        assert!((vertices[0].position[0] - 96.0).abs() < 1e-3);
        assert!((vertices[6].position[0] + 4.0).abs() < 1e-3);
    }

    #[test]
    fn move_to_spreads_new_particles_along_the_way() {
        let mut particles = system(100);
        particles.set_particle_lifetime(5.0, 5.0);
        particles.set_emission_rate(4.0);
        particles.move_to((100.0, 0.0));
        particles.update(1.0);
        let vertices = particles.vertices(&Transform::IDENTITY, Color::WHITE);
        let centers: Vec<f32> = vertices
            .chunks(6)
            .map(|quad| (quad[0].position[0] + quad[2].position[0]) / 2.0)
            .collect();
        assert_eq!(centers.len(), 4);
        for (center, expected) in centers.iter().zip([25.0, 50.0, 75.0, 100.0]) {
            assert!((center - expected).abs() < 1e-3, "{centers:?}");
        }
        // Once it has caught up, new particles start at the new position.
        particles.emit(1);
        assert!((particles.particles[4].x - 100.0).abs() < 1e-3);
    }

    #[test]
    fn every_distribution_stays_within_its_area() {
        use AreaSpreadDistribution::*;
        let mut rng = Rng::new(7);
        for distribution in [Uniform, Ellipse, BorderEllipse, BorderRectangle] {
            let area = EmissionArea {
                distribution,
                dx: 10.0,
                dy: 5.0,
                ..EmissionArea::default()
            };
            for _ in 0..500 {
                let (x, y) = area_offset(&mut rng, &area);
                assert!(
                    x.abs() <= 10.0 + 1e-3 && y.abs() <= 5.0 + 1e-3,
                    "{distribution:?} {x} {y}"
                );
                match distribution {
                    Ellipse => assert!((x / 10.0).powi(2) + (y / 5.0).powi(2) <= 1.0 + 1e-3),
                    BorderEllipse => {
                        assert!(((x / 10.0).powi(2) + (y / 5.0).powi(2) - 1.0).abs() < 1e-3)
                    }
                    BorderRectangle => {
                        assert!((x.abs() - 10.0).abs() < 1e-3 || (y.abs() - 5.0).abs() < 1e-3)
                    }
                    _ => {}
                }
            }
        }
        let none = EmissionArea {
            dx: 10.0,
            ..EmissionArea::default()
        };
        assert_eq!(area_offset(&mut rng, &none), (0.0, 0.0));
    }

    #[test]
    fn a_normal_spread_clusters_around_the_emitter() {
        let mut rng = Rng::new(3);
        let area = EmissionArea {
            distribution: AreaSpreadDistribution::Normal,
            dx: 2.0,
            dy: 2.0,
            ..EmissionArea::default()
        };
        let samples: Vec<f32> = (0..4000).map(|_| area_offset(&mut rng, &area).0).collect();
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        let variance =
            samples.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / samples.len() as f32;
        assert!(mean.abs() < 0.2, "{mean}");
        assert!((variance.sqrt() - 2.0).abs() < 0.2, "{variance}");
    }

    #[test]
    fn the_area_angle_turns_the_area() {
        let mut rng = Rng::new(1);
        let area = EmissionArea {
            distribution: AreaSpreadDistribution::BorderRectangle,
            dx: 10.0,
            dy: 0.0,
            angle: std::f32::consts::FRAC_PI_2,
            relative_direction: false,
        };
        // A flat line across, turned a quarter, runs down instead.
        for _ in 0..50 {
            let (x, y) = area_offset(&mut rng, &area);
            assert!(x.abs() < 1e-3 && y.abs() <= 10.0 + 1e-3);
        }
    }

    #[test]
    fn a_relative_direction_sends_particles_away_from_the_emitter() {
        let mut particles = system(100);
        particles.set_particle_lifetime(5.0, 5.0);
        particles.set_speed(10.0, 10.0);
        particles.set_emission_area_ex(
            AreaSpreadDistribution::BorderEllipse,
            (5.0, 5.0),
            0.0,
            true,
        );
        particles.emit(50);
        for particle in &particles.particles {
            // Moving the same way as the particle lies from the center.
            assert!(particle.x * particle.vx + particle.y * particle.vy > 0.0);
        }
    }

    #[test]
    fn radial_acceleration_pushes_away_and_tangential_swirls() {
        let mut particles = system(1);
        particles.set_particle_lifetime(5.0, 5.0);
        particles.set_emission_area_ex(
            AreaSpreadDistribution::BorderRectangle,
            (1.0, 0.0),
            0.0,
            false,
        );
        particles.set_radial_acceleration(100.0, 100.0);
        particles.emit(1);
        let start = particles.particles[0].x;
        particles.update(0.5);
        let end = particles.particles[0].x;
        assert!(end.abs() > start.abs() && end.signum() == start.signum());

        let mut swirling = system(1);
        swirling.set_particle_lifetime(5.0, 5.0);
        swirling.set_emission_area_ex(
            AreaSpreadDistribution::BorderRectangle,
            (1.0, 0.0),
            0.0,
            false,
        );
        swirling.set_tangential_acceleration(100.0, 100.0);
        swirling.emit(1);
        let x = swirling.particles[0].x;
        swirling.update(0.1);
        // At right angles to the line out: (-y, x) is straight down for
        // a particle to the right.
        assert!(swirling.particles[0].vy * x.signum() > 0.0);
        assert!(swirling.particles[0].vx.abs() < 1e-3);
    }

    #[test]
    fn spin_changes_from_start_to_end_over_a_life() {
        let mut particles = system(1);
        particles.set_particle_lifetime(2.0, 2.0);
        particles.set_spin(0.0, 2.0);
        particles.emit(1);
        for _ in 0..200 {
            particles.update(0.01);
        }
        // The rate rises evenly from 0 to 2 over two seconds: 2 radians.
        let rotation = particles.particles[0].rotation;
        assert!((rotation - 2.0).abs() < 0.05, "{rotation}");
    }

    #[test]
    fn spin_variation_spreads_the_rates() {
        let mut rng = Rng::new(5);
        let values: Vec<f32> = (0..200).map(|_| vary(&mut rng, 1.0, 3.0, 1.0)).collect();
        assert!(values.iter().all(|value| (0.0..=2.0).contains(value)));
        assert!(values.iter().any(|&value| value < 0.5) && values.iter().any(|&value| value > 1.5));
        assert_eq!(vary(&mut rng, 1.0, 3.0, 0.0), 1.0);
    }

    #[test]
    fn size_variation_narrows_the_sizes_a_particle_passes() {
        let mut particles = system(200);
        particles.set_particle_lifetime(1.0, 1.0);
        particles.set_size_variation(1.0);
        particles.emit(200);
        for particle in &particles.particles {
            let (first, last) = (
                particle.size_offset,
                particle.size_offset + particle.size_interval,
            );
            assert!((0.0..=1.0).contains(&first) && last <= 1.0);
        }
        let mut plain = system(1);
        plain.emit(1);
        assert_eq!(
            (
                plain.particles[0].size_offset,
                plain.particles[0].size_interval
            ),
            (0.0, 1.0)
        );
    }

    #[test]
    fn quads_animate_over_a_life_and_are_centered() {
        let mut particles = system(1);
        particles.set_particle_lifetime(2.0, 2.0);
        // Two 4x2 frames side by side in the 8x8 image.
        particles.set_quads(&[
            Quad::new((0.0, 0.0), (4.0, 2.0), (8.0, 8.0)),
            Quad::new((4.0, 0.0), (4.0, 2.0), (8.0, 8.0)),
        ]);
        particles.emit(1);
        let first = particles.vertices(&Transform::IDENTITY, Color::WHITE);
        assert_eq!(first[0].uv, [0.0, 0.0]);
        assert_eq!(first[2].uv, [0.5, 0.25]);
        assert_eq!(first[0].position, [-2.0, -1.0]);
        particles.update(1.5);
        let second = particles.vertices(&Transform::IDENTITY, Color::WHITE);
        assert_eq!(second[0].uv, [0.5, 0.0]);
        assert_eq!(second[2].uv, [1.0, 0.25]);
    }

    #[test]
    fn the_offset_is_the_point_on_the_particle() {
        let mut particles = system(1);
        particles.set_particle_lifetime(2.0, 2.0);
        particles.set_offset((0.0, 0.0));
        particles.emit(1);
        let vertices = particles.vertices(&Transform::IDENTITY, Color::WHITE);
        assert_eq!(vertices[0].position, [0.0, 0.0]);
        assert_eq!(vertices[2].position, [8.0, 8.0]);
    }

    #[test]
    fn relative_rotation_faces_the_way_of_travel() {
        let mut particles = system(1);
        particles.set_particle_lifetime(2.0, 2.0);
        particles.set_speed(10.0, 10.0);
        particles.set_direction(std::f32::consts::FRAC_PI_2);
        particles.set_relative_rotation(true);
        particles.emit(1);
        let vertices = particles.vertices(&Transform::IDENTITY, Color::WHITE);
        // Turned a quarter: the top-left corner is now at the top right.
        assert!((vertices[0].position[0] - 4.0).abs() < 1e-3);
        assert!((vertices[0].position[1] + 4.0).abs() < 1e-3);
    }
}
