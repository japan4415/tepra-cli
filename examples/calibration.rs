//! Make an asymmetric calibration target without a font or an image service.
use image::{DynamicImage, Rgb, RgbImage};
use std::{error::Error, fs::OpenOptions, io::BufWriter};

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: cargo run --example calibration -- output.png")?;
    let mut image = RgbImage::from_pixel(709, 256, Rgb([255, 255, 255]));
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let border = !(3..706).contains(&x) || !(3..253).contains(&y);
        let top_left = (16..80).contains(&x) && (16..48).contains(&y);
        let bottom_right = (650..690).contains(&x) && (190..240).contains(&y);
        let ruler = (120..600).contains(&x) && y >= 128 && y < if x % 71 < 3 { 180 } else { 131 };
        if border || top_left || bottom_right || ruler {
            *pixel = Rgb([0, 0, 0]);
        }
    }
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(file);
    DynamicImage::ImageRgb8(image).write_to(&mut writer, image::ImageFormat::Png)?;
    Ok(())
}
