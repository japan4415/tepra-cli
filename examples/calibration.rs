//! Make an asymmetric calibration target without a font or an image service.
use image::{DynamicImage, Rgb, RgbImage};
use std::{error::Error, fs::OpenOptions, io::BufWriter};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or(
        "usage: cargo run --example calibration -- output.png [content-length-mm: 10..100]",
    )?;
    let length: f64 = args
        .next()
        .map(|s| s.to_string_lossy().parse())
        .transpose()?
        .unwrap_or(50.0);
    if !length.is_finite() || !(10.0..=100.0).contains(&length) || args.next().is_some() {
        return Err("expected content length between 10 and 100 mm".into());
    }
    let width = (length * 360.0 / 25.4).round() as u32;
    let ticks: Vec<_> = (0..=((length / 5.0).floor() as u32))
        .map(|n| (f64::from(n) * 5.0 * 360.0 / 25.4).round() as u32)
        .collect();
    let mut image = RgbImage::from_pixel(width, 256, Rgb([255, 255, 255]));
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let border = !(3..width - 3).contains(&x) || !(3..253).contains(&y);
        let top_left = (16..80).contains(&x) && (16..48).contains(&y);
        let bottom_right = (width - 59..width - 19).contains(&x) && (190..240).contains(&y);
        let tick = ticks.iter().any(|&at| (at..at + 3).contains(&x));
        let ruler = (3..width - 3).contains(&x) && y >= 128 && y < if tick { 180 } else { 131 };
        if border || top_left || bottom_right || ruler {
            *pixel = Rgb([0, 0, 0]);
        }
    }
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(file);
    DynamicImage::ImageRgb8(image).write_to(&mut writer, image::ImageFormat::Png)?;
    Ok(())
}
