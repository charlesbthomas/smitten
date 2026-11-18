//! Random numbers, noise and a little geometry, the equivalent of
//! `love.math`.
//!
//! Unlike the other modules, everything here also works when no game is
//! running, which makes it usable in tests and tools.
//!
//! ```
//! use smitten::math;
//!
//! math::set_random_seed(42);
//! let roll = math::random_int(1, 6);
//! assert!((1..=6).contains(&roll));
//! let height = math::noise2(0.3, 1.7);
//! assert!((0.0..=1.0).contains(&height));
//! ```

use std::cell::RefCell;

use crate::Color;

/// A source of random numbers with its own seed, the equivalent of LÖVE's
/// `RandomGenerator`.
///
/// The functions in this module, such as [`random`], share one generator.
/// Make your own when something must be repeatable no matter what else
/// asks for random numbers, such as a level built from a seed.
///
/// The same seed always gives the same numbers, on every platform and in
/// every version of Smitten. The numbers are not suitable for
/// cryptography.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomGenerator {
    seed: u64,
    state: [u64; 4],
}

impl RandomGenerator {
    /// Creates a generator that produces the sequence belonging to `seed`.
    pub fn new(seed: u64) -> RandomGenerator {
        // Spread the seed over the whole state with SplitMix64, as the
        // authors of xoshiro recommend. It never yields an all-zero state.
        let mut mix = seed;
        let mut next = || {
            mix = mix.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = mix;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        RandomGenerator {
            seed,
            state: [next(), next(), next(), next()],
        }
    }

    /// The seed the generator was last given.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Starts the sequence belonging to `seed` from its beginning.
    pub fn set_seed(&mut self, seed: u64) {
        *self = RandomGenerator::new(seed);
    }

    /// The generator's current place in its sequence, like LÖVE's
    /// `RandomGenerator:getState`. Give it to [`set_state`](Self::set_state)
    /// later, on this generator or another, to carry on from here: a saved
    /// game can then replay the same numbers.
    ///
    /// The state is a string of hexadecimal digits, so it can be written
    /// to a save file as it is. Like LÖVE's, it is meant to be passed back
    /// unchanged, not read.
    pub fn state(&self) -> String {
        let mut text = format!("0x{:016x}", self.seed);
        for word in self.state {
            text.push_str(&format!(":{word:016x}"));
        }
        text
    }

    /// Carries on from a place saved by [`state`](Self::state). Returns
    /// [`InvalidRandomState`] and changes nothing if `state` did not come
    /// from [`state`](Self::state).
    pub fn set_state(&mut self, state: &str) -> Result<(), InvalidRandomState> {
        let digits = state.strip_prefix("0x").ok_or(InvalidRandomState)?;
        let words: Vec<&str> = digits.split(':').collect();
        if words.len() != 5 {
            return Err(InvalidRandomState);
        }
        let mut parsed = [0u64; 5];
        for (slot, word) in parsed.iter_mut().zip(words) {
            if word.len() != 16 {
                return Err(InvalidRandomState);
            }
            *slot = u64::from_str_radix(word, 16).map_err(|_| InvalidRandomState)?;
        }
        let [seed, a, b, c, d] = parsed;
        let state = [a, b, c, d];
        // xoshiro never reaches an all-zero state and would stay stuck there.
        if state == [0; 4] {
            return Err(InvalidRandomState);
        }
        *self = RandomGenerator { seed, state };
        Ok(())
    }

    /// The next 64 random bits (xoshiro256++).
    pub fn next_u64(&mut self) -> u64 {
        let state = &mut self.state;
        let result = state[0]
            .wrapping_add(state[3])
            .rotate_left(23)
            .wrapping_add(state[0]);
        let shifted = state[1] << 17;
        state[2] ^= state[0];
        state[3] ^= state[1];
        state[1] ^= state[2];
        state[0] ^= state[3];
        state[2] ^= shifted;
        state[3] = state[3].rotate_left(45);
        result
    }

    /// A number from 0.0 up to, but not including, 1.0.
    pub fn random(&mut self) -> f32 {
        // An f32 holds 24 bits exactly, so use the top 24.
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// A number from `min` up to, but not including, `max`.
    pub fn random_range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.random()
    }

    /// A whole number from `min` to `max`, including both, like LÖVE's
    /// `random(min, max)`. The two may be given in either order.
    pub fn random_int(&mut self, min: i64, max: i64) -> i64 {
        let (min, max) = if min <= max { (min, max) } else { (max, min) };
        let choices = (max as i128 - min as i128 + 1) as u128;
        let pick = (self.next_u64() as u128 * choices) >> 64;
        (min as i128 + pick as i128) as i64
    }

    /// A number from a bell curve centered on `mean`, where about two
    /// thirds of the results fall within `deviation` of it.
    pub fn random_normal(&mut self, deviation: f32, mean: f32) -> f32 {
        // Box-Muller. Flip the first number so it is never zero.
        let a = 1.0 - self.random();
        let b = self.random();
        let unit = (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos();
        mean + deviation * unit
    }

    /// `true` with the given probability: 0.25 is one time in four.
    pub fn chance(&mut self, probability: f32) -> bool {
        self.random() < probability
    }

    /// One of `items`, each equally likely, or `None` if there are none.
    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            return None;
        }
        let index = self.random_int(0, items.len() as i64 - 1) as usize;
        items.get(index)
    }

    /// Puts `items` in a random order.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for last in (1..items.len()).rev() {
            let other = self.random_int(0, last as i64) as usize;
            items.swap(last, other);
        }
    }
}

/// The error from [`RandomGenerator::set_state`] and [`set_random_state`]
/// when the text is not a state that [`RandomGenerator::state`] made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRandomState;

impl std::fmt::Display for InvalidRandomState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("not a random generator state")
    }
}

impl std::error::Error for InvalidRandomState {}

thread_local! {
    static SHARED: RefCell<RandomGenerator> = RefCell::new(RandomGenerator::new(time_seed()));
}

/// A seed that differs from run to run, so games are not the same every
/// time unless they ask to be.
fn time_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0)
}

fn shared<R>(use_it: impl FnOnce(&mut RandomGenerator) -> R) -> R {
    SHARED.with(|generator| use_it(&mut generator.borrow_mut()))
}

/// Restarts the shared generator with `seed`, so the numbers that follow
/// are the same every time. Without this, each run of the game differs.
pub fn set_random_seed(seed: u64) {
    shared(|generator| generator.set_seed(seed));
}

/// The seed the shared generator was last given.
pub fn random_seed() -> u64 {
    shared(|generator| generator.seed())
}

/// The shared generator's current place in its sequence. See
/// [`RandomGenerator::state`].
pub fn random_state() -> String {
    shared(|generator| generator.state())
}

/// Carries the shared generator on from a place saved by
/// [`random_state`]. See [`RandomGenerator::set_state`].
pub fn set_random_state(state: &str) -> Result<(), InvalidRandomState> {
    shared(|generator| generator.set_state(state))
}

/// A number from 0.0 up to, but not including, 1.0.
pub fn random() -> f32 {
    shared(RandomGenerator::random)
}

/// A number from `min` up to, but not including, `max`.
pub fn random_range(min: f32, max: f32) -> f32 {
    shared(|generator| generator.random_range(min, max))
}

/// A whole number from `min` to `max`, including both.
pub fn random_int(min: i64, max: i64) -> i64 {
    shared(|generator| generator.random_int(min, max))
}

/// A number from a bell curve centered on `mean`. See
/// [`RandomGenerator::random_normal`].
pub fn random_normal(deviation: f32, mean: f32) -> f32 {
    shared(|generator| generator.random_normal(deviation, mean))
}

/// `true` with the given probability: 0.25 is one time in four.
pub fn chance(probability: f32) -> bool {
    shared(|generator| generator.chance(probability))
}

/// One of `items`, each equally likely, or `None` if there are none.
pub fn choose<T>(items: &[T]) -> Option<&T> {
    shared(|generator| generator.choose(items))
}

/// Puts `items` in a random order.
pub fn shuffle<T>(items: &mut [T]) {
    shared(|generator| generator.shuffle(items));
}

// ---------------------------------------------------------------------------
// Noise

/// Smooth noise along a line, from 0.0 to 1.0, like `love.math.noise(x)`.
///
/// Nearby inputs give nearby outputs, and the same input always gives the
/// same output. The pattern has features about one unit across, so scale
/// the input to taste: `noise(time * 0.5)` drifts slowly. At whole
/// numbers the result is always 0.5.
pub fn noise(x: f32) -> f32 {
    noise3(x, 0.0, 0.0)
}

/// Smooth noise over a plane, from 0.0 to 1.0. See [`noise`].
pub fn noise2(x: f32, y: f32) -> f32 {
    noise3(x, y, 0.0)
}

/// Smooth noise through space, from 0.0 to 1.0. See [`noise`]. Using time
/// as the third input animates a 2D pattern.
pub fn noise3(x: f32, y: f32, z: f32) -> f32 {
    // Ken Perlin's improved noise, with a hash in place of his table.
    let (floor_x, floor_y, floor_z) = (x.floor(), y.floor(), z.floor());
    let (cell_x, cell_y, cell_z) = (floor_x as i32, floor_y as i32, floor_z as i32);
    let (x, y, z) = (x - floor_x, y - floor_y, z - floor_z);
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v, w) = (fade(x), fade(y), fade(z));
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let corner = |dx: i32, dy: i32, dz: i32| {
        let hash = hash3(
            cell_x.wrapping_add(dx),
            cell_y.wrapping_add(dy),
            cell_z.wrapping_add(dz),
        );
        gradient(hash, x - dx as f32, y - dy as f32, z - dz as f32)
    };

    let near = mix(
        mix(corner(0, 0, 0), corner(1, 0, 0), u),
        mix(corner(0, 1, 0), corner(1, 1, 0), u),
        v,
    );
    let far = mix(
        mix(corner(0, 0, 1), corner(1, 0, 1), u),
        mix(corner(0, 1, 1), corner(1, 1, 1), u),
        v,
    );
    (0.5 + 0.5 * mix(near, far, w)).clamp(0.0, 1.0)
}

/// Smooth noise through four dimensions, from 0.0 to 1.0, like
/// `love.math.noise(x, y, z, w)`. See [`noise`]. Using time as the fourth
/// input animates a 3D pattern.
pub fn noise4(x: f32, y: f32, z: f32, w: f32) -> f32 {
    let floors = [x.floor(), y.floor(), z.floor(), w.floor()];
    let cells = floors.map(|floor| floor as i32);
    let offsets = [x - floors[0], y - floors[1], z - floors[2], w - floors[3]];
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;

    // The sixteen corners of the cell, numbered by their bits: bit 0 is
    // one step along x, bit 1 along y, and so on.
    let mut values = [0.0f32; 16];
    for (corner, value) in values.iter_mut().enumerate() {
        let step = |axis: usize| ((corner >> axis) & 1) as i32;
        let hash = hash4(
            cells[0].wrapping_add(step(0)),
            cells[1].wrapping_add(step(1)),
            cells[2].wrapping_add(step(2)),
            cells[3].wrapping_add(step(3)),
        );
        let local = [0, 1, 2, 3].map(|axis| offsets[axis] - step(axis) as f32);
        *value = gradient4(hash, local);
    }
    // Blend neighboring corners, one axis at a time, x first.
    let mut count = 16;
    for offset in offsets {
        count /= 2;
        for index in 0..count {
            values[index] = mix(values[2 * index], values[2 * index + 1], fade(offset));
        }
    }
    (0.5 + 0.5 * values[0]).clamp(0.0, 1.0)
}

fn hash4(x: i32, y: i32, z: i32, w: i32) -> u32 {
    let mixed = hash3(x, y, z) ^ (w as u32).wrapping_mul(0x9E37_79B1);
    let mixed = mixed.wrapping_mul(0x2C1B_3C6D);
    mixed ^ (mixed >> 15)
}

/// The dot product of `offset` with one of the 32 gradient directions of
/// 4D Perlin noise, picked by `hash`. Each direction leaves one axis out
/// and points one way or the other along the rest.
fn gradient4(hash: u32, offset: [f32; 4]) -> f32 {
    let pick = (hash & 31) as usize;
    let skip = pick >> 3;
    let mut total = 0.0;
    let mut sign_bit = 0;
    for (axis, value) in offset.into_iter().enumerate() {
        if axis == skip {
            continue;
        }
        let sign = if pick & (1 << sign_bit) == 0 {
            1.0
        } else {
            -1.0
        };
        total += sign * value;
        sign_bit += 1;
    }
    // Three axes reach further than the 3D gradients' two; scale back to
    // about the same range.
    total * (2.0 / 3.0)
}

fn hash3(x: i32, y: i32, z: i32) -> u32 {
    let mut hash = (x as u32).wrapping_mul(0x8DA6_B343)
        ^ (y as u32).wrapping_mul(0xD816_3841)
        ^ (z as u32).wrapping_mul(0xCB1A_B31F);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x2C1B_3C6D);
    hash ^= hash >> 12;
    hash = hash.wrapping_mul(0x297A_2D39);
    hash ^ (hash >> 15)
}

/// The dot product of `(x, y, z)` with one of Perlin's twelve gradient
/// directions, picked by `hash`.
fn gradient(hash: u32, x: f32, y: f32, z: f32) -> f32 {
    let pick = hash & 15;
    let a = if pick < 8 { x } else { y };
    let b = if pick < 4 {
        y
    } else if pick == 12 || pick == 14 {
        x
    } else {
        z
    };
    let a = if pick & 1 == 0 { a } else { -a };
    let b = if pick & 2 == 0 { b } else { -b };
    a + b
}

// ---------------------------------------------------------------------------
// Geometry and color

/// Whether a polygon has no dents: every corner turns the same way.
/// Polygons with fewer than three corners are not convex.
pub fn is_convex(points: &[(f32, f32)]) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut direction = 0.0f32;
    for index in 0..points.len() {
        let turn = cross(
            points[index],
            points[(index + 1) % points.len()],
            points[(index + 2) % points.len()],
        );
        if turn != 0.0 {
            if direction != 0.0 && turn.signum() != direction {
                return false;
            }
            direction = turn.signum();
        }
    }
    direction != 0.0
}

/// Splits a polygon into triangles, like `love.math.triangulate`. The
/// polygon may have dents but its edges must not cross each other.
///
/// Returns nothing for fewer than three corners, or if the polygon is so
/// tangled that no triangle can be cut from it.
pub fn triangulate(points: &[(f32, f32)]) -> Vec<[(f32, f32); 3]> {
    if points.len() < 3 {
        return Vec::new();
    }
    // Ear clipping wants the corners in one known direction.
    let doubled_area: f32 = (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    let mut remaining: Vec<(f32, f32)> = points.to_vec();
    if doubled_area < 0.0 {
        remaining.reverse();
    }

    let mut triangles = Vec::with_capacity(points.len() - 2);
    while remaining.len() > 3 {
        let count = remaining.len();
        let ear = (0..count).find(|&index| {
            let previous = remaining[(index + count - 1) % count];
            let tip = remaining[index];
            let next = remaining[(index + 1) % count];
            // An ear is a corner that turns the polygon's way and has no
            // other corner inside its triangle.
            cross(previous, tip, next) > 0.0
                && !remaining.iter().any(|&point| {
                    point != previous
                        && point != tip
                        && point != next
                        && inside_triangle(point, previous, tip, next)
                })
        });
        let Some(index) = ear else {
            // Only corners in a straight line, or crossing edges, are left.
            return triangles;
        };
        triangles.push([
            remaining[(index + count - 1) % count],
            remaining[index],
            remaining[(index + 1) % count],
        ]);
        remaining.remove(index);
    }
    triangles.push([remaining[0], remaining[1], remaining[2]]);
    triangles
}

/// Positive when going from `a` to `b` to `c` turns one way, negative for
/// the other, and zero when they are in a line.
fn cross(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0)
}

fn inside_triangle(point: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    cross(a, b, point) >= 0.0 && cross(b, c, point) >= 0.0 && cross(c, a, point) >= 0.0
}

/// Turns color components from 0 to 255, as image editors show them, into
/// a [`Color`] with components from 0.0 to 1.0, like
/// `love.math.colorFromBytes`.
pub fn color_from_bytes(r: u8, g: u8, b: u8, a: u8) -> Color {
    let unit = |byte: u8| byte as f32 / 255.0;
    Color::rgba(unit(r), unit(g), unit(b), unit(a))
}

/// Turns a [`Color`] into components from 0 to 255, like
/// `love.math.colorToBytes`. Components outside 0.0 to 1.0 are cut to
/// that range, and each is rounded to the nearest whole number.
pub fn color_to_bytes(color: Color) -> [u8; 4] {
    let byte = |unit: f32| (unit.clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte(color.r), byte(color.g), byte(color.b), byte(color.a)]
}

/// Converts one color component from the usual sRGB encoding to linear
/// light, where adding and averaging behave physically.
pub fn gamma_to_linear(component: f32) -> f32 {
    if component <= 0.04045 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

/// Converts one color component from linear light back to sRGB.
pub fn linear_to_gamma(component: f32) -> f32 {
    if component <= 0.003_130_8 {
        component * 12.92
    } else {
        1.055 * component.powf(1.0 / 2.4) - 0.055
    }
}

// ---------------------------------------------------------------------------
// Curves

/// A Bezier curve of any degree, the equivalent of LÖVE's `BezierCurve`.
///
/// The curve starts at its first control point, ends at its last, and is
/// pulled towards the ones between. Two points make a straight line, three
/// a quadratic curve, four a cubic one. Draw it by passing
/// [`BezierCurve::render`] to a line-drawing function.
#[derive(Debug, Clone, PartialEq)]
pub struct BezierCurve {
    points: Vec<(f32, f32)>,
}

impl BezierCurve {
    /// Creates a curve from its control points.
    ///
    /// Panics if there are fewer than two, since that is not a curve.
    pub fn new(points: &[(f32, f32)]) -> BezierCurve {
        assert!(
            points.len() >= 2,
            "a Bezier curve needs at least 2 control points, got {}",
            points.len()
        );
        BezierCurve {
            points: points.to_vec(),
        }
    }

    /// One less than the number of control points.
    pub fn degree(&self) -> usize {
        self.points.len() - 1
    }

    /// The control points in order.
    pub fn control_points(&self) -> &[(f32, f32)] {
        &self.points
    }

    /// The control point at `index`, counting from zero.
    pub fn control_point(&self, index: usize) -> Option<(f32, f32)> {
        self.points.get(index).copied()
    }

    /// Moves the control point at `index`. Panics if there is none.
    pub fn set_control_point(&mut self, index: usize, point: (f32, f32)) {
        self.points[index] = point;
    }

    /// Adds a control point before position `index`, or at the end if
    /// `index` is the number of points.
    pub fn insert_control_point(&mut self, index: usize, point: (f32, f32)) {
        self.points.insert(index, point);
    }

    /// Removes and returns the control point at `index`.
    ///
    /// Panics if that would leave fewer than two.
    pub fn remove_control_point(&mut self, index: usize) -> (f32, f32) {
        assert!(
            self.points.len() > 2,
            "a Bezier curve needs at least 2 control points"
        );
        self.points.remove(index)
    }

    /// The point on the curve at `t`, where 0 is the start and 1 the end.
    pub fn evaluate(&self, t: f32) -> (f32, f32) {
        // De Casteljau: blend neighbours until one point is left.
        let mut points = self.points.clone();
        while points.len() > 1 {
            for i in 0..points.len() - 1 {
                points[i] = lerp(points[i], points[i + 1], t);
            }
            points.pop();
        }
        points[0]
    }

    /// The curve whose points are this curve's velocity: `evaluate(t)` on
    /// the result gives the direction and speed of travel at `t`.
    pub fn derivative(&self) -> BezierCurve {
        let degree = self.degree() as f32;
        let mut points: Vec<(f32, f32)> = self
            .points
            .windows(2)
            .map(|pair| {
                (
                    (pair[1].0 - pair[0].0) * degree,
                    (pair[1].1 - pair[0].1) * degree,
                )
            })
            .collect();
        if points.len() == 1 {
            // A straight line moves at one constant velocity.
            points.push(points[0]);
        }
        BezierCurve { points }
    }

    /// The part of the curve between `t1` and `t2` as a curve of its own.
    pub fn segment(&self, t1: f32, t2: f32) -> BezierCurve {
        // Cut off everything after t2, then everything before t1 in what
        // remains.
        let (left, _) = split(&self.points, t2);
        let inner = if t2 == 0.0 { 0.0 } else { t1 / t2 };
        let (_, right) = split(&left, inner);
        BezierCurve { points: right }
    }

    /// Points along the whole curve, for drawing it as a line. Each step
    /// of `depth` doubles how many there are; LÖVE's default is 5.
    pub fn render(&self, depth: u32) -> Vec<(f32, f32)> {
        self.render_segment(0.0, 1.0, depth)
    }

    /// Points along the curve between `t1` and `t2`.
    pub fn render_segment(&self, t1: f32, t2: f32, depth: u32) -> Vec<(f32, f32)> {
        let steps = self.degree() << depth.min(12);
        (0..=steps)
            .map(|step| self.evaluate(t1 + (t2 - t1) * step as f32 / steps as f32))
            .collect()
    }

    /// Moves the whole curve by `(dx, dy)`.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        for point in &mut self.points {
            *point = (point.0 + dx, point.1 + dy);
        }
    }

    /// Turns the curve by `angle` radians around `(ox, oy)`.
    pub fn rotate(&mut self, angle: f32, ox: f32, oy: f32) {
        let (sin, cos) = angle.sin_cos();
        for point in &mut self.points {
            let (x, y) = (point.0 - ox, point.1 - oy);
            *point = (ox + x * cos - y * sin, oy + x * sin + y * cos);
        }
    }

    /// Grows or shrinks the curve by `factor` around `(ox, oy)`.
    pub fn scale(&mut self, factor: f32, ox: f32, oy: f32) {
        for point in &mut self.points {
            *point = (ox + (point.0 - ox) * factor, oy + (point.1 - oy) * factor);
        }
    }
}

type Points = Vec<(f32, f32)>;

fn lerp(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Cuts a curve's control points at `t` into the control points of the
/// part before and the part after.
fn split(points: &[(f32, f32)], t: f32) -> (Points, Points) {
    let mut working = points.to_vec();
    let mut left = Vec::with_capacity(points.len());
    let mut right = Vec::with_capacity(points.len());
    while !working.is_empty() {
        left.push(working[0]);
        right.push(working[working.len() - 1]);
        for i in 0..working.len() - 1 {
            working[i] = lerp(working[i], working[i + 1], t);
        }
        working.pop();
    }
    right.reverse();
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn a_curve_starts_and_ends_at_its_outer_control_points() {
        let curve = BezierCurve::new(&[(0.0, 0.0), (50.0, 100.0), (100.0, 0.0)]);
        assert_eq!(curve.degree(), 2);
        assert!(close(curve.evaluate(0.0), (0.0, 0.0)));
        assert!(close(curve.evaluate(1.0), (100.0, 0.0)));
        // A quadratic reaches half way to its middle control point.
        assert!(close(curve.evaluate(0.5), (50.0, 50.0)));
    }

    #[test]
    fn a_two_point_curve_is_a_straight_line() {
        let line = BezierCurve::new(&[(0.0, 0.0), (10.0, 20.0)]);
        assert!(close(line.evaluate(0.25), (2.5, 5.0)));
        assert!(close(line.derivative().evaluate(0.7), (10.0, 20.0)));
    }

    #[test]
    #[should_panic(expected = "at least 2 control points")]
    fn a_curve_needs_two_points() {
        BezierCurve::new(&[(0.0, 0.0)]);
    }

    #[test]
    fn the_derivative_is_the_velocity_along_the_curve() {
        let curve = BezierCurve::new(&[(0.0, 0.0), (50.0, 100.0), (100.0, 0.0)]);
        let velocity = curve.derivative();
        assert_eq!(velocity.degree(), 1);
        // Leaving towards the middle point, flat at the top, then down.
        assert!(close(velocity.evaluate(0.0), (100.0, 200.0)));
        assert!(close(velocity.evaluate(0.5), (100.0, 0.0)));
        assert!(close(velocity.evaluate(1.0), (100.0, -200.0)));
    }

    #[test]
    fn a_segment_traces_the_same_path_as_the_whole_curve() {
        let curve = BezierCurve::new(&[(0.0, 0.0), (30.0, 90.0), (70.0, -40.0), (100.0, 10.0)]);
        let part = curve.segment(0.25, 0.75);
        assert_eq!(part.degree(), 3);
        for step in 0..=4 {
            let t = step as f32 / 4.0;
            assert!(close(part.evaluate(t), curve.evaluate(0.25 + t * 0.5)));
        }
        assert!(close(curve.segment(0.0, 0.0).evaluate(0.5), (0.0, 0.0)));
    }

    #[test]
    fn rendering_gives_more_points_at_greater_depth() {
        let curve = BezierCurve::new(&[(0.0, 0.0), (50.0, 100.0), (100.0, 0.0)]);
        let coarse = curve.render(1);
        let fine = curve.render(5);
        assert_eq!(coarse.len(), 5);
        assert_eq!(fine.len(), 65);
        assert!(close(fine[0], (0.0, 0.0)) && close(fine[64], (100.0, 0.0)));
        let half = curve.render_segment(0.0, 0.5, 3);
        assert!(close(half[half.len() - 1], curve.evaluate(0.5)));
    }

    #[test]
    fn control_points_can_be_changed() {
        let mut curve = BezierCurve::new(&[(0.0, 0.0), (10.0, 0.0)]);
        curve.insert_control_point(1, (5.0, 5.0));
        assert_eq!(
            curve.control_points(),
            [(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)]
        );
        curve.set_control_point(1, (5.0, 9.0));
        assert_eq!(curve.control_point(1), Some((5.0, 9.0)));
        assert_eq!(curve.remove_control_point(1), (5.0, 9.0));
        assert_eq!(curve.degree(), 1);
        assert_eq!(curve.control_point(7), None);
    }

    #[test]
    fn a_curve_can_be_moved_turned_and_resized() {
        let mut curve = BezierCurve::new(&[(0.0, 0.0), (10.0, 0.0)]);
        curve.translate(5.0, 5.0);
        assert_eq!(curve.control_points(), [(5.0, 5.0), (15.0, 5.0)]);
        curve.scale(2.0, 5.0, 5.0);
        assert_eq!(curve.control_points(), [(5.0, 5.0), (25.0, 5.0)]);
        curve.rotate(std::f32::consts::FRAC_PI_2, 5.0, 5.0);
        assert!(close(curve.control_points()[1], (5.0, 25.0)));
    }

    #[test]
    fn the_same_seed_gives_the_same_numbers() {
        let mut a = RandomGenerator::new(123);
        let mut b = RandomGenerator::new(123);
        let mut c = RandomGenerator::new(124);
        let from_a: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let from_b: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        let from_c: Vec<u64> = (0..8).map(|_| c.next_u64()).collect();
        assert_eq!(from_a, from_b);
        assert_ne!(from_a, from_c);
    }

    #[test]
    fn the_sequence_for_a_seed_never_changes() {
        // Saved games and seeded levels depend on this staying put.
        let mut generator = RandomGenerator::new(0);
        assert_eq!(generator.next_u64(), 0x53175D61490B23DF);
        assert_eq!(generator.next_u64(), 0x61DA6F3DC380D507);
    }

    #[test]
    fn setting_the_seed_restarts_the_sequence() {
        let mut generator = RandomGenerator::new(9);
        let first = generator.next_u64();
        generator.next_u64();
        generator.set_seed(9);
        assert_eq!(generator.next_u64(), first);
        assert_eq!(generator.seed(), 9);
    }

    #[test]
    fn random_stays_in_its_range_and_spreads_out() {
        let mut generator = RandomGenerator::new(1);
        let values: Vec<f32> = (0..10_000).map(|_| generator.random()).collect();
        assert!(values.iter().all(|value| (0.0..1.0).contains(value)));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.02, "mean was {mean}");

        for _ in 0..1000 {
            let value = generator.random_range(-3.0, 5.0);
            assert!((-3.0..5.0).contains(&value));
        }
    }

    #[test]
    fn random_int_includes_both_ends_and_takes_either_order() {
        let mut generator = RandomGenerator::new(2);
        let mut seen = [false; 6];
        for _ in 0..1000 {
            let roll = generator.random_int(1, 6);
            assert!((1..=6).contains(&roll));
            seen[roll as usize - 1] = true;
        }
        assert!(seen.iter().all(|&hit| hit));

        assert_eq!(generator.random_int(4, 4), 4);
        assert!((-2..=2).contains(&generator.random_int(2, -2)));
        // The widest range must not overflow.
        generator.random_int(i64::MIN, i64::MAX);
    }

    #[test]
    fn random_normal_centers_on_the_mean() {
        let mut generator = RandomGenerator::new(3);
        let values: Vec<f32> = (0..20_000)
            .map(|_| generator.random_normal(2.0, 10.0))
            .collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        let within = values
            .iter()
            .filter(|&&value| (value - 10.0).abs() < 2.0)
            .count() as f32
            / values.len() as f32;
        assert!((mean - 10.0).abs() < 0.1, "mean was {mean}");
        assert!(
            (within - 0.683).abs() < 0.03,
            "{within} within one deviation"
        );
    }

    #[test]
    fn shuffle_keeps_every_item_and_choose_picks_one_of_them() {
        let mut generator = RandomGenerator::new(4);
        let mut items: Vec<u32> = (0..50).collect();
        generator.shuffle(&mut items);
        assert_ne!(items, (0..50).collect::<Vec<_>>());
        let mut sorted = items.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..50).collect::<Vec<_>>());

        assert!(items.contains(generator.choose(&items).unwrap()));
        assert_eq!(generator.choose::<u32>(&[]), None);
    }

    #[test]
    fn the_shared_generator_repeats_after_seeding() {
        set_random_seed(77);
        let first = (random(), random_int(1, 100), random_range(0.0, 9.0));
        set_random_seed(77);
        let second = (random(), random_int(1, 100), random_range(0.0, 9.0));
        assert_eq!(first, second);
        assert_eq!(random_seed(), 77);
    }

    #[test]
    fn a_saved_state_carries_on_the_same_sequence() {
        let mut generator = RandomGenerator::new(9);
        generator.next_u64();
        let saved = generator.state();
        let expected: Vec<u64> = (0..5).map(|_| generator.next_u64()).collect();

        let mut other = RandomGenerator::new(1);
        other.set_state(&saved).unwrap();
        assert_eq!(other.seed(), 9);
        let replayed: Vec<u64> = (0..5).map(|_| other.next_u64()).collect();
        assert_eq!(replayed, expected);

        let too_long = format!("{saved}:0000000000000000");
        let zeros = format!("0x{}", ["0000000000000000"; 5].join(":"));
        for bad in ["", "0x12", "garbage", &too_long, &zeros] {
            assert_eq!(other.set_state(bad), Err(InvalidRandomState), "{bad}");
        }
        assert_eq!(other.seed(), 9);
    }

    #[test]
    fn the_shared_state_round_trips() {
        set_random_seed(4);
        let saved = random_state();
        let first = random();
        set_random_state(&saved).unwrap();
        assert_eq!(random(), first);
    }

    #[test]
    fn colors_convert_to_and_from_bytes() {
        let color = color_from_bytes(255, 0, 51, 128);
        assert_eq!(color.r, 1.0);
        assert_eq!(color.g, 0.0);
        assert!((color.b - 0.2).abs() < 1e-6);
        assert_eq!(color_to_bytes(color), [255, 0, 51, 128]);
        assert_eq!(
            color_to_bytes(Color::rgba(2.0, -1.0, 0.5, 1.0)),
            [255, 0, 128, 255]
        );
    }

    #[test]
    fn four_dimensional_noise_is_smooth_repeatable_and_in_range() {
        let at = |t: f32| noise4(t, 0.3 + t * 0.5, 0.7, 1.1 - t);
        let mut previous = at(0.0);
        let mut low: f32 = 1.0;
        let mut high: f32 = 0.0;
        for step in 1..2000 {
            let t = step as f32 * 0.01;
            let value = at(t);
            assert!((0.0..=1.0).contains(&value));
            assert!((value - previous).abs() < 0.1, "jumped at {t}");
            low = low.min(value);
            high = high.max(value);
            previous = value;
        }
        assert!(high - low > 0.3, "too flat: {low} to {high}");
        assert_eq!(noise4(1.5, 2.5, 3.5, 4.5), noise4(1.5, 2.5, 3.5, 4.5));
        assert_eq!(noise4(1.0, 2.0, 3.0, 4.0), 0.5);
    }

    #[test]
    fn noise_is_smooth_repeatable_and_in_range() {
        let mut low = f32::MAX;
        let mut high = f32::MIN;
        let mut previous = noise2(0.0, 0.3);
        for step in 1..4000 {
            let x = step as f32 * 0.01;
            let value = noise2(x, 0.3);
            assert!((0.0..=1.0).contains(&value));
            assert!((value - previous).abs() < 0.05, "jumped at {x}");
            low = low.min(value);
            high = high.max(value);
            previous = value;
        }
        // It uses a good part of the range rather than hovering at 0.5.
        assert!(low < 0.25 && high > 0.75, "only spanned {low} to {high}");
        assert_eq!(noise3(1.3, 2.7, -4.1), noise3(1.3, 2.7, -4.1));
        assert_eq!(noise(3.0), 0.5);
    }

    #[test]
    fn convexity() {
        let square = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let arrow = [(0.0, 0.0), (2.0, 1.0), (0.0, 2.0), (0.5, 1.0)];
        assert!(is_convex(&square));
        assert!(!is_convex(&arrow));
        assert!(!is_convex(&square[..2]));
    }

    fn area(triangles: &[[(f32, f32); 3]]) -> f32 {
        triangles
            .iter()
            .map(|[a, b, c]| cross(*a, *b, *c).abs() / 2.0)
            .sum()
    }

    #[test]
    fn triangulating_covers_the_polygon_exactly() {
        // An L shape, area 3, given in each direction.
        let mut shape = vec![
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ];
        for _ in 0..2 {
            let triangles = triangulate(&shape);
            assert_eq!(triangles.len(), 4);
            assert!((area(&triangles) - 3.0).abs() < 1e-5);
            shape.reverse();
        }
        assert!(triangulate(&shape[..2]).is_empty());
    }

    #[test]
    fn gamma_and_linear_undo_each_other() {
        for step in 0..=10 {
            let value = step as f32 / 10.0;
            assert!((linear_to_gamma(gamma_to_linear(value)) - value).abs() < 1e-5);
        }
        assert!((gamma_to_linear(0.5) - 0.214).abs() < 0.001);
    }
}
