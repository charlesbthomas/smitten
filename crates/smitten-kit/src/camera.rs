//! A 2D camera: follow with a dead zone, bounds, eased zoom, shake, and
//! conversion between screen and world positions.
//!
//! A [`Camera`] is a value the game owns. Each frame, tell it what to look
//! at with [`follow`](Camera::follow) (or [`pan_to`](Camera::pan_to) and
//! [`frame`](Camera::frame)), call [`update`](Camera::update), and draw the
//! world inside [`draw`](Camera::draw):
//!
//! ```no_run
//! use smitten_kit::camera::{Camera, Follow, Smoothing};
//! use smitten_kit::math::Rect;
//! use smitten::Vec2;
//!
//! let mut camera = Camera::new((640.0, 360.0))
//!     .with_bounds(Rect::from_size((2000.0, 1000.0)))
//!     .with_follow(Follow {
//!         dead_zone: Vec2::new(64.0, 32.0),
//!         smoothing: Smoothing::Spring(0.25),
//!         look_ahead: 0.0,
//!     });
//!
//! # let (hero, dt) = (Vec2::ZERO, 1.0 / 60.0);
//! camera.follow(hero);
//! camera.update(dt);
//! camera.draw(|| {
//!     // Draw the world here, in world positions.
//! });
//! // Draw the HUD out here, in screen positions.
//! ```
//!
//! Everything except [`draw`](Camera::draw) and [`Camera::fullscreen`] is
//! plain arithmetic, so it works in tests without a window.

use smitten::{Vec2, graphics, math};

use crate::math::Rect;
use crate::tween::{Spring, approach};

/// How the camera eases toward where it wants to be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Smoothing {
    /// Jump straight there.
    Snap,
    /// Close half the distance every this many seconds. Quick to react,
    /// slows as it arrives.
    HalfLife(f32),
    /// A critically damped spring that arrives in roughly this many
    /// seconds: starts gently, lands softly, never overshoots, and keeps
    /// its speed when the target moves mid-way.
    Spring(f32),
}

/// How the camera follows a target. See [`Camera::follow`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follow {
    /// The size of a box around the center of the view, in world units,
    /// that the target can move inside without moving the camera.
    pub dead_zone: Vec2,
    /// How the camera eases after the target.
    pub smoothing: Smoothing,
    /// Look this many seconds ahead along the target's velocity, so more
    /// of what it is heading toward is on screen.
    pub look_ahead: f32,
}

impl Default for Follow {
    /// No dead zone, a 0.3 second spring, no look-ahead.
    fn default() -> Self {
        Follow {
            dead_zone: Vec2::ZERO,
            smoothing: Smoothing::Spring(0.3),
            look_ahead: 0.0,
        }
    }
}

/// How the camera shakes. See [`Camera::add_trauma`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shake {
    /// How far the view moves at full trauma, in screen units.
    pub max_offset: f32,
    /// How far the view turns at full trauma, in radians.
    pub max_angle: f32,
    /// Trauma lost per second.
    pub drain: f32,
    /// How fast the shake moves. Low is a sway, high a rattle.
    pub frequency: f32,
}

impl Default for Shake {
    fn default() -> Self {
        Shake {
            max_offset: 12.0,
            max_angle: 0.05,
            drain: 1.5,
            frequency: 20.0,
        }
    }
}

/// A view onto the world. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct Camera {
    /// The world position at the center of the view.
    pub position: Vec2,
    /// How much bigger the world is drawn: 2.0 shows half as much.
    pub zoom: f32,
    /// How far the view is turned, in radians.
    pub rotation: f32,
    viewport: Rect,
    clip: bool,
    bounds: Option<Rect>,
    follow: Follow,
    shake: Shake,
    zoom_limits: (f32, f32),
    zoom_smooth: f32,
    zoom_snap: Option<f32>,
    pixel_snap: bool,

    goal: Option<Vec2>,
    follow_target: Option<Vec2>,
    last_target: Option<Vec2>,
    spring: Spring<Vec2>,
    zoom_goal: f32,
    log_zoom: Spring<f32>,
    trauma: f32,
    shake_time: f32,
}

impl Camera {
    /// A camera for a view `size` units across, filling the screen from
    /// its top-left corner. It starts centered on `size / 2`, so at zoom
    /// 1.0 world positions are screen positions.
    pub fn new(size: impl Into<Vec2>) -> Self {
        let size = size.into();
        let position = size / 2.0;
        let follow = Follow::default();
        Camera {
            position,
            zoom: 1.0,
            rotation: 0.0,
            viewport: Rect::from_size(size),
            clip: false,
            bounds: None,
            follow,
            shake: Shake::default(),
            zoom_limits: (0.05, 20.0),
            zoom_smooth: 0.3,
            zoom_snap: None,
            pixel_snap: false,
            goal: None,
            follow_target: None,
            last_target: None,
            spring: Spring::new(position, 0.3),
            zoom_goal: 1.0,
            log_zoom: Spring::new(0.0, 0.3),
            trauma: 0.0,
            shake_time: 0.0,
        }
    }

    /// A camera the size of the screen (see [`graphics::dimensions`]).
    /// Needs the game to be running.
    pub fn fullscreen() -> Self {
        let (w, h) = graphics::dimensions();
        Camera::new((w as f32, h as f32))
    }

    /// The same camera drawing into `viewport`, a part of the screen, for
    /// split screen or a minimap. Drawing is clipped to it.
    pub fn with_viewport(mut self, viewport: Rect) -> Self {
        self.set_viewport(viewport);
        self
    }

    /// Draws into `viewport`, clipped to it.
    pub fn set_viewport(&mut self, viewport: Rect) {
        self.viewport = viewport;
        self.clip = true;
    }

    /// Changes the size of the view without clipping, as after the window
    /// is resized.
    pub fn resize(&mut self, size: impl Into<Vec2>) {
        self.viewport = Rect::from_position_size(self.viewport.min, size);
    }

    /// The part of the screen the camera draws into.
    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    /// The same camera, kept inside `bounds`. On an axis where the view
    /// is wider than the bounds, the camera stays at their center, so a
    /// bounds rectangle with no width locks the camera's x.
    pub fn with_bounds(mut self, bounds: Rect) -> Self {
        self.bounds = Some(bounds);
        self
    }

    /// Keeps the camera inside `bounds`, or lets it go anywhere with
    /// `None`.
    pub fn set_bounds(&mut self, bounds: Option<Rect>) {
        self.bounds = bounds;
    }

    /// The same camera with `follow` settings.
    pub fn with_follow(mut self, follow: Follow) -> Self {
        self.set_follow(follow);
        self
    }

    /// Changes how the camera follows.
    pub fn set_follow(&mut self, follow: Follow) {
        self.follow = follow;
        if let Smoothing::Spring(seconds) = follow.smoothing {
            self.spring.set_smooth_time(seconds);
        }
    }

    /// The same camera with `shake` settings.
    pub fn with_shake(mut self, shake: Shake) -> Self {
        self.shake = shake;
        self
    }

    /// The same camera, never zoomed below `min` or above `max`.
    pub fn with_zoom_limits(mut self, min: f32, max: f32) -> Self {
        self.zoom_limits = (min.max(f32::EPSILON), max.max(min));
        self.zoom_goal = self.clamp_zoom(self.zoom_goal);
        self
    }

    /// The same camera, taking about `seconds` to ease to a new zoom.
    /// Zoom eases on a log scale, so zooming in and out feel the same.
    pub fn with_zoom_smoothing(mut self, seconds: f32) -> Self {
        self.zoom_smooth = seconds;
        self.log_zoom.set_smooth_time(seconds);
        self
    }

    /// The same camera, rounding each zoom it is asked for so one art
    /// pixel covers a whole number of screen pixels, for crisp pixel art.
    /// `art_scale` is how many screen pixels an art pixel covers at zoom
    /// 1.0.
    pub fn with_zoom_snap(mut self, art_scale: f32) -> Self {
        self.zoom_snap = (art_scale > 0.0).then_some(art_scale);
        self
    }

    /// The same camera, drawing at whole screen pixels so pixel art does
    /// not shimmer as it moves.
    pub fn with_pixel_snap(mut self, snap: bool) -> Self {
        self.pixel_snap = snap;
        self
    }

    fn clamp_zoom(&self, zoom: f32) -> f32 {
        let zoom = zoom.clamp(self.zoom_limits.0, self.zoom_limits.1);
        match self.zoom_snap {
            Some(scale) => {
                let snapped = (zoom * scale).round().max(1.0) / scale;
                snapped.clamp(self.zoom_limits.0, self.zoom_limits.1)
            }
            None => zoom,
        }
    }

    /// Half the world size the view covers at `zoom`.
    fn half_view(&self, zoom: f32) -> Vec2 {
        self.viewport.size() / (2.0 * zoom)
    }

    fn clamp_position(&self, position: Vec2, zoom: f32) -> Vec2 {
        let Some(bounds) = self.bounds else {
            return position;
        };
        let half = self.half_view(zoom);
        let axis = |p: f32, min: f32, max: f32, half: f32| {
            if max - min <= half * 2.0 {
                (min + max) / 2.0
            } else {
                p.clamp(min + half, max - half)
            }
        };
        Vec2::new(
            axis(position.x, bounds.min.x, bounds.max.x, half.x),
            axis(position.y, bounds.min.y, bounds.max.y, half.y),
        )
    }

    /// Jumps to center on `position` at once, with no easing.
    pub fn look_at(&mut self, position: impl Into<Vec2>) {
        let position = self.clamp_position(position.into(), self.zoom);
        self.position = position;
        self.spring.snap_to(position);
        self.goal = None;
        self.follow_target = None;
        self.last_target = None;
    }

    /// Follows `target` this frame, using the [`Follow`] settings. Call
    /// it each frame with the target's position, then
    /// [`update`](Self::update).
    pub fn follow(&mut self, target: impl Into<Vec2>) {
        self.follow_target = Some(target.into());
    }

    /// Stops following and eases to center on `position`.
    pub fn pan_to(&mut self, position: impl Into<Vec2>) {
        self.follow_target = None;
        self.last_target = None;
        self.goal = Some(position.into());
    }

    /// Eases to `zoom`.
    pub fn zoom_to(&mut self, zoom: f32) {
        self.zoom_goal = self.clamp_zoom(zoom);
    }

    /// The zoom the camera is easing to.
    pub fn zoom_goal(&self) -> f32 {
        self.zoom_goal
    }

    /// Eases to show all of `rect`, filling `fill` (0.0 to 1.0) of the
    /// view on its tighter axis.
    pub fn frame(&mut self, rect: Rect, fill: f32) {
        let size = self.viewport.size() * fill;
        let zoom =
            (size.x / rect.width().max(f32::EPSILON)).min(size.y / rect.height().max(f32::EPSILON));
        self.zoom_to(zoom);
        self.pan_to(rect.center());
    }

    /// Multiplies the zoom by `factor`, keeping the world point under
    /// `screen` where it is: what a mouse wheel zoom should do.
    pub fn zoom_at(&mut self, screen: impl Into<Vec2>, factor: f32) {
        let screen = screen.into();
        let anchor = self.goal.unwrap_or(self.position);
        let offset = self.unrotate(screen - self.viewport.center());
        let world = anchor + offset / self.zoom_goal;
        self.zoom_to(self.zoom_goal * factor);
        self.pan_to(world - offset / self.zoom_goal);
    }

    /// Moves the view by `screen_delta` at once, so the world sticks to a
    /// dragging mouse.
    pub fn pan_by(&mut self, screen_delta: impl Into<Vec2>) {
        let delta = self.unrotate(screen_delta.into()) / self.zoom;
        let position = self.clamp_position(self.position - delta, self.zoom);
        self.position = position;
        self.spring.snap_to(position);
        if let Some(goal) = self.goal {
            self.goal = Some(self.clamp_position(goal - delta, self.zoom_goal));
        }
    }

    /// Whether the camera is still easing toward its goal or zoom.
    pub fn is_moving(&self) -> bool {
        let goal_far = self
            .goal
            .is_some_and(|goal| goal.distance_squared(self.position) > 0.05 * 0.05);
        goal_far || (self.zoom - self.zoom_goal).abs() > 1e-4
    }

    /// Adds `amount` of trauma, from 0.0 to 1.0. The shake grows with
    /// trauma squared, so small knocks barely register and big hits rattle.
    pub fn add_trauma(&mut self, amount: f32) {
        self.trauma = (self.trauma + amount).clamp(0.0, 1.0);
    }

    /// The trauma now, from 0.0 to 1.0.
    pub fn trauma(&self) -> f32 {
        self.trauma
    }

    /// Moves the camera on by `dt` seconds: follows, eases, zooms, keeps
    /// inside the bounds and drains the shake. Call it once a frame.
    pub fn update(&mut self, dt: f32) {
        // Zoom first, so the bounds use this frame's view size.
        if self.zoom_smooth <= 0.0 {
            self.zoom = self.zoom_goal;
        } else {
            self.log_zoom.set_value(self.zoom.max(f32::EPSILON).ln());
            self.log_zoom.set_target(self.zoom_goal.ln());
            self.zoom = self.log_zoom.update(dt).exp();
            if (self.zoom - self.zoom_goal).abs() < 1e-4 {
                self.zoom = self.zoom_goal;
            }
        }

        if let Some(target) = self.follow_target {
            let velocity = match self.last_target {
                Some(last) if dt > 0.0 => (target - last) / dt,
                _ => Vec2::ZERO,
            };
            self.last_target = Some(target);
            let aim = target + velocity * self.follow.look_ahead;
            // Move only as far as it takes to bring the aim back inside
            // the dead zone.
            let half = self.follow.dead_zone / 2.0;
            let from = self.goal.unwrap_or(self.position);
            let gap = aim - from;
            let push = |gap: f32, half: f32| {
                if gap > half {
                    gap - half
                } else if gap < -half {
                    gap + half
                } else {
                    0.0
                }
            };
            self.goal = Some(from + Vec2::new(push(gap.x, half.x), push(gap.y, half.y)));
        }

        if let Some(goal) = self.goal {
            let goal = self.clamp_position(goal, self.zoom_goal);
            self.goal = Some(goal);
            self.position = match self.follow.smoothing {
                Smoothing::Snap => goal,
                Smoothing::HalfLife(half_life) => approach(self.position, goal, half_life, dt),
                Smoothing::Spring(_) => {
                    self.spring.set_value(self.position);
                    self.spring.set_target(goal);
                    self.spring.update(dt)
                }
            };
        }
        self.position = self.clamp_position(self.position, self.zoom);

        self.trauma = (self.trauma - self.shake.drain * dt).max(0.0);
        self.shake_time += dt;
    }

    /// How far the shake moves and turns the view this frame.
    fn shake_now(&self) -> (Vec2, f32) {
        if self.trauma <= 0.0 {
            return (Vec2::ZERO, 0.0);
        }
        let strength = self.trauma * self.trauma;
        let t = self.shake_time * self.shake.frequency;
        // Smooth noise rather than random jumps, so the shake reads as
        // motion instead of flicker. Each channel samples its own row.
        let wave = |row: f32| math::noise2(row, t) * 2.0 - 1.0;
        let offset = Vec2::new(wave(1.5), wave(7.5)) * self.shake.max_offset * strength;
        (offset, wave(13.5) * self.shake.max_angle * strength)
    }

    fn rotate(&self, v: Vec2) -> Vec2 {
        Vec2::from_angle(self.rotation).rotate(v)
    }

    fn unrotate(&self, v: Vec2) -> Vec2 {
        Vec2::from_angle(-self.rotation).rotate(v)
    }

    /// Where on the screen the world point `world` is drawn, leaving out
    /// the shake.
    pub fn world_to_screen(&self, world: impl Into<Vec2>) -> Vec2 {
        self.viewport.center() + self.rotate((world.into() - self.position) * self.zoom)
    }

    /// Which world point is drawn at `screen`, leaving out the shake, such
    /// as what the mouse is pointing at.
    pub fn screen_to_world(&self, screen: impl Into<Vec2>) -> Vec2 {
        self.position + self.unrotate(screen.into() - self.viewport.center()) / self.zoom
    }

    /// The part of the world in view, for culling tiles and sprites. When
    /// the camera is turned, this is the box around the turned view.
    pub fn visible_rect(&self) -> Rect {
        let v = self.viewport;
        let corners = [
            v.min,
            Vec2::new(v.max.x, v.min.y),
            v.max,
            Vec2::new(v.min.x, v.max.y),
        ]
        .map(|corner| self.screen_to_world(corner));
        corners
            .iter()
            .skip(1)
            .fold(Rect::new(corners[0], corners[0]), |rect, &p| {
                rect.union(&Rect::new(p, p))
            })
    }

    /// Draws `draw` through the camera: world positions inside it land
    /// where the camera shows them. Every drawing setting is put back
    /// afterwards.
    pub fn draw(&self, draw: impl FnOnce()) {
        let (shake_offset, shake_angle) = self.shake_now();
        let mut center = self.viewport.center() + shake_offset;
        let mut position = self.position;
        if self.pixel_snap {
            center = center.round();
            position = (position * self.zoom).round() / self.zoom;
        }
        graphics::push_all();
        if self.clip {
            let v = self.viewport;
            graphics::intersect_scissor(
                v.min.x.round() as i32,
                v.min.y.round() as i32,
                v.width().round().max(0.0) as u32,
                v.height().round().max(0.0) as u32,
            );
        }
        graphics::translate(center);
        graphics::rotate(self.rotation + shake_angle);
        graphics::scale(self.zoom, self.zoom);
        graphics::translate(-position);
        draw();
        graphics::pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn settle(camera: &mut Camera) {
        for _ in 0..600 {
            camera.update(DT);
        }
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        a.distance(b) < 0.01
    }

    #[test]
    fn starts_with_world_matching_screen() {
        let camera = Camera::new((800.0, 600.0));
        assert_eq!(camera.screen_to_world((10.0, 20.0)), Vec2::new(10.0, 20.0));
        assert_eq!(camera.visible_rect(), Rect::from_size((800.0, 600.0)));
    }

    #[test]
    fn screen_and_world_round_trip_when_zoomed_and_turned() {
        let mut camera = Camera::new((800.0, 600.0));
        camera.position = Vec2::new(1000.0, -50.0);
        camera.zoom = 2.5;
        camera.rotation = 0.7;
        let world = Vec2::new(1012.0, -40.0);
        let back = camera.screen_to_world(camera.world_to_screen(world));
        assert!(close(back, world));
        assert!(close(
            camera.world_to_screen(camera.position),
            Vec2::new(400.0, 300.0)
        ));
    }

    #[test]
    fn follows_inside_the_dead_zone_without_moving() {
        let mut camera = Camera::new((800.0, 600.0)).with_follow(Follow {
            dead_zone: Vec2::new(100.0, 100.0),
            smoothing: Smoothing::Snap,
            look_ahead: 0.0,
        });
        camera.look_at((0.0, 0.0));
        camera.follow((40.0, -40.0));
        camera.update(DT);
        assert_eq!(camera.position, Vec2::ZERO);
        camera.follow((80.0, 0.0));
        camera.update(DT);
        assert_eq!(camera.position, Vec2::new(30.0, 0.0));
    }

    #[test]
    fn spring_follow_arrives() {
        let mut camera = Camera::new((800.0, 600.0));
        camera.look_at((0.0, 0.0));
        camera.follow((500.0, 200.0));
        settle(&mut camera);
        assert!(close(camera.position, Vec2::new(500.0, 200.0)));
    }

    #[test]
    fn bounds_keep_the_view_inside() {
        let mut camera = Camera::new((800.0, 600.0)).with_bounds(Rect::from_size((2000.0, 1000.0)));
        camera.look_at((0.0, 0.0));
        assert_eq!(camera.position, Vec2::new(400.0, 300.0));
        camera.look_at((5000.0, 5000.0));
        assert_eq!(camera.position, Vec2::new(1600.0, 700.0));
    }

    #[test]
    fn narrow_bounds_lock_an_axis() {
        let mut camera =
            Camera::new((800.0, 600.0)).with_bounds(Rect::new((400.0, f32::MIN), (400.0, 300.0)));
        camera.look_at((900.0, -2000.0));
        assert_eq!(camera.position, Vec2::new(400.0, -2000.0));
        // The bottom of the view stops at the bottom of the bounds.
        camera.look_at((900.0, 1000.0));
        assert_eq!(camera.position.y, 0.0);
    }

    #[test]
    fn frame_fits_a_rect() {
        let mut camera = Camera::new((800.0, 600.0));
        camera.frame(
            Rect::from_position_size((100.0, 100.0), (200.0, 100.0)),
            0.8,
        );
        settle(&mut camera);
        // 800 * 0.8 / 200 = 3.2 and 600 * 0.8 / 100 = 4.8: x is tighter.
        assert!((camera.zoom - 3.2).abs() < 1e-3);
        assert!(close(camera.position, Vec2::new(200.0, 150.0)));
        assert!(!camera.is_moving());
    }

    #[test]
    fn zoom_snaps_to_whole_art_pixels() {
        let mut camera = Camera::new((800.0, 600.0)).with_zoom_snap(4.0);
        camera.zoom_to(1.6);
        // 1.6 * 4 = 6.4 screen pixels per art pixel, rounded to 6.
        assert_eq!(camera.zoom_goal(), 1.5);
    }

    #[test]
    fn zoom_at_keeps_the_point_under_the_cursor() {
        let mut camera = Camera::new((800.0, 600.0)).with_follow(Follow {
            smoothing: Smoothing::Snap,
            ..Follow::default()
        });
        let cursor = Vec2::new(700.0, 100.0);
        let before = camera.screen_to_world(cursor);
        camera.zoom_at(cursor, 2.0);
        settle(&mut camera);
        assert!((camera.zoom - 2.0).abs() < 1e-3);
        assert!(close(camera.screen_to_world(cursor), before));
    }

    #[test]
    fn pan_by_drags_the_world() {
        let mut camera = Camera::new((800.0, 600.0));
        camera.zoom = 2.0;
        let grabbed = camera.screen_to_world((300.0, 300.0));
        camera.pan_by((50.0, 0.0));
        assert!(close(camera.screen_to_world((350.0, 300.0)), grabbed));
    }

    #[test]
    fn trauma_drains() {
        let mut camera = Camera::new((800.0, 600.0)).with_shake(Shake {
            drain: 1.0,
            ..Shake::default()
        });
        camera.add_trauma(0.7);
        camera.add_trauma(0.7);
        assert_eq!(camera.trauma(), 1.0);
        for _ in 0..30 {
            camera.update(DT);
        }
        assert!((camera.trauma() - 0.5).abs() < 1e-3);
        assert!(camera.shake_now().0.length() > 0.0 || camera.shake_now().1 != 0.0);
    }
}
