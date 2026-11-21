//! Shapes shared across the kit.

use smitten::Vec2;

/// An axis-aligned rectangle from `min` (top left) to `max` (bottom
/// right), in whatever space the caller uses.
///
/// A rectangle with `min == max` on an axis has no size on that axis; the
/// camera uses one to lock that axis in place.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    /// The top-left corner.
    pub min: Vec2,
    /// The bottom-right corner.
    pub max: Vec2,
}

impl Rect {
    /// A rectangle between two corners, in either order.
    pub fn new(a: impl Into<Vec2>, b: impl Into<Vec2>) -> Rect {
        let (a, b) = (a.into(), b.into());
        Rect {
            min: a.min(b),
            max: a.max(b),
        }
    }

    /// A rectangle from its top-left corner and its size.
    pub fn from_position_size(position: impl Into<Vec2>, size: impl Into<Vec2>) -> Rect {
        let position = position.into();
        Rect::new(position, position + size.into())
    }

    /// A rectangle from the origin to `size`.
    pub fn from_size(size: impl Into<Vec2>) -> Rect {
        Rect::from_position_size(Vec2::ZERO, size)
    }

    /// A rectangle centered on `center`.
    pub fn from_center_size(center: impl Into<Vec2>, size: impl Into<Vec2>) -> Rect {
        let (center, half) = (center.into(), size.into() / 2.0);
        Rect::new(center - half, center + half)
    }

    /// Width and height.
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }

    /// The width.
    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    /// The height.
    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    /// The middle.
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) / 2.0
    }

    /// Whether `point` is inside or on the edge.
    pub fn contains(&self, point: impl Into<Vec2>) -> bool {
        let point = point.into();
        point.cmpge(self.min).all() && point.cmple(self.max).all()
    }

    /// Whether the two rectangles overlap, touching edges included.
    pub fn intersects(&self, other: &Rect) -> bool {
        self.min.cmple(other.max).all() && other.min.cmple(self.max).all()
    }

    /// The overlap of two rectangles, or `None` when they are apart.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        self.intersects(other).then(|| Rect {
            min: self.min.max(other.min),
            max: self.max.min(other.max),
        })
    }

    /// The smallest rectangle holding both.
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// The rectangle grown by `amount` on every side. A negative amount
    /// shrinks it.
    pub fn expand(&self, amount: f32) -> Rect {
        Rect::new(
            self.min - Vec2::splat(amount),
            self.max + Vec2::splat(amount),
        )
    }

    /// The same rectangle moved by `offset`.
    pub fn translate(&self, offset: impl Into<Vec2>) -> Rect {
        let offset = offset.into();
        Rect {
            min: self.min + offset,
            max: self.max + offset,
        }
    }

    /// The nearest point inside the rectangle to `point`.
    pub fn clamp(&self, point: impl Into<Vec2>) -> Vec2 {
        point.into().clamp(self.min, self.max)
    }
}

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
}
