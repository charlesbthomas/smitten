//! Custom shaders.
//!
//! - The scene is drawn to a canvas, and the canvas is drawn to the
//!   screen through a shader loaded from `assets/wobble.wgsl`. This is how
//!   full-screen effects are done.
//! - The hearts on the right use a small shader written inline, which
//!   turns the picture gray by an amount sent from the game.
//! - Space switches the full-screen effect on and off. Escape quits.

use smitten::graphics::{Canvas, DrawMode, Image, Shader};
use smitten::{BlendMode, Color, Config, Filter, Game, Key, event, graphics, timer};

const WIDTH: u32 = 720;
const HEIGHT: u32 = 400;

/// Mixes the picture toward gray. `amount` goes from 0.0 to 1.0.
const GRAY: &str = "
struct Params {
    amount: f32,
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let texel = textureSample(image, image_sampler, uv) * color;
    let gray = dot(texel.rgb, vec3<f32>(0.299, 0.587, 0.114));
    return vec4<f32>(mix(texel.rgb, vec3<f32>(gray), params.amount), texel.a);
}
";

struct Shaders {
    scene: Canvas,
    wobble: Shader,
    gray: Shader,
    heart: Image,
    effect_on: bool,
}

impl Shaders {
    fn draw_scene(&self) {
        let time = timer::time() as f32;

        graphics::clear(Color::rgb(0.09, 0.1, 0.18));
        for column in 0..12 {
            let x = column as f32 * 60.0;
            let height = 120.0 + (time + column as f32 * 0.7).sin() * 60.0;
            graphics::set_color(Color::rgb(0.2, 0.3 + column as f32 * 0.04, 0.6));
            graphics::rectangle(DrawMode::Fill, x + 6.0, 400.0 - height, 48.0, height);
        }
        graphics::set_color(Color::rgb(1.0, 0.85, 0.4));
        graphics::circle(
            DrawMode::Fill,
            120.0 + (time * 0.5).sin() * 40.0,
            90.0,
            36.0,
        );

        // The same heart five times, each a little grayer than the last.
        graphics::set_color(Color::WHITE);
        graphics::set_shader(Some(&self.gray));
        for step in 0..5 {
            self.gray.send("amount", step as f32 / 4.0);
            graphics::draw_ex(
                &self.heart,
                330.0 + step as f32 * 72.0,
                60.0,
                0.0,
                8.0,
                8.0,
                0.0,
                0.0,
            );
        }
        graphics::set_shader(None);

        graphics::print("Space switches the screen effect", 16.0, 16.0);
    }
}

impl Game for Shaders {
    fn load() -> Self {
        graphics::set_default_filter(Filter::Nearest);
        let wobble = Shader::from_file("wobble.wgsl").unwrap_or_else(|error| panic!("{error}"));
        wobble.send("size", (WIDTH as f32, HEIGHT as f32));
        Shaders {
            scene: graphics::new_canvas(WIDTH, HEIGHT),
            wobble,
            gray: graphics::new_shader(GRAY),
            heart: graphics::new_image("heart.png"),
            effect_on: true,
        }
    }

    fn draw(&mut self) {
        self.scene.render_to(|| self.draw_scene());

        self.wobble.send("time", timer::time() as f32);
        self.wobble
            .send("strength", if self.effect_on { 1.0f32 } else { 0.0 });

        graphics::set_color(Color::WHITE);
        graphics::set_blend_mode(BlendMode::Premultiplied);
        graphics::set_shader(Some(&self.wobble));
        graphics::draw(&self.scene, 0.0, 0.0);
        graphics::set_shader(None);
        graphics::set_blend_mode(BlendMode::Alpha);
    }

    fn key_pressed(&mut self, key: Key, is_repeat: bool) {
        match key {
            Key::Space if !is_repeat => self.effect_on = !self.effect_on,
            Key::Escape => event::quit(),
            _ => {}
        }
    }
}

fn main() {
    smitten::run::<Shaders>(Config {
        title: "Smitten shaders".into(),
        width: WIDTH,
        height: HEIGHT,
        ..Config::default()
    });
}
