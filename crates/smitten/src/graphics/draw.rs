//! Placing things on the screen: [`DrawParams`], the [`Drawable`] trait
//! and the text that [`print`](super::print) takes.

use glam::Vec2;

use crate::Color;

use super::{Image, Quad, Transform};

/// Where and how [`draw`](super::draw) places something: the position,
/// rotation, scale, origin and shear, and an optional [`Transform`] applied
/// around them.
///
/// Anything that turns into a position works where `DrawParams` is asked
/// for, so the plain case stays short:
///
/// ```no_run
/// use smitten::graphics::{self, DrawParams, Image};
///
/// # fn demo(image: &Image) {
/// graphics::draw(image, (100.0, 50.0));
/// // Spun around its center and doubled in size.
/// let center = image.size() / 2.0;
/// graphics::draw(image, DrawParams::at((100.0, 50.0)).rotation(0.5).scale(2.0).origin(center));
/// # }
/// ```
///
/// The parts are applied in this order: the origin is moved to `(0, 0)`,
/// the result is sheared, scaled, rotated, and then moved to `position`. A
/// `transform` set as well is applied last, to the placed result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawParams {
    /// Where the origin ends up. Defaults to `(0, 0)`.
    pub position: Vec2,
    /// The rotation in radians, clockwise on screen. Defaults to 0.
    pub rotation: f32,
    /// The scale along each axis. Defaults to `(1, 1)`.
    pub scale: Vec2,
    /// The point inside the drawn thing that is placed at `position` and
    /// rotated and scaled around. Defaults to its top-left corner,
    /// `(0, 0)`.
    pub origin: Vec2,
    /// The shear along each axis. Defaults to `(0, 0)`.
    pub shear: Vec2,
    /// A transform applied after the others. Defaults to the identity.
    pub transform: Transform,
}

impl Default for DrawParams {
    fn default() -> Self {
        Self::new()
    }
}

impl DrawParams {
    /// Parameters that draw something as it is, at `(0, 0)`.
    pub const fn new() -> Self {
        Self {
            position: Vec2::ZERO,
            rotation: 0.0,
            scale: Vec2::ONE,
            origin: Vec2::ZERO,
            shear: Vec2::ZERO,
            transform: Transform::IDENTITY,
        }
    }

    /// Parameters that draw something as it is, at `position`.
    pub fn at(position: impl Into<Vec2>) -> Self {
        Self::new().position(position)
    }

    /// Sets where the origin ends up.
    #[must_use]
    pub fn position(mut self, position: impl Into<Vec2>) -> Self {
        self.position = position.into();
        self
    }

    /// Sets the rotation, in radians clockwise on screen.
    #[must_use]
    pub fn rotation(mut self, angle: f32) -> Self {
        self.rotation = angle;
        self
    }

    /// Sets the same scale along both axes.
    #[must_use]
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = Vec2::splat(scale);
        self
    }

    /// Sets a separate scale along each axis. A negative scale mirrors.
    #[must_use]
    pub fn scale_xy(mut self, scale: impl Into<Vec2>) -> Self {
        self.scale = scale.into();
        self
    }

    /// Sets the point inside the drawn thing that is placed at the
    /// position and turned around. Half an image's size spins it around
    /// its center.
    #[must_use]
    pub fn origin(mut self, origin: impl Into<Vec2>) -> Self {
        self.origin = origin.into();
        self
    }

    /// Sets the shear along each axis.
    #[must_use]
    pub fn shear(mut self, shear: impl Into<Vec2>) -> Self {
        self.shear = shear.into();
        self
    }

    /// Sets a transform applied after the position, rotation, scale and
    /// the rest.
    #[must_use]
    pub fn transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    /// The single transformation these parameters describe.
    pub fn to_transform(&self) -> Transform {
        let mut local = Transform::IDENTITY;
        local.translate(self.position);
        local.rotate(self.rotation);
        local.scale(self.scale.x, self.scale.y);
        local.shear(self.shear);
        local.translate(-self.origin);
        self.transform * local
    }
}

impl From<Vec2> for DrawParams {
    fn from(position: Vec2) -> Self {
        Self::at(position)
    }
}

impl From<(f32, f32)> for DrawParams {
    fn from(position: (f32, f32)) -> Self {
        Self::at(position)
    }
}

impl From<[f32; 2]> for DrawParams {
    fn from(position: [f32; 2]) -> Self {
        Self::at(position)
    }
}

impl From<Transform> for DrawParams {
    fn from(transform: Transform) -> Self {
        Self::new().transform(transform)
    }
}

impl From<&Transform> for DrawParams {
    fn from(transform: &Transform) -> Self {
        Self::new().transform(*transform)
    }
}

impl From<DrawParams> for Transform {
    fn from(params: DrawParams) -> Self {
        params.to_transform()
    }
}

/// Something [`draw`](super::draw) can put on the screen: an [`Image`], a
/// [`Canvas`](super::Canvas), a [`Video`](super::Video), part of an image
/// from [`Image::region`], a layer from
/// [`ArrayImage::layer`](super::ArrayImage::layer), a
/// [`SpriteBatch`](super::SpriteBatch), a [`Mesh`](super::Mesh), a
/// [`ParticleSystem`](super::ParticleSystem) or a [`Text`](super::Text).
///
/// The trait is sealed: smitten draws each of these its own way.
pub trait Drawable: sealed::Sealed {}

pub(crate) mod sealed {
    use super::Transform;

    pub trait Sealed {
        /// Queues the drawable, placed by `local` inside the current
        /// transform.
        fn draw_transformed(&self, local: &Transform);
    }
}

impl<T: Drawable + ?Sized> Drawable for &T {}

impl<T: sealed::Sealed + ?Sized> sealed::Sealed for &T {
    fn draw_transformed(&self, local: &Transform) {
        (**self).draw_transformed(local);
    }
}

/// Part of an image, drawn by [`draw`](super::draw). Made with
/// [`Image::region`].
#[derive(Debug, Clone, Copy)]
pub struct ImageRegion<'a> {
    pub(crate) image: &'a Image,
    pub(crate) quad: &'a Quad,
}

impl Image {
    /// The part of this image that `quad` describes, to draw with
    /// [`draw`](super::draw). The origin of the [`DrawParams`] is then
    /// measured from the quad's top-left corner.
    pub fn region<'a>(&'a self, quad: &'a Quad) -> ImageRegion<'a> {
        ImageRegion { image: self, quad }
    }
}

/// Text for [`print`](super::print), [`printf`](super::printf) and
/// [`Text`](super::Text): a plain string, or a list of parts each tinted by
/// its own color as well as the current one.
///
/// ```no_run
/// use smitten::{Color, graphics};
///
/// graphics::print("Hello", (10.0, 10.0));
/// graphics::print(&[(Color::WHITE, "Score: "), (Color::rgb(1.0, 0.8, 0.2), "120")], (10.0, 30.0));
/// ```
#[derive(Debug, Clone, Copy)]
pub enum TextSpans<'a> {
    /// A string in the current color.
    Plain(&'a str),
    /// Parts each tinted by their color as well as the current one.
    Colored(&'a [(Color, &'a str)]),
}

impl TextSpans<'_> {
    /// Calls `f` with the parts as a list, a plain string being one white
    /// part.
    pub(crate) fn with_segments<R>(self, f: impl FnOnce(&[(Color, &str)]) -> R) -> R {
        match self {
            TextSpans::Plain(text) => f(&[(Color::WHITE, text)]),
            TextSpans::Colored(segments) => f(segments),
        }
    }

    /// Whether there is no text at all.
    pub(crate) fn is_empty(self) -> bool {
        match self {
            TextSpans::Plain(text) => text.is_empty(),
            TextSpans::Colored(segments) => segments.iter().all(|(_, text)| text.is_empty()),
        }
    }
}

impl<'a> From<&'a str> for TextSpans<'a> {
    fn from(text: &'a str) -> Self {
        TextSpans::Plain(text)
    }
}

impl<'a> From<&&'a str> for TextSpans<'a> {
    fn from(text: &&'a str) -> Self {
        TextSpans::Plain(text)
    }
}

impl<'a> From<&'a String> for TextSpans<'a> {
    fn from(text: &'a String) -> Self {
        TextSpans::Plain(text)
    }
}

impl<'a> From<&'a [(Color, &'a str)]> for TextSpans<'a> {
    fn from(segments: &'a [(Color, &'a str)]) -> Self {
        TextSpans::Colored(segments)
    }
}

impl<'a, const N: usize> From<&'a [(Color, &'a str); N]> for TextSpans<'a> {
    fn from(segments: &'a [(Color, &'a str); N]) -> Self {
        TextSpans::Colored(segments)
    }
}

impl<'a> From<&'a Vec<(Color, &'a str)>> for TextSpans<'a> {
    fn from(segments: &'a Vec<(Color, &'a str)>) -> Self {
        TextSpans::Colored(segments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_match_the_draw_arguments() {
        let params = DrawParams::at((30.0, 40.0))
            .rotation(0.7)
            .scale_xy((2.0, 3.0))
            .origin((5.0, 6.0));
        let expected = Transform::from_draw_args(30.0, 40.0, 0.7, 2.0, 3.0, 5.0, 6.0);
        for point in [(0.0, 0.0), (1.0, 2.0), (-4.0, 9.0)] {
            let (got, want) = (params.to_transform().apply(point), expected.apply(point));
            assert!((got - want).length() < 1e-4, "{got} != {want}");
        }
    }

    #[test]
    fn a_transform_is_applied_after_the_rest() {
        let mut outer = Transform::IDENTITY;
        outer.translate((100.0, 0.0));
        let params = DrawParams::at((1.0, 2.0)).scale(2.0).transform(outer);
        assert_eq!(
            params.to_transform().apply((1.0, 1.0)),
            Vec2::new(103.0, 4.0)
        );
    }
}
