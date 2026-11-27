//! Shapes shared across the kit: [`Rect`] and [`Circle`] in `f32`, and
//! [`DRect`] and [`DCircle`] in `f64` for games that keep their world in
//! doubles.

use glam::{DVec2, Vec2};

macro_rules! rect {
    ($name:ident, $vec:ident, $f:ident) => {
        #[doc = concat!("An axis-aligned rectangle of `", stringify!($f), "` from `min` (top left) to")]
        /// `max` (bottom right), in whatever space the caller uses.
        ///
        /// A rectangle with `min == max` on an axis has no size on that axis; the
        /// camera uses one to lock that axis in place.
        #[derive(Clone, Copy, Debug, PartialEq, Default)]
        pub struct $name {
            /// The top-left corner.
            pub min: $vec,
            /// The bottom-right corner.
            pub max: $vec,
        }

        impl $name {
            /// A rectangle between two corners, in either order.
            pub fn new(a: impl Into<$vec>, b: impl Into<$vec>) -> $name {
                let (a, b) = (a.into(), b.into());
                $name {
                    min: a.min(b),
                    max: a.max(b),
                }
            }

            /// A rectangle from its top-left corner and its size.
            pub fn from_position_size(position: impl Into<$vec>, size: impl Into<$vec>) -> $name {
                let position = position.into();
                $name::new(position, position + size.into())
            }

            /// A rectangle from the origin to `size`.
            pub fn from_size(size: impl Into<$vec>) -> $name {
                $name::from_position_size($vec::ZERO, size)
            }

            /// A rectangle centered on `center`.
            pub fn from_center_size(center: impl Into<$vec>, size: impl Into<$vec>) -> $name {
                let (center, half) = (center.into(), size.into() / 2.0);
                $name::new(center - half, center + half)
            }

            /// Width and height.
            pub fn size(&self) -> $vec {
                self.max - self.min
            }

            /// The width.
            pub fn width(&self) -> $f {
                self.max.x - self.min.x
            }

            /// The height.
            pub fn height(&self) -> $f {
                self.max.y - self.min.y
            }

            /// The middle.
            pub fn center(&self) -> $vec {
                (self.min + self.max) / 2.0
            }

            /// Whether `point` is inside or on the edge.
            pub fn contains(&self, point: impl Into<$vec>) -> bool {
                let point = point.into();
                point.cmpge(self.min).all() && point.cmple(self.max).all()
            }

            /// Whether the two rectangles overlap, touching edges included.
            pub fn intersects(&self, other: &$name) -> bool {
                self.min.cmple(other.max).all() && other.min.cmple(self.max).all()
            }

            /// The overlap of two rectangles, or `None` when they are apart.
            pub fn intersection(&self, other: &$name) -> Option<$name> {
                self.intersects(other).then(|| $name {
                    min: self.min.max(other.min),
                    max: self.max.min(other.max),
                })
            }

            /// The smallest rectangle holding both.
            pub fn union(&self, other: &$name) -> $name {
                $name {
                    min: self.min.min(other.min),
                    max: self.max.max(other.max),
                }
            }

            /// The rectangle grown by `amount` on every side. A negative amount
            /// shrinks it.
            pub fn expand(&self, amount: $f) -> $name {
                $name::new(
                    self.min - $vec::splat(amount),
                    self.max + $vec::splat(amount),
                )
            }

            /// The same rectangle moved by `offset`.
            pub fn translate(&self, offset: impl Into<$vec>) -> $name {
                let offset = offset.into();
                $name {
                    min: self.min + offset,
                    max: self.max + offset,
                }
            }

            /// The nearest point inside the rectangle to `point`.
            pub fn clamp(&self, point: impl Into<$vec>) -> $vec {
                point.into().clamp(self.min, self.max)
            }
        }
    };
}

rect!(Rect, Vec2, f32);
rect!(DRect, DVec2, f64);

macro_rules! circle {
    ($name:ident, $rect:ident, $vec:ident, $f:ident) => {
        #[doc = concat!("A circle of `", stringify!($f), "`: a center and a radius.")]
        #[derive(Clone, Copy, Debug, PartialEq, Default)]
        pub struct $name {
            /// The middle.
            pub center: $vec,
            /// How far the edge is from the center.
            pub radius: $f,
        }

        impl $name {
            /// A circle around `center`.
            pub fn new(center: impl Into<$vec>, radius: $f) -> $name {
                $name {
                    center: center.into(),
                    radius: radius.abs(),
                }
            }

            /// Whether `point` is inside or on the edge.
            pub fn contains(&self, point: impl Into<$vec>) -> bool {
                self.center.distance_squared(point.into()) <= self.radius * self.radius
            }

            /// Whether the two circles overlap, touching included.
            pub fn intersects(&self, other: &$name) -> bool {
                let reach = self.radius + other.radius;
                self.center.distance_squared(other.center) <= reach * reach
            }

            /// Whether the circle overlaps the rectangle, touching included.
            pub fn intersects_rect(&self, rect: &$rect) -> bool {
                self.contains(rect.clamp(self.center))
            }

            /// The smallest rectangle holding the circle.
            pub fn bounds(&self) -> $rect {
                $rect::from_center_size(self.center, $vec::splat(self.radius * 2.0))
            }

            /// The same circle moved by `offset`.
            pub fn translate(&self, offset: impl Into<$vec>) -> $name {
                $name {
                    center: self.center + offset.into(),
                    radius: self.radius,
                }
            }
        }
    };
}

circle!(Circle, Rect, Vec2, f32);
circle!(DCircle, DRect, DVec2, f64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_in_any_order() {
        let r = Rect::new((10.0, 0.0), (0.0, 5.0));
        assert_eq!(r.min, Vec2::new(0.0, 0.0));
        assert_eq!(r.max, Vec2::new(10.0, 5.0));
        assert_eq!(r.size(), Vec2::new(10.0, 5.0));
        assert_eq!(r.center(), Vec2::new(5.0, 2.5));
    }

    #[test]
    fn overlap_and_union() {
        let a = Rect::from_size((10.0, 10.0));
        let b = Rect::from_position_size((5.0, 5.0), (10.0, 10.0));
        let c = Rect::from_position_size((20.0, 20.0), (1.0, 1.0));
        assert!(a.intersects(&b));
        assert!(!a.intersects(&c));
        assert_eq!(
            a.intersection(&b),
            Some(Rect::new((5.0, 5.0), (10.0, 10.0)))
        );
        assert_eq!(a.intersection(&c), None);
        assert_eq!(a.union(&c), Rect::new((0.0, 0.0), (21.0, 21.0)));
    }

    #[test]
    fn contains_and_clamp() {
        let r = Rect::from_center_size((0.0, 0.0), (4.0, 2.0));
        assert!(r.contains((2.0, 1.0)));
        assert!(!r.contains((2.1, 0.0)));
        assert_eq!(r.clamp((5.0, -5.0)), Vec2::new(2.0, -1.0));
        assert_eq!(r.expand(1.0).size(), Vec2::new(6.0, 4.0));
    }

    #[test]
    fn drect_matches_rect() {
        let r = DRect::from_center_size((0.0, 0.0), (4.0, 2.0));
        assert_eq!(r.min, DVec2::new(-2.0, -1.0));
        assert!(r.contains((2.0, 1.0)));
        assert_eq!(r.width(), 4.0_f64);
    }

    #[test]
    fn circles() {
        let a = Circle::new((0.0, 0.0), 1.0);
        assert!(a.contains((0.6, 0.6)));
        assert!(!a.contains((0.8, 0.8)));
        assert!(a.intersects(&Circle::new((2.0, 0.0), 1.0)));
        assert!(!a.intersects(&Circle::new((2.1, 0.0), 1.0)));
        assert!(a.intersects_rect(&Rect::from_position_size((0.9, -5.0), (1.0, 10.0))));
        assert!(!a.intersects_rect(&Rect::from_position_size((0.8, 0.8), (1.0, 1.0))));
        assert_eq!(a.bounds(), Rect::new((-1.0, -1.0), (1.0, 1.0)));
        let d = DCircle::new((0.0, 0.0), 2.0);
        assert!(d.contains((1.0, 1.0)));
    }
}
