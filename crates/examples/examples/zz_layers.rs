use smitten::graphics::{self, ArrayImage, ImageData, SpriteBatch};
use smitten::{Color, Config, Game, event};

fn solid(color: Color) -> ImageData {
    let mut data = ImageData::new(16, 16);
    data.map_pixels(|_, _, _| color);
    data
}

struct T { batch: SpriteBatch, frames: u32 }

impl Game for T {
    fn load() -> Self {
        let image = ArrayImage::from_image_data(&[
            solid(Color::rgb(1.0, 0.0, 0.0)),
            solid(Color::rgb(0.0, 1.0, 0.0)),
            solid(Color::rgb(0.0, 0.0, 1.0)),
        ]).unwrap();
        let mut batch = graphics::new_array_sprite_batch(&image, 8);
        for (i, layer) in [0, 0, 1, 2, 2, 1].into_iter().enumerate() {
            batch.add_layer(layer, i as f32 * 20.0, 0.0);
        }
        batch.set_layer(1, 2, 20.0, 20.0);
        T { batch, frames: 0 }
    }
    fn draw(&mut self) {
        graphics::clear(Color::BLACK);
        graphics::draw_sprite_batch(&self.batch, 10.0, 10.0);
        self.frames += 1;
        if self.frames == 3 {
            graphics::capture_screenshot(|p| {
                let px = |x, y| p.pixel(x, y);
                println!("{:?} {:?} {:?} {:?} {:?} {:?} {:?}", px(15, 15), px(35, 15), px(35, 35), px(55, 15), px(75, 15), px(95, 15), px(115, 15));
                event::quit();
            });
        }
    }
}

fn main() { smitten::run::<T>(Config { width: 200, height: 100, ..Config::default() }); }
