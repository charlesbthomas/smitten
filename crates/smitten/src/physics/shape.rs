use glam::Vec2;
use rapier2d::prelude::*;
use std::marker::PhantomData;
use std::ops::Deref;

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
/// use smitten::physics::{BodyType, Shape, World, shape_kind};
///
/// let world = World::new((0.0, 0.0));
/// let body = world.new_body((100.0, 100.0), BodyType::Dynamic);
/// let fixture = body.add_shape(&Shape::circle(10.0));
///
/// // Grow the circle.
/// let circle = fixture.shape().downcast::<shape_kind::Circle>().unwrap();
/// fixture.set_shape(&circle.with_radius(20.0));
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
    pub fn circle(radius: f32) -> CircleShape {
        Shape::circle_at((0.0, 0.0), radius)
    }

    /// A circle centered `(x, y)` from the body's origin.
    pub fn circle_at(position: impl Into<Vec2>, radius: f32) -> CircleShape {
        let Vec2 { x, y } = position.into();
        assert!(
            radius > 0.0,
            "a circle needs a positive radius, got {radius}"
        );
        ShapeOf::wrap(Shape::new(
            SharedShape::ball(radius),
            Pose::from_translation(Vector::new(x, y)),
        ))
    }

    /// A rectangle of `width` by `height` centered on the body's origin.
    /// LÖVE's `newRectangleShape(width, height)`. It is a polygon.
    pub fn rectangle(size: impl Into<Vec2>) -> PolygonShape {
        let Vec2 {
            x: width,
            y: height,
        } = size.into();
        Shape::rectangle_at((0.0, 0.0), (width, height), 0.0)
    }

    /// A rectangle centered `(x, y)` from the body's origin and turned by
    /// `angle` radians.
    pub fn rectangle_at(
        position: impl Into<Vec2>,
        size: impl Into<Vec2>,
        angle: f32,
    ) -> PolygonShape {
        let Vec2 { x, y } = position.into();
        let Vec2 {
            x: width,
            y: height,
        } = size.into();
        assert!(
            width > 0.0 && height > 0.0,
            "a rectangle needs a positive size, got {width} by {height}"
        );
        ShapeOf::wrap(Shape::new(
            SharedShape::cuboid(width / 2.0, height / 2.0),
            Pose::new(Vector::new(x, y), angle),
        ))
    }

    /// A polygon with these corners, measured from the body's origin. A
    /// polygon with dents is filled out to the convex shape around it.
    /// LÖVE's `newPolygonShape`.
    ///
    /// Panics if the points do not span an area.
    pub fn polygon(points: &[impl Into<Vec2> + Copy]) -> PolygonShape {
        let points = crate::math::tuples(points);
        let points = &points[..];
        let points: Vec<Vector> = points.iter().map(|&(x, y)| Vector::new(x, y)).collect();
        let shape = SharedShape::convex_hull(&points)
            .expect("a polygon needs at least three corners that are not in a line");
        ShapeOf::wrap(Shape::new(shape, Pose::IDENTITY))
    }

    /// A thin line from `(x1, y1)` to `(x2, y2)`. LÖVE's `newEdgeShape`.
    pub fn edge(anchor1: impl Into<Vec2>, anchor2: impl Into<Vec2>) -> EdgeShape {
        let Vec2 { x: x1, y: y1 } = anchor1.into();
        let Vec2 { x: x2, y: y2 } = anchor2.into();
        ShapeOf::wrap(Shape::new(
            SharedShape::segment(Vector::new(x1, y1), Vector::new(x2, y2)),
            Pose::IDENTITY,
        ))
    }

    /// Lines through these points, joined back to the start if `closed`.
    /// LÖVE's `newChainShape`.
    ///
    /// Panics with fewer than two points.
    pub fn chain(points: &[impl Into<Vec2> + Copy], closed: bool) -> ChainShape {
        let points = crate::math::tuples(points);
        let points = &points[..];
        assert!(points.len() >= 2, "a chain needs at least two points");
        let vertices: Vec<Vector> = points.iter().map(|&(x, y)| Vector::new(x, y)).collect();
        let count = vertices.len() as u32;
        let mut indices: Vec<[u32; 2]> = (0..count - 1).map(|index| [index, index + 1]).collect();
        if closed && count > 2 {
            indices.push([count - 1, 0]);
        }
        ShapeOf::wrap(Shape::new(
            SharedShape::polyline(vertices, Some(indices)),
            Pose::IDENTITY,
        ))
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

    /// Where a circle's center is on the body.
    fn center(&self) -> Vec2 {
        let center = self.offset.translation;
        Vec2::new(center.x, center.y)
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

    /// Where the shape would be on a body at `(x, y)` turned by `angle`.
    fn placed(&self, x: f32, y: f32, angle: f32) -> Pose {
        Pose::new(Vector::new(x, y), angle) * self.offset
    }

    /// The smallest box around the shape on a body at `(x, y)` turned by
    /// `angle`, as `(left, top, right, bottom)`. LÖVE's
    /// `Shape:computeAABB`.
    #[doc(alias = "Shape:computeAABB")]
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
    #[doc(alias = "Shape:computeMass")]
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
    #[doc(alias = "Shape:rayCast")]
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
    #[doc(alias = "Shape:testPoint")]
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
                .field("point", &self.center())
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
            && (self.kind() != ShapeType::Circle || self.center() == other.center())
    }
}

/// A shape of one kind, `K` from [`shape_kind`], such as a
/// [`CircleShape`]. It has the methods that kind of shape has, and derefs
/// to [`Shape`] for the ones every shape has.
pub struct ShapeOf<K> {
    inner: Shape,
    kind: PhantomData<K>,
}

impl<K: shape_kind::Kind> ShapeOf<K> {
    fn wrap(inner: Shape) -> Self {
        debug_assert_eq!(inner.kind(), K::TYPE);
        ShapeOf {
            inner,
            kind: PhantomData,
        }
    }
}

impl<K> Deref for ShapeOf<K> {
    type Target = Shape;

    fn deref(&self) -> &Shape {
        &self.inner
    }
}

impl<K> AsRef<Shape> for ShapeOf<K> {
    fn as_ref(&self) -> &Shape {
        &self.inner
    }
}

impl<K> From<ShapeOf<K>> for Shape {
    fn from(shape: ShapeOf<K>) -> Shape {
        shape.inner
    }
}

impl<K> Clone for ShapeOf<K> {
    fn clone(&self) -> Self {
        ShapeOf {
            inner: self.inner.clone(),
            kind: PhantomData,
        }
    }
}

impl<K> PartialEq for ShapeOf<K> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<K> PartialEq<Shape> for ShapeOf<K> {
    fn eq(&self, other: &Shape) -> bool {
        self.inner == *other
    }
}

impl<K> PartialEq<ShapeOf<K>> for Shape {
    fn eq(&self, other: &ShapeOf<K>) -> bool {
        *self == other.inner
    }
}

impl<K> std::fmt::Debug for ShapeOf<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

/// The kinds of shape, as types for [`ShapeOf`].
pub mod shape_kind {
    use super::ShapeType;

    mod sealed {
        pub trait Sealed {}
    }

    /// A kind of shape.
    pub trait Kind: sealed::Sealed {
        /// The kind as a value.
        const TYPE: ShapeType;
    }

    /// Edges and chains, which can have ghost vertices.
    pub trait Lines: Kind {}

    macro_rules! kinds {
        ($($name:ident: $($has:ident),*;)*) => {$(
            #[doc = concat!("The kind of a [`ShapeType::", stringify!($name), "`] shape.")]
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub enum $name {}
            impl sealed::Sealed for $name {}
            impl Kind for $name {
                const TYPE: ShapeType = ShapeType::$name;
            }
            $(impl $has for $name {})*
        )*};
    }

    kinds! {
        Circle: ;
        Polygon: ;
        Edge: Lines;
        Chain: Lines;
    }
}

/// A circle, the equivalent of LÖVE's `CircleShape`.
pub type CircleShape = ShapeOf<shape_kind::Circle>;
/// A convex polygon, rectangles included, the equivalent of LÖVE's
/// `PolygonShape`.
pub type PolygonShape = ShapeOf<shape_kind::Polygon>;
/// A single line, the equivalent of LÖVE's `EdgeShape`.
pub type EdgeShape = ShapeOf<shape_kind::Edge>;
/// Lines joined end to end, the equivalent of LÖVE's `ChainShape`.
pub type ChainShape = ShapeOf<shape_kind::Chain>;

/// A shape as its own type, from [`Shape::typed`], to match on.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum AnyShape {
    Circle(CircleShape),
    Polygon(PolygonShape),
    Edge(EdgeShape),
    Chain(ChainShape),
}

impl Shape {
    /// The shape as its own type, to match on.
    pub fn typed(&self) -> AnyShape {
        let shape = self.clone();
        match self.kind() {
            ShapeType::Circle => AnyShape::Circle(ShapeOf::wrap(shape)),
            ShapeType::Polygon => AnyShape::Polygon(ShapeOf::wrap(shape)),
            ShapeType::Edge => AnyShape::Edge(ShapeOf::wrap(shape)),
            ShapeType::Chain => AnyShape::Chain(ShapeOf::wrap(shape)),
        }
    }

    /// The shape as a shape of kind `K`, or `None` if it is another kind.
    pub fn downcast<K: shape_kind::Kind>(&self) -> Option<ShapeOf<K>> {
        (self.kind() == K::TYPE).then(|| ShapeOf::wrap(self.clone()))
    }
}

/// Circles.
impl CircleShape {
    /// The same circle with another radius. LÖVE's
    /// `CircleShape:setRadius`, on a copy.
    #[doc(alias = "CircleShape:setRadius")]
    pub fn with_radius(&self, radius: f32) -> CircleShape {
        Shape::circle_at(self.point(), radius)
    }

    /// A circle's center, measured from the body's origin. LÖVE's
    /// `CircleShape:getPoint`.
    #[doc(alias = "CircleShape:getPoint")]
    pub fn point(&self) -> Vec2 {
        self.center()
    }

    /// The same circle moved to be centered `(x, y)` from the body's
    /// origin. LÖVE's `CircleShape:setPoint`, on a copy.
    #[doc(alias = "CircleShape:setPoint")]
    pub fn with_point(&self, position: impl Into<Vec2>) -> CircleShape {
        Shape::circle_at(position, self.radius())
    }
}

/// Edges and chains: their ghost vertices.
impl<K: shape_kind::Lines> ShapeOf<K> {
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
    #[doc(alias = "EdgeShape:setPreviousVertex")]
    pub fn set_previous_vertex(&mut self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.inner.previous = Some((x, y));
    }

    /// Tells an edge, or the end of a chain, what comes after it, the
    /// equivalent of `setNextVertex`. See [`set_previous_vertex`](Self::set_previous_vertex).
    pub fn set_next_vertex(&mut self, position: impl Into<Vec2>) {
        let Vec2 { x, y } = position.into();
        self.inner.next = Some((x, y));
    }
}

/// Chains.
impl ChainShape {
    /// One line of a chain, counted from 0, as an edge. LÖVE's
    /// `ChainShape:getChildEdge`.
    ///
    /// Panics if the chain has no such line.
    #[doc(alias = "ChainShape:getChildEdge")]
    pub fn child_edge(&self, index: usize) -> EdgeShape {
        let chain = self
            .shape
            .as_polyline()
            .expect("a chain shape holds a polyline");
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
}
