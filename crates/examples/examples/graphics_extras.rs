//! Text objects, transforms, mipmaps, array and cube images, the color
//! mask, wireframe and the graphics statistics.
//!
//! Move the mouse over the spinning square to light it up: the square is
//! placed by a `Transform`, and its inverse turns the mouse position into
//! the square's own coordinates. Press W for wireframe and Escape to quit.

use smitten::graphics::{
    self, Align, ArrayImage, CubeImage, DrawMode, Image, ImageData, ImageSettings, Shader, Text,
};
use smitten::math::{self, Transform};
use smitten::{Color, Config, Game, Key, event, mouse};

/// Samples a cube image by direction, sweeping once around it from left
/// to right, and an array image by layer, a third of the width each.
const SHADER: &str = "
var sky: texture_cube<f32>;
var tiles: texture_2d_array<f32>;

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    if (uv.y < 0.5) {
        let angle = uv.x * 6.2831853;
        let direction = vec3<f32>(sin(angle), 0.0, cos(angle));
        return textureSample(sky, sky_sampler, direction) * color;
    }
    let layer = i32(uv.x * 3.0);
    return textureSample(tiles, tiles_sampler, vec2<f32>(fract(uv.x * 3.0), uv.y * 2.0 - 1.0), layer) * color;
}
";

struct Extras {
    title: Text,
    labels: Text,
    square: Transform,
    angle: f32,
    plain: Image,
    mipmapped: Image,
    tiles: ArrayImage,
    shader: Shader,
    // Kept alive while the shader holds it, though the shader keeps its
    // own handle too.
    _sky: CubeImage,
}

/// A 16 by 16 tile in `color`, striped so scaling shows.
fn tile(color: Color) -> ImageData {
    let mut data = ImageData::new(16, 16);
    let dark = Color::rgb(color.r * 0.5, color.g * 0.5, color.b * 0.5);
    data.map_pixels(|x, y, _| if (x + y) % 8 < 4 { color } else { dark });
    data
}

impl Game for Extras {
    fn load() -> Self {
        graphics::set_background_color(Color::rgb(0.08, 0.09, 0.12));

        // Laid out once, then drawn every frame for the cost of an image.
        let big = graphics::new_default_font(24.0);
        let title = graphics::new_text(&big, "Graphics extras");
        let small = graphics::new_default_font(13.0);
        let mut labels = graphics::new_text(&small, "");
        labels.add("Hover the square", 40.0, 250.0);
        labels.add_f("Plain / mipmapped", 220.0, Align::Center, 260.0, 250.0);
        labels.add("Array image layers", 540.0, 250.0);
        labels.add_f(
            "Cube image by direction, then array image by layer, read in a shader",
            360.0,
            Align::Left,
            40.0,
            470.0,
        );
        labels.add("Color mask: no red", 560.0, 380.0);

        // A fine checkerboard shimmers when drawn small, unless it has
        // mipmaps to draw from.
        let mut checks = ImageData::new(256, 256);
        checks.map_pixels(|x, y, _| {
            if (x + y) % 2 == 0 {
                Color::WHITE
            } else {
                Color::BLACK
            }
        });
        let plain = Image::from_image_data(&checks).expect("the GPU took the image");
        let mipmapped = Image::from_image_data_with(&checks, ImageSettings { mipmaps: true })
            .expect("the GPU took the image");

        let red = Color::rgb(0.9, 0.25, 0.2);
        let green = Color::rgb(0.3, 0.8, 0.35);
        let blue = Color::rgb(0.25, 0.45, 0.95);
        let tiles = ArrayImage::from_image_data(&[tile(red), tile(green), tile(blue)])
            .expect("the GPU took the array image");
        let sky = CubeImage::from_image_data(&[
            tile(red),
            tile(Color::rgb(0.2, 0.8, 0.8)),
            tile(green),
            tile(Color::rgb(0.8, 0.3, 0.8)),
            tile(blue),
            tile(Color::rgb(0.9, 0.85, 0.3)),
        ])
        .expect("the GPU took the cube image");

        let shader = graphics::new_shader(SHADER);
        shader.send_texture("sky", &sky);
        shader.send_texture("tiles", &tiles);

        Extras {
            title,
            labels,
            square: Transform::new(),
            angle: 0.0,
            plain,
            mipmapped,
            tiles,
            shader,
            _sky: sky,
        }
    }

    fn update(&mut self, dt: f32) {
        self.angle += dt * 0.8;
        // Centered on (130, 160), turning, and leaning over with a shear.
        self.square = math::new_transform(130.0, 160.0, self.angle, 1.0, 1.0, 40.0, 40.0, 0.3, 0.0);
    }

    fn key_pressed(&mut self, key: Key, _is_repeat: bool) {
        match key {
            Key::W => graphics::set_wireframe(!graphics::is_wireframe()),
            Key::Escape => event::quit(),
            _ => {}
        }
    }

    fn draw(&mut self) {
        graphics::set_color(Color::WHITE);
        graphics::draw_text(&self.title, 40.0, 30.0);
        graphics::set_color(Color::rgb(0.7, 0.72, 0.8));
        graphics::draw_text(&self.labels, 0.0, 0.0);

        // The mouse in the square's own coordinates.
        let (mx, my) = mouse::position();
        let (lx, ly) = self.square.inverse_transform_point(mx, my);
        let inside = (0.0..80.0).contains(&lx) && (0.0..80.0).contains(&ly);
        graphics::push();
        graphics::apply_transform(&self.square);
        graphics::set_color(if inside {
            Color::rgb(1.0, 0.8, 0.3)
        } else {
            Color::rgb(0.4, 0.45, 0.6)
        });
        graphics::rectangle(DrawMode::Fill, 0.0, 0.0, 80.0, 80.0);
        graphics::pop();

        graphics::set_color(Color::WHITE);
        graphics::draw_ex(&self.plain, 300.0, 120.0, 0.0, 0.4, 0.4, 0.0, 0.0);
        graphics::draw_ex(&self.mipmapped, 420.0, 120.0, 0.0, 0.4, 0.4, 0.0, 0.0);

        for layer in 0..self.tiles.layer_count() {
            let x = 560.0 + layer as f32 * 60.0;
            graphics::draw_layer_ex(&self.tiles, layer, x, 140.0, 0.0, 3.0, 3.0, 0.0, 0.0);
        }

        graphics::set_shader(Some(&self.shader));
        graphics::rectangle(DrawMode::Fill, 40.0, 320.0, 480.0, 140.0);
        graphics::set_shader(None);

        // White drawn without its red channel leaves cyan.
        graphics::set_color_mask(false, true, true, true);
        graphics::rectangle(DrawMode::Fill, 580.0, 300.0, 140.0, 70.0);
        graphics::set_color_mask(true, true, true, true);

        let stats = graphics::stats();
        graphics::set_color(Color::rgb(0.6, 0.62, 0.7));
        graphics::print(
            &format!(
                "{} draw calls ({} batched), {} images, {} fonts, {:.1} MB of textures. W: wireframe",
                stats.draw_calls,
                stats.draw_calls_batched,
                stats.images,
                stats.fonts,
                stats.texture_memory as f32 / 1_000_000.0
            ),
            40.0,
            560.0,
        );
    }
}

fn main() {
    smitten::run::<Extras>(Config {
        title: "Graphics extras".to_string(),
        ..Config::default()
    });
}
