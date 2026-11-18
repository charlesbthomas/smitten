//! Canvases, blend modes and the scissor.
//!
//! - The scene on the left is drawn to a small canvas and scaled up with
//!   hard pixel edges, the usual way to make a pixel art game.
//! - The glowing dots use additive blending, so they brighten where they
//!   overlap.
//! - Move the mouse over the panel on the right to paint. The paint lives
//!   on a canvas that is never cleared, so strokes stay without being
//!   redrawn. Press C to wipe it.
//! - The stripes are cut off by a scissor rectangle.
//! - Escape quits.

use smitten::graphics::DrawParams;
use smitten::graphics::{Canvas, CanvasSettings, DrawMode};
use smitten::{BlendMode, Color, Config, Filter, Game, Key, event, graphics, mouse, timer};

const SCENE_WIDTH: u32 = 96;
const SCENE_HEIGHT: u32 = 96;
const SCENE_SCALE: f32 = 4.0;
const MARGIN: f32 = 24.0;
const PANEL_X: f32 = MARGIN * 2.0 + SCENE_WIDTH as f32 * SCENE_SCALE;
const PANEL_SIZE: u32 = 384;

struct Canvases {
    scene: Canvas,
    paint: Canvas,
}

impl Canvases {
    /// Redraws the small scene. Positions here are in canvas pixels.
    fn draw_scene(&self) {
        let time = timer::time() as f32;
        self.scene.render_to(|| {
            graphics::clear(Color::rgb(0.08, 0.09, 0.16));

            graphics::set_color(Color::rgb(0.2, 0.55, 0.35));
            graphics::rectangle(DrawMode::Fill, (0.0, 72.0), (SCENE_WIDTH as f32, 24.0));
            graphics::set_color(Color::rgb(0.95, 0.35, 0.45));
            graphics::circle(
                DrawMode::Fill,
                (48.0 + (time * 1.3).cos() * 30.0, 60.0),
                10.0,
            );

            // Lights add to what is under them instead of covering it.
            graphics::set_blend_mode(BlendMode::Add);
            for light in 0..3 {
                let angle = time + light as f32 * 2.1;
                graphics::set_color(Color::rgba(1.0, 0.8, 0.3, 0.45));
                graphics::circle(
                    DrawMode::Fill,
                    (48.0 + angle.cos() * 24.0, 40.0 + (angle * 1.7).sin() * 16.0),
                    14.0,
                );
            }
            graphics::set_blend_mode(BlendMode::Alpha);
        });
    }
}

impl Game for Canvases {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::rgb(0.11, 0.12, 0.15));
        // Exactly one pixel per unit, so it looks just as chunky on a
        // high-DPI display. The paint canvas below is as sharp as the screen.
        let scene = Canvas::with_settings(
            SCENE_WIDTH,
            SCENE_HEIGHT,
            CanvasSettings {
                dpi_scale: Some(1.0),
                ..CanvasSettings::default()
            },
        )?;
        scene.set_filter(Filter::Nearest);
        Ok(Canvases {
            scene,
            paint: Canvas::new(PANEL_SIZE, PANEL_SIZE)?,
        })
    }

    fn update(&mut self, _dt: f32) {
        // Add a dab of paint under the mouse. Nothing else on the paint
        // canvas is touched, so earlier dabs stay.
        let smitten::Vec2 { x, y } = mouse::position();
        let hue = timer::time() as f32;
        self.paint.render_to(|| {
            graphics::set_color(Color::rgba(
                0.5 + 0.5 * hue.sin(),
                0.5 + 0.5 * (hue + 2.0).sin(),
                0.5 + 0.5 * (hue + 4.0).sin(),
                0.6,
            ));
            graphics::circle(DrawMode::Fill, (x - PANEL_X, y - MARGIN), 9.0);
        });
    }

    fn draw(&mut self) {
        self.draw_scene();

        graphics::set_color(Color::WHITE);
        graphics::draw(
            &self.scene,
            DrawParams::at((MARGIN, MARGIN)).scale(SCENE_SCALE),
        );

        // The panel: stripes limited by a scissor, with the paint on top.
        let panel = PANEL_SIZE as f32;
        graphics::set_color(Color::rgb(0.16, 0.17, 0.21));
        graphics::rectangle(DrawMode::Fill, (PANEL_X, MARGIN), (panel, panel));

        graphics::set_scissor(PANEL_X as i32, MARGIN as i32, PANEL_SIZE, PANEL_SIZE);
        graphics::set_color(Color::rgb(0.2, 0.21, 0.26));
        let slide = (timer::time() as f32 * 30.0) % 48.0;
        for stripe in -10..10 {
            let x = PANEL_X + stripe as f32 * 48.0 + slide;
            graphics::polygon(
                DrawMode::Fill,
                &[
                    (x, MARGIN),
                    (x + 24.0, MARGIN),
                    (x + 24.0 + panel, MARGIN + panel),
                    (x + panel, MARGIN + panel),
                ],
            );
        }
        graphics::clear_scissor();

        // A canvas holds colors already multiplied by their alpha.
        graphics::set_color(Color::WHITE);
        graphics::set_blend_mode(BlendMode::Premultiplied);
        graphics::draw(&self.paint, (PANEL_X, MARGIN));
        graphics::set_blend_mode(BlendMode::Alpha);

        graphics::print(
            "Low-resolution canvas, scaled up",
            (MARGIN, MARGIN + panel + 10.0),
        );
        graphics::print(
            "Move the mouse here to paint. C clears.",
            (PANEL_X, MARGIN + panel + 10.0),
        );
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        match key {
            Key::C => self
                .paint
                .render_to(|| graphics::clear(Color::rgba(0.0, 0.0, 0.0, 0.0))),
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Canvases>(Config {
        title: "Smitten canvases".into(),
        width: (PANEL_X + PANEL_SIZE as f32 + MARGIN) as u32,
        height: (MARGIN * 2.0 + PANEL_SIZE as f32 + 30.0) as u32,
        ..Config::default()
    });
}
