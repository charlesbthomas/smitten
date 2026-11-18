//! How the game's coordinates map onto the window's pixels.

use smitten_core::{Config, Scaling};

use super::Transform;

/// Where a unit square lands on the window: `scale` pixels per unit,
/// starting `(x, y)` pixels from the top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct View {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

impl View {
    pub fn transform(&self) -> Transform {
        let mut transform = Transform::IDENTITY;
        transform.translate((self.x, self.y));
        transform.scale(self.scale, self.scale);
        transform
    }

    /// Turns a position in window pixels into units.
    pub fn to_units(self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.x) / self.scale, (y - self.y) / self.scale)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Screen {
    /// The window's drawable size in pixels.
    pub pixels: (u32, u32),
    /// Pixels per unit asked for by the display, or 1.0 when the game
    /// turned DPI scaling off.
    pub dpi: f32,
    dpi_scaling: bool,
    pub design: Option<(u32, u32)>,
    pub scaling: Scaling,
}

impl Default for Screen {
    fn default() -> Self {
        Screen {
            pixels: (0, 0),
            dpi: 1.0,
            dpi_scaling: true,
            design: None,
            scaling: Scaling::Fit,
        }
    }
}

impl Screen {
    pub fn new(config: &Config) -> Screen {
        Screen {
            dpi_scaling: config.dpi_scaling,
            design: config
                .design_size
                .filter(|&(width, height)| width > 0 && height > 0),
            scaling: config.design_scaling,
            ..Screen::default()
        }
    }

    /// Records the window's current size and the display's scale.
    /// Whether sizes are measured in units rather than pixels. See
    /// [`Config::dpi_scaling`].
    pub fn dpi_scaling(&self) -> bool {
        self.dpi_scaling
    }

    pub fn update(&mut self, pixels: (u32, u32), display_scale: f32) {
        self.pixels = pixels;
        self.dpi = if self.dpi_scaling && display_scale > 0.0 {
            display_scale
        } else {
            1.0
        };
    }

    /// The design size, if the game draws to a surface of exactly that
    /// many pixels which is enlarged afterwards.
    pub fn pixel_perfect(&self) -> Option<(u32, u32)> {
        match self.scaling {
            Scaling::PixelPerfect => self.design,
            Scaling::Fit => None,
        }
    }

    /// Where the game's picture sits in the window.
    pub fn view(&self) -> View {
        let Some((width, height)) = self.design else {
            return View {
                x: 0.0,
                y: 0.0,
                scale: self.dpi,
            };
        };
        let (pixel_width, pixel_height) = (self.pixels.0 as f32, self.pixels.1 as f32);
        let (width, height) = (width as f32, height as f32);
        let mut scale = (pixel_width / width).min(pixel_height / height);
        if self.scaling == Scaling::PixelPerfect {
            scale = scale.floor().max(1.0);
        }
        if !(scale.is_finite() && scale > 0.0) {
            // The window has no size yet, or is minimized.
            scale = 1.0;
        }
        View {
            // Whole pixels, so the picture's edges are not blurred.
            x: ((pixel_width - width * scale) / 2.0).floor(),
            y: ((pixel_height - height * scale) / 2.0).floor(),
            scale,
        }
    }

    /// The size the game draws in: the design size, or the window in units.
    pub fn size(&self) -> (u32, u32) {
        self.design.unwrap_or_else(|| self.window_size())
    }

    /// The window's size in units, whatever the design size.
    pub fn window_size(&self) -> (u32, u32) {
        (
            (self.pixels.0 as f32 / self.dpi).round() as u32,
            (self.pixels.1 as f32 / self.dpi).round() as u32,
        )
    }

    /// The number of real pixels behind [`Screen::size`].
    pub fn pixel_size(&self) -> (u32, u32) {
        match (self.pixel_perfect(), self.design) {
            (Some(design), _) => design,
            (None, Some((width, height))) => {
                let scale = self.view().scale;
                (
                    (width as f32 * scale).round() as u32,
                    (height as f32 * scale).round() as u32,
                )
            }
            (None, None) => self.pixels,
        }
    }

    /// Pixels per unit on the surface the game draws to. New fonts and
    /// canvases are made this sharp.
    pub fn draw_scale(&self) -> f32 {
        if self.pixel_perfect().is_some() {
            1.0
        } else {
            self.view().scale
        }
    }

    /// The parts of the window the game's picture does not cover, as
    /// `[left, top, right, bottom]` pixel rectangles. Empty when it covers
    /// everything.
    pub fn bars(&self) -> Vec<[f32; 4]> {
        let Some((width, height)) = self.design else {
            return Vec::new();
        };
        let view = self.view();
        let (pixel_width, pixel_height) = (self.pixels.0 as f32, self.pixels.1 as f32);
        let (right, bottom) = (
            view.x + width as f32 * view.scale,
            view.y + height as f32 * view.scale,
        );
        [
            [0.0, 0.0, pixel_width, view.y],
            [0.0, bottom, pixel_width, pixel_height],
            [0.0, view.y, view.x, bottom],
            [right, view.y, pixel_width, bottom],
        ]
        .into_iter()
        .filter(|bar| bar[2] > bar[0] && bar[3] > bar[1])
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    fn screen(config: Config, pixels: (u32, u32), display_scale: f32) -> Screen {
        let mut screen = Screen::new(&config);
        screen.update(pixels, display_scale);
        screen
    }

    #[test]
    fn on_an_ordinary_display_a_unit_is_a_pixel() {
        let screen = screen(Config::default(), (800, 600), 1.0);
        assert_eq!(screen.size(), (800, 600));
        assert_eq!(screen.pixel_size(), (800, 600));
        assert_eq!(screen.draw_scale(), 1.0);
        assert_eq!(screen.view().to_units(400.0, 300.0), (400.0, 300.0));
        assert!(screen.bars().is_empty());
    }

    #[test]
    fn a_high_dpi_display_keeps_the_size_in_units() {
        let screen = screen(Config::default(), (1600, 1200), 2.0);
        assert_eq!(screen.size(), (800, 600));
        assert_eq!(screen.pixel_size(), (1600, 1200));
        assert_eq!(screen.draw_scale(), 2.0);
        assert_eq!(screen.view().to_units(800.0, 600.0), (400.0, 300.0));
        assert_eq!(
            screen.view().transform().apply((400.0, 300.0)),
            Vec2::new(800.0, 600.0)
        );
    }

    #[test]
    fn dpi_scaling_can_be_turned_off() {
        let config = Config {
            dpi_scaling: false,
            ..Config::default()
        };
        let screen = screen(config, (1600, 1200), 2.0);
        assert_eq!(screen.size(), (1600, 1200));
        assert_eq!(screen.draw_scale(), 1.0);
    }

    #[test]
    fn a_design_size_is_fitted_and_centered_with_bars() {
        let config = Config {
            design_size: Some((320, 180)),
            ..Config::default()
        };
        // A 4:3 window is taller than the 16:9 design.
        let screen = screen(config, (800, 600), 1.0);
        let view = screen.view();
        assert_eq!(screen.size(), (320, 180));
        assert_eq!(screen.window_size(), (800, 600));
        assert_eq!(view.scale, 2.5);
        assert_eq!((view.x, view.y), (0.0, 75.0));
        assert_eq!(screen.pixel_size(), (800, 450));
        assert_eq!(screen.draw_scale(), 2.5);
        assert_eq!(view.to_units(400.0, 300.0), (160.0, 90.0));
        assert_eq!(
            screen.bars(),
            [[0.0, 0.0, 800.0, 75.0], [0.0, 525.0, 800.0, 600.0]]
        );
    }

    #[test]
    fn pixel_perfect_scaling_uses_whole_multiples() {
        let config = Config {
            design_size: Some((320, 180)),
            design_scaling: Scaling::PixelPerfect,
            ..Config::default()
        };
        let mut screen = screen(config, (800, 600), 1.0);
        let view = screen.view();
        assert_eq!(view.scale, 2.0);
        assert_eq!((view.x, view.y), (80.0, 120.0));
        assert_eq!(screen.pixel_size(), (320, 180));
        assert_eq!(screen.draw_scale(), 1.0);
        assert_eq!(screen.bars().len(), 4);

        // A window smaller than the design still shows it at 1x.
        screen.update((200, 100), 1.0);
        assert_eq!(screen.view().scale, 1.0);
    }

    #[test]
    fn a_design_size_ignores_the_display_scale() {
        let config = Config {
            design_size: Some((400, 300)),
            ..Config::default()
        };
        let screen = screen(config, (1600, 1200), 2.0);
        assert_eq!(screen.view().scale, 4.0);
        assert_eq!(screen.window_size(), (800, 600));
    }
}
