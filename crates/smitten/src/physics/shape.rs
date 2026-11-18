use glam::Vec2;
use rapier2d::prelude::*;

/// What kind of outline a [`Shape`] is, the equivalent of LÖVE's
/// `ShapeType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShapeType {
    /// A circle, made with [`Shape::circle`].
    Circle,
    /// A convex polygon, rectangles included.
    Polygon,
    /// A single line.
    Edge,
    /// Lines joined end to end.
    Chain,
}

/// An outline that can be given to a body as a fixture, the equivalent of
/// LÖVE's `Shape` and its kinds (`CircleShape`, `PolygonShape`,
/// `EdgeShape` and `ChainShape`).
///
/// A shape is a plain value, measured from the origin of the body it goes
/// on: make one, then attach a copy of it with [`Body::add_shape`]. The
/// `add_` methods of [`Body`] make and attach one in one go.
/// [`Fixture::shape`] gives back a copy of a fixture's shape, and
/// [`Fixture::set_shape`] changes it, which is how a circle's radius is
/// changed.
///
/// [`Body`]: super::Body
/// [`Body::add_shape`]: super::Body::add_shape
/// [`Fixture::shape`]: super::Fixture::shape
/// [`Fixture::set_shape`]: super::Fixture::set_shape
///
/// ```
/// use smitten::physics::{BodyType, Shape, World};
///
/// let world = World::new((0.0, 0.0));
/// let body = world.new_body((100.0, 100.0), BodyType::Dynamic);
/// let fixture = body.add_shape(&Shape::circle(10.0));
///
/// // Grow the circle.
/// fixture.set_shape(&fixture.shape().with_radius(20.0));
/// assert_eq!(fixture.shape().radius(), 20.0);
/// ```
#[derive(Clone)]
pub struct Shape {
    pub(super) shape: SharedShape,
    /// Where the shape sits on the body.
    pub(super) offset: Pose,
    /// The ghost vertices of an edge or chain, measured from the body's
    /// origin.
    pub(super) previous: Option<(f32, f32)>,
    pub(super) next: Option<(f32, f32)>,
}

impl Shape {
    pub(super) fn new(shape: SharedShape, offset: Pose) -> Shape {
        Shape {
            shape,
            offset,
            previous: None,
            next: None,
        }
    }

    /// A circle centered on the body's origin. LÖVE's
    /// `newCircleShape(radius)`.
    pub fn circle(radius: f32) -> Shape {
        Shape::circle_at((0.0, 0.0), radius)
    }

    /// A circle centered `(x, y)` from the body's origin.
    pub fn circle_at(position: impl Into<Vec2>, radius: f32) -> Shape {
        let Vec2 { x, y } = position.into();
        assert!(
            radius > 0.0,
            "a circle needs a positive radius, got {radius}"
        );
        Shape::new(
            SharedShape::ball(radius),
            Pose::from_translation(Vector::new(x, y)),
        )
    }

    /// A rectangle of `width` by `height` centered on the body's origin.
    /// LÖVE's `newRectangleShape(width, height)`. It is a polygon.
    pub fn rectangle(size: impl Into<Vec2>) -> Shape {
        let Vec2 {
            x: width,
            y: height,
        } = size.into();
        Shape::rectangle_at((0.0, 0.0), (width, height), 0.0)
    }

    /// A rectangle centered `(x, y)` from the body's origin and turned by
    /// `angle` radians.
    pub fn rectangle_at(position: impl Into<Vec2>, size: impl Into<Vec2>, angle: f32) -> Shape {
        let Vec2 { x, y } = position.into();
        let Vec2 {
            x: width,
            y: height,
        } = size.into();
        assert!(
            width > 0.0 && height > 0.0,
            "a rectangle needs a positive size, got {width} by {height}"
        );
        Shape::new(
            SharedShape::cuboid(width / 2.0, height / 2.0),
            Pose::new(Vector::new(x, y), angle),
        )
    }

    /// A polygon with these corners, measured from the body's origin. A
    /// polygon with dents is filled out to the convex shape around it.
    /// LÖVE's `newPolygonShape`.
    ///
    /// Panics if the points do not span an area.
    pub fn polygon(points: &[impl Into<Vec2> + Copy]) -> Shape {
        let points = crate::math::tuples(points);
        let points = &points[..];
        let points: Vec<Vector> = points.iter().map(|&(x, y)| Vector::new(x, y)).collect();
        let shape = SharedShape::convex_hull(&points)
            .expect("a polygon needs at least three corners that are not in a line");
        Shape::new(shape, Pose::IDENTITY)
    }

    /// A thin line from `(x1, y1)` to `(x2, y2)`. LÖVE's `newEdgeShape`.
    pub fn edge(anchor1: impl Into<Vec2>, anchor2: impl Into<Vec2>) -> Shape {
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
        Shape::new(
            SharedShape::segment(Vector::new(x1, y1), Vector::new(x2, y2)),
            Pose::IDENTITY,
        )
    }

    /// Lines through these points, joined back to the start if `closed`.
    /// LÖVE's `newChainShape`.
    ///
    /// Panics with fewer than two points.
    pub fn chain(points: &[impl Into<Vec2> + Copy], closed: bool) -> Shape {
        let points = crate::math::tuples(points);
        let points = &points[..];
        assert!(points.len() >= 2, "a chain needs at least two points");
        let vertices: Vec<Vector> = points.iter().map(|&(x, y)| Vector::new(x, y)).collect();
        let count = vertices.len() as u32;
        let mut indices: Vec<[u32; 2]> = (0..count - 1).map(|index| [index, index + 1]).collect();
        if closed && count > 2 {
            indices.push([count - 1, 0]);
        }
        Shape::new(
            SharedShape::polyline(vertices, Some(indices)),
            Pose::IDENTITY,
        )
    }

    /// Tells an edge, or the start of a chain, what comes before it, the
    /// equivalent of `EdgeShape:setPreviousVertex` and
    /// `ChainShape:setPreviousVertex`. The point is measured from the
    /// body's origin like the shape's own.
    ///
    /// Nothing collides with this ghost vertex. It lets a body sliding
    /// over the start of the line pass onto it smoothly from the line
    /// before, where it would otherwise catch on the join. A chain's own
    /// joins are smooth already; this is for where one edge or chain
    /// meets another.
    ///
    /// Panics if this is not an edge or a chain.
    pub fn set_previous_vertex(&mut self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.check_lines("set_previous_vertex");
        self.previous = Some((x, y));
    }

    /// Tells an edge, or the end of a chain, what comes after it, the
    /// equivalent of `setNextVertex`. See [`Shape::set_previous_vertex`].
    ///
    /// Panics if this is not an edge or a chain.
    pub fn set_next_vertex(&mut self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.check_lines("set_next_vertex");
        self.next = Some((x, y));
    }

    /// The ghost vertex before an edge or chain, if one was set, the
    /// equivalent of `getPreviousVertex`. Other shapes have none.
    pub fn previous_vertex(&self) -> Option<Vec2> {
        self.previous.map(Vec2::from)
    }

    /// The ghost vertex after an edge or chain, if one was set, the
    /// equivalent of `getNextVertex`. Other shapes have none.
    pub fn next_vertex(&self) -> Option<Vec2> {
        self.next.map(Vec2::from)
    }

    fn check_lines(&self, method: &str) {
        assert!(
            matches!(self.kind(), ShapeType::Edge | ShapeType::Chain),
            "a {:?} shape has no ghost vertices, so it cannot {method}; only edges and chains do",
            self.kind()
        );
    }

    /// The ghost vertices in the space of the shape itself, without the
    /// offset on the body.
    pub(super) fn ghosts(&self) -> super::ghost::Ghosts {
        let local = |(x, y): (f32, f32)| self.offset.inverse_transform_point(Vector::new(x, y));
        super::ghost::Ghosts {
            previous: self.previous.map(local),
            next: self.next.map(local),
        }
    }

    /// Gives the shape back ghost vertices kept in its own space.
    pub(super) fn with_ghosts(mut self, ghosts: super::ghost::Ghosts) -> Shape {
        let placed = |point: Vector| {
            let point = self.offset.transform_point(point);
            (point.x, point.y)
        };
        self.previous = ghosts.previous.map(placed);
        self.next = ghosts.next.map(placed);
        self
    }

    /// What kind of shape this is.
    pub fn kind(&self) -> ShapeType {
        let shape = &self.shape;
        if shape.as_ball().is_some() {
            ShapeType::Circle
        } else if shape.as_segment().is_some() {
            ShapeType::Edge
        } else if shape.as_polyline().is_some() {
            ShapeType::Chain
        } else {
            ShapeType::Polygon
        }
    }

    /// A circle's radius. Other shapes have none and give 0.
    pub fn radius(&self) -> f32 {
        self.shape.as_ball().map_or(0.0, |ball| ball.radius)
    }

    /// The same circle with another radius. LÖVE's
    /// `CircleShape:setRadius`, on a copy.
    ///
    /// Panics if this is not a circle.
    pub fn with_radius(&self, radius: f32) -> Shape {
        let Vec2 { x, y } = self.point();
        Shape::circle_at((x, y), radius)
    }

    /// A circle's center, measured from the body's origin. LÖVE's
    /// `CircleShape:getPoint`.
    ///
    /// Panics if this is not a circle.
    pub fn point(&self) -> Vec2 {
        assert!(
            self.kind() == ShapeType::Circle,
            "a {:?} shape has no center point; only circles do",
            self.kind()
        );
        let center = self.offset.translation;
        (center.x, center.y).into()
    }

    /// The same circle moved to be centered `(x, y)` from the body's
    /// origin. LÖVE's `CircleShape:setPoint`, on a copy.
    ///
    /// Panics if this is not a circle.
    pub fn with_point(&self, position: impl Into<Vec2>) -> Shape {
        let Vec2 { x, y } = position.into();
        assert!(
            self.kind() == ShapeType::Circle,
            "a {:?} shape has no center point; only circles do",
            self.kind()
        );
        Shape::circle_at((x, y), self.radius())
    }

    /// The corners of a polygon, the two ends of an edge, or the points
    /// of a chain, measured from the body's origin. A circle has none.
    pub fn points(&self) -> Vec<Vec2> {
        let place = |point: Vector| {
            let point = self.offset.transform_point(point);
            Vec2::new(point.x, point.y)
        };
        let shape = &self.shape;
        if let Some(cuboid) = shape.as_cuboid() {
            let half = cuboid.half_extents;
            [
                Vector::new(-half.x, -half.y),
                Vector::new(half.x, -half.y),
                Vector::new(half.x, half.y),
                Vector::new(-half.x, half.y),
            ]
            .into_iter()
            .map(place)
            .collect()
        } else if let Some(polygon) = shape.as_convex_polygon() {
            polygon.points().iter().copied().map(place).collect()
        } else if let Some(segment) = shape.as_segment() {
            vec![place(segment.a), place(segment.b)]
        } else if let Some(chain) = shape.as_polyline() {
            chain.vertices().iter().copied().map(place).collect()
        } else {
            Vec::new()
        }
    }

    /// How many points [`Shape::points`] gives.
    pub fn vertex_count(&self) -> usize {
        self.points().len()
    }

    /// How many pieces the shape is made of: one for each line of a chain,
    /// and one for any other shape. Contacts say which piece was hit.
    pub fn child_count(&self) -> usize {
        self.shape
            .as_polyline()
            .map_or(1, |chain| chain.indices().len())
    }

    /// One line of a chain, counted from 0, as an edge. LÖVE's
    /// `ChainShape:getChildEdge`.
    ///
    /// Panics if this is not a chain or it has no such line.
    pub fn child_edge(&self, index: usize) -> Shape {
        let chain = self.shape.as_polyline().unwrap_or_else(|| {
            panic!(
                "a {:?} shape has no child edges; only chains do",
                self.kind()
            )
        });
        let segment = chain.indices().get(index).unwrap_or_else(|| {
            panic!(
                "this chain has {} edges, so there is no edge {index}",
                chain.indices().len()
            )
        });
        let vertices = chain.vertices();
        let (a, b) = (
            self.offset.transform_point(vertices[segment[0] as usize]),
            self.offset.transform_point(vertices[segment[1] as usize]),
        );
        Shape::edge((a.x, a.y), (b.x, b.y))
    }

    /// Where the shape would be on a body at `(x, y)` turned by `angle`.
    fn placed(&self, x: f32, y: f32, angle: f32) -> Pose {
        Pose::new(Vector::new(x, y), angle) * self.offset
    }

    /// The smallest box around the shape on a body at `(x, y)` turned by
    /// `angle`, as `(left, top, right, bottom)`. LÖVE's
    /// `Shape:computeAABB`.
    pub fn bounding_box(&self, position: impl Into<Vec2>, angle: f32) -> (f32, f32, f32, f32) {
        let Vec2 { x, y } = position.into();
        let aabb = self.shape.compute_aabb(&self.placed(x, y, angle));
        (aabb.mins.x, aabb.mins.y, aabb.maxs.x, aabb.maxs.y)
    }

    /// The `(center_x, center_y, mass, inertia)` the shape would give a
    /// body at a `density` per square meter, with `meter` units to a
    /// meter (pass [`World::meter`](super::World::meter)): its center of
    /// mass from the body's origin, its mass, and its inertia around that
    /// center. Lines and chains have no area and so no mass. LÖVE's
    /// `Shape:computeMass`.
    pub fn compute_mass(&self, density: f32, meter: f32) -> (f32, f32, f32, f32) {
        let props = self
            .shape
            .mass_properties(density / (meter * meter))
            .transform_by(&self.offset);
        (
            props.local_com.x,
            props.local_com.y,
            props.mass(),
            props.principal_inertia(),
        )
    }

    /// Where the line from `(x1, y1)` to `(x2, y2)` first runs into the
    /// shape on a body at `(x, y)` turned by `angle`, as `(normal_x,
    /// normal_y, fraction)`: the way the surface faces there, and how far
    /// along the line the hit is, from 0.0 to 1.0. LÖVE's
    /// `Shape:rayCast`.
    #[allow(clippy::too_many_arguments)] // Mirrors Shape:rayCast.
    pub fn ray_cast(
        &self,
        anchor1: impl Into<Vec2>,
        anchor2: impl Into<Vec2>,
        position: impl Into<Vec2>,
        angle: f32,
    ) -> Option<(f32, f32, f32)> {
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
        let Vec2 { x, y } = position.into();
        let ray = Ray::new(Vector::new(x1, y1), Vector::new(x2 - x1, y2 - y1));
        let hit = self
            .shape
            .cast_ray_and_get_normal(&self.placed(x, y, angle), &ray, 1.0, true)?;
        Some((hit.normal.x, hit.normal.y, hit.time_of_impact))
    }

    /// Whether the shape, on a body at `(x, y)` turned by `angle`, covers
    /// the point `(point_x, point_y)`. LÖVE's `Shape:testPoint`.
    pub fn test_point(
        &self,
        position: impl Into<Vec2>,
        angle: f32,
        point: impl Into<Vec2>,
    ) -> bool {
        let Vec2 { x, y } = position.into();
        let Vec2 {
            x: point_x,
            y: point_y,
        } = point.into();
        self.shape
            .contains_point(&self.placed(x, y, angle), Vector::new(point_x, point_y))
    }
}

impl std::fmt::Debug for Shape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = f.debug_struct("Shape");
        out.field("kind", &self.kind());
        match self.kind() {
            ShapeType::Circle => out
                .field("point", &self.point())
                .field("radius", &self.radius()),
            _ => out.field("points", &self.points()),
        };
        out.finish()
    }
}

impl PartialEq for Shape {
    /// Two shapes are equal if they are the same kind, in the same place,
    /// with the same points.
    fn eq(&self, other: &Shape) -> bool {
        self.kind() == other.kind()
            && self.radius() == other.radius()
            && self.points() == other.points()
            && (self.kind() != ShapeType::Circle || self.point() == other.point())
    }
}
