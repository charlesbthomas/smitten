/// A 2D affine transformation: any mix of moving, rotating and scaling.
///
/// Maps a point `(x, y)` to `(a*x + c*y + tx, b*x + d*y + ty)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// The transformation that leaves every point where it is.
    pub const IDENTITY: Transform = Transform {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// Builds the transformation LÖVE's `draw` arguments describe: scale and
    /// rotate around the origin offset `(ox, oy)`, then place it at `(x, y)`.
    pub fn from_draw_args(x: f32, y: f32, r: f32, sx: f32, sy: f32, ox: f32, oy: f32) -> Self {
        let mut transform = Self::IDENTITY;
        transform.translate(x, y);
        transform.rotate(r);
        transform.scale(sx, sy);
        transform.translate(-ox, -oy);
        transform
    }

    /// Moves the coordinate system by `(dx, dy)`.
    ///
    /// Like the other methods here, this affects what is drawn afterwards
    /// and is itself affected by the rotation and scale applied before it.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.tx += self.a * dx + self.c * dy;
        self.ty += self.b * dx + self.d * dy;
    }

    /// Rotates the coordinate system by `angle` radians, clockwise on screen.
    pub fn rotate(&mut self, angle: f32) {
        let (sin, cos) = angle.sin_cos();
        let (a, b, c, d) = (self.a, self.b, self.c, self.d);
        self.a = a * cos + c * sin;
        self.b = b * cos + d * sin;
        self.c = c * cos - a * sin;
        self.d = d * cos - b * sin;
    }

    /// Scales the coordinate system by `sx` horizontally and `sy` vertically.
    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.a *= sx;
        self.b *= sx;
        self.c *= sy;
        self.d *= sy;
    }

    /// Returns the transformation that applies `inner` first and then `self`.
    pub fn then_inner(&self, inner: &Transform) -> Transform {
        Transform {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            tx: self.a * inner.tx + self.c * inner.ty + self.tx,
            ty: self.b * inner.tx + self.d * inner.ty + self.ty,
        }
    }

    /// Transforms the point `(x, y)`.
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn assert_close(actual: (f32, f32), expected: (f32, f32)) {
        assert!(
            (actual.0 - expected.0).abs() < 1e-4 && (actual.1 - expected.1).abs() < 1e-4,
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn identity_leaves_points_alone() {
        assert_close(Transform::IDENTITY.apply(3.0, -4.0), (3.0, -4.0));
    }

    #[test]
    fn later_calls_happen_in_the_earlier_coordinate_system() {
        // As in LÖVE: after scaling by 2, a translation of 10 moves 20 pixels.
        let mut transform = Transform::IDENTITY;
        transform.scale(2.0, 2.0);
        transform.translate(10.0, 0.0);
        assert_close(transform.apply(1.0, 1.0), (22.0, 2.0));
    }

    #[test]
    fn positive_rotation_turns_x_toward_y() {
        // With y pointing down the screen, this is clockwise.
        let mut transform = Transform::IDENTITY;
        transform.rotate(FRAC_PI_2);
        assert_close(transform.apply(1.0, 0.0), (0.0, 1.0));
    }

    #[test]
    fn draw_args_rotate_around_the_origin_offset() {
        // A 10x10 sprite centered on (100, 100) and turned a quarter: its
        // center stays put and its top-left corner swings to the top-right.
        let transform = Transform::from_draw_args(100.0, 100.0, FRAC_PI_2, 1.0, 1.0, 5.0, 5.0);
        assert_close(transform.apply(5.0, 5.0), (100.0, 100.0));
        assert_close(transform.apply(0.0, 0.0), (105.0, 95.0));
    }

    #[test]
    fn then_inner_matches_applying_one_after_the_other() {
        let mut outer = Transform::IDENTITY;
        outer.translate(7.0, -2.0);
        outer.rotate(0.6);
        let mut inner = Transform::IDENTITY;
        inner.scale(3.0, 0.5);
        inner.translate(1.0, 4.0);

        let (ix, iy) = inner.apply(2.0, 9.0);
        assert_close(
            outer.then_inner(&inner).apply(2.0, 9.0),
            outer.apply(ix, iy),
        );
    }
}
