//! 2D lights and shadows, made from canvases, a shader and the stencil.
//!
//! The room is drawn as usual, fully lit. Then a light map is built on a
//! canvas: it starts out as the dim ambient color, and each light adds
//! its glow to it. Before a light is added, the shadows it casts are
//! marked in the canvas's stencil, and the glow is kept out of them.
//! Finally the light map is multiplied over the room, so lit parts keep
//! their colors and the rest goes dark.
//!
//! - One light follows the mouse. Click to leave a light behind, and
//!   press C to take those away again.
//! - Space switches the shadows off and on.
//! - L shows the room without the light map, and M shows the light map
//!   by itself.
//! - Up and Down change how bright the darkness is.
//! - Escape quits.

use std::f32::consts::TAU;

use smitten::graphics::{Canvas, DrawMode, Image, Shader};
use smitten::{
    BlendMode, Color, CompareMode, Config, Game, Key, MouseButton, event, graphics, keyboard, math,
    mouse, timer,
};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 520;
const TILE: f32 = 40.0;

/// How far shadows reach. Anything past the edge of the window will do.
const SHADOW_LENGTH: f32 = 2000.0;

/// Turns a plain square into a round glow that fades out toward its edge.
///
/// The glow is strongest in the middle of the picture and gone at the
/// edge of the circle that fits inside it. `falloff` shapes the fade:
/// higher values give a smaller bright core and a longer dim tail.
const GLOW: &str = "
struct Params {
    falloff: f32,
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let distance_from_center = distance(uv, vec2<f32>(0.5, 0.5)) * 2.0;
    let strength = pow(clamp(1.0 - distance_from_center, 0.0, 1.0), params.falloff);
    return vec4<f32>(color.rgb, color.a * strength);
}
";

#[derive(Clone, Copy)]
struct Light {
    x: f32,
    y: f32,
    radius: f32,
    color: Color,
}

/// Something that blocks light: a convex polygon, with its corners in
/// clockwise or counter-clockwise order.
struct Block {
    corners: Vec<(f32, f32)>,
    color: Color,
}

struct Lighting {
    light_map: Canvas,
    glow: Shader,
    heart: Image,
    placed: Vec<Light>,
    shadows_on: bool,
    view: View,
    ambient: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Lit,
    Unlit,
    LightMap,
}

/// The corners of an axis-aligned box.
fn box_corners(x: f32, y: f32, width: f32, height: f32) -> Vec<(f32, f32)> {
    vec![
        (x, y),
        (x + width, y),
        (x + width, y + height),
        (x, y + height),
    ]
}

/// The corners of a regular polygon with `sides` sides, turned by `turn`.
fn regular_corners(x: f32, y: f32, radius: f32, sides: usize, turn: f32) -> Vec<(f32, f32)> {
    (0..sides)
        .map(|index| {
            let angle = turn + index as f32 / sides as f32 * TAU;
            (x + angle.cos() * radius, y + angle.sin() * radius)
        })
        .collect()
}

/// The blocks in the room. Two of them move, so they are rebuilt every
/// frame.
fn blocks() -> Vec<Block> {
    let time = timer::time() as f32;
    let stone = Color::rgb(0.55, 0.5, 0.6);
    let crate_wood = Color::rgb(0.65, 0.45, 0.28);
    vec![
        Block {
            corners: box_corners(120.0, 120.0, 80.0, 80.0),
            color: stone,
        },
        Block {
            corners: box_corners(600.0, 120.0, 80.0, 80.0),
            color: stone,
        },
        Block {
            corners: box_corners(120.0, 340.0, 80.0, 80.0),
            color: stone,
        },
        Block {
            corners: box_corners(600.0, 340.0, 80.0, 80.0),
            color: stone,
        },
        Block {
            corners: box_corners(330.0, 60.0, 140.0, 24.0),
            color: crate_wood,
        },
        // A crate that turns in the middle of the room.
        Block {
            corners: regular_corners(400.0, 260.0, 42.0, 4, time * 0.6),
            color: crate_wood,
        },
        // A pillar that slides back and forth along the bottom.
        Block {
            corners: regular_corners(400.0 + (time * 0.8).sin() * 150.0, 440.0, 22.0, 8, 0.0),
            color: stone,
        },
    ]
}

/// The lights that move by themselves.
fn wandering_lights() -> Vec<Light> {
    let time = timer::time() as f32;
    // A torch whose brightness and size flicker by a little noise.
    let flicker = math::noise(time * 6.0);
    vec![
        Light {
            x: 60.0,
            y: 470.0,
            radius: 260.0 + flicker * 40.0,
            color: Color::rgb(1.0, 0.55 + flicker * 0.15, 0.2),
        },
        Light {
            x: 400.0 + (time * 0.5).cos() * 320.0,
            y: 140.0 + (time * 0.9).sin() * 40.0,
            radius: 300.0,
            color: Color::rgb(0.35, 0.55, 1.0),
        },
        Light {
            x: 700.0,
            y: 260.0 + (time * 0.7).sin() * 150.0,
            radius: 240.0,
            color: Color::rgb(0.4, 1.0, 0.5),
        },
    ]
}

impl Lighting {
    fn all_lights(&self) -> Vec<Light> {
        let (x, y) = mouse::position();
        let mut lights = wandering_lights();
        lights.push(Light {
            x,
            y,
            radius: 280.0,
            color: Color::rgb(1.0, 0.92, 0.75),
        });
        lights.extend(self.placed.iter().copied());
        lights
    }

    /// The room as it would look in full light.
    fn draw_room(&self, blocks: &[Block]) {
        // A stone floor, each tile a slightly different shade, with
        // darker mortar showing between them.
        graphics::clear(Color::rgb(0.24, 0.22, 0.22));
        for row in 0..(HEIGHT as f32 / TILE).ceil() as i32 {
            for column in 0..(WIDTH as f32 / TILE).ceil() as i32 {
                let shade = 0.42 + math::noise2(column as f32 * 0.73, row as f32 * 0.61) * 0.16;
                graphics::set_color(Color::rgb(shade, shade * 0.95, shade * 0.9));
                graphics::rectangle(
                    DrawMode::Fill,
                    column as f32 * TILE + 1.0,
                    row as f32 * TILE + 1.0,
                    TILE - 2.0,
                    TILE - 2.0,
                );
            }
        }

        // Hearts on the floor, to show that lights tint whatever they
        // fall on.
        graphics::set_color(Color::WHITE);
        for (x, y) in [
            (270.0, 200.0),
            (500.0, 330.0),
            (280.0, 400.0),
            (520.0, 160.0),
        ] {
            graphics::draw_ex(&self.heart, x, y, 0.0, 3.0, 3.0, 8.0, 8.0);
        }

        for block in blocks {
            graphics::set_color(block.color);
            graphics::polygon(DrawMode::Fill, &block.corners);
            graphics::set_color(Color::rgb(0.15, 0.13, 0.15));
            graphics::polygon(DrawMode::Line, &block.corners);
        }
    }

    /// Fills the light map: the ambient color, plus every light's glow
    /// wherever that light is not shadowed.
    fn draw_light_map(&self, lights: &[Light], blocks: &[Block]) {
        self.light_map.render_to(|| {
            let ambient = self.ambient;
            graphics::clear(Color::rgb(ambient, ambient, ambient * 1.3));

            graphics::set_shader(Some(&self.glow));
            for light in lights {
                if self.shadows_on {
                    // Each light starts with a clean stencil, then marks
                    // its own shadows.
                    graphics::stencil(|| {
                        // Only the far edges of a block cast shadows, so
                        // the block itself stays lit, as if light falls
                        // on its top.
                        for block in blocks {
                            graphics::shadow(light.x, light.y, &block.corners, SHADOW_LENGTH);
                        }
                    });
                    graphics::set_stencil_test(CompareMode::Equal, 0);
                }

                // Light adds up where lights overlap.
                graphics::set_blend_mode(BlendMode::Add);
                graphics::set_color(light.color);
                // A filled shape's texture coordinates run across it as
                // they do across an image, so the shader needs no image.
                graphics::rectangle(
                    DrawMode::Fill,
                    light.x - light.radius,
                    light.y - light.radius,
                    light.radius * 2.0,
                    light.radius * 2.0,
                );
                graphics::set_blend_mode(BlendMode::Alpha);
                graphics::clear_stencil_test();
            }
            graphics::set_shader(None);
        });
    }
}

impl Game for Lighting {
    fn load() -> smitten::Result<Self> {
        graphics::set_background_color(Color::BLACK);
        // Holding Up or Down keeps changing the ambient light.
        keyboard::set_key_repeat(true);
        let glow = Shader::new(GLOW)?;
        glow.send("falloff", 1.6_f32);
        Ok(Lighting {
            light_map: Canvas::new(WIDTH, HEIGHT)?,
            glow,
            heart: Image::load("heart.png")?,
            placed: Vec::new(),
            shadows_on: true,
            view: View::Lit,
            ambient: 0.12,
        })
    }

    fn draw(&mut self) {
        let lights = self.all_lights();
        let blocks = blocks();

        if self.view != View::LightMap {
            self.draw_room(&blocks);
        }
        if self.view != View::Unlit {
            self.draw_light_map(&lights, &blocks);
            // White in the light map leaves the room as it is, black hides
            // it, and colors tint it. The map has no see-through parts, so
            // Multiply works without the premultiplied blend mode.
            graphics::set_color(Color::WHITE);
            graphics::set_blend_mode(if self.view == View::LightMap {
                BlendMode::Alpha
            } else {
                BlendMode::Multiply
            });
            graphics::draw(&self.light_map, 0.0, 0.0);
            graphics::set_blend_mode(BlendMode::Alpha);
        }

        // The light sources themselves are drawn on top, unaffected by the
        // light map.
        for light in &lights {
            graphics::set_color(light.color);
            graphics::circle(DrawMode::Fill, light.x, light.y, 5.0);
        }

        graphics::set_color(Color::WHITE);
        let view = match self.view {
            View::Lit => "lit",
            View::Unlit => "unlit (L)",
            View::LightMap => "light map (M)",
        };
        graphics::print(
            &format!(
                "Click to place a light, C clears. Space: shadows {}. Up/Down: ambient {:.2}. View: {view}",
                if self.shadows_on { "on" } else { "off" },
                self.ambient,
            ),
            12.0,
            HEIGHT as f32 - 26.0,
        );
    }

    fn mouse_pressed(&mut self, x: f32, y: f32, button: MouseButton, _presses: u32) {
        if button == MouseButton::Left {
            let hue = math::random() * TAU;
            self.placed.push(Light {
                x,
                y,
                radius: 220.0,
                color: Color::rgb(
                    0.6 + 0.4 * hue.sin(),
                    0.6 + 0.4 * (hue + 2.1).sin(),
                    0.6 + 0.4 * (hue + 4.2).sin(),
                ),
            });
        }
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        match key {
            Key::Space if !is_repeat => self.shadows_on = !self.shadows_on,
            Key::L if !is_repeat => {
                self.view = if self.view == View::Unlit {
                    View::Lit
                } else {
                    View::Unlit
                }
            }
            Key::M if !is_repeat => {
                self.view = if self.view == View::LightMap {
                    View::Lit
                } else {
                    View::LightMap
                }
            }
            Key::C => self.placed.clear(),
            Key::Up => self.ambient = (self.ambient + 0.04).min(1.0),
            Key::Down => self.ambient = (self.ambient - 0.04).max(0.0),
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Lighting>(Config {
        title: "Smitten lighting".into(),
        width: WIDTH,
        height: HEIGHT,
        ..Config::default()
    });
}
