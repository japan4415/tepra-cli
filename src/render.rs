use crate::error::{AppError, Result};
use clap::ValueEnum;
use image::{ImageReader, Rgb, RgbImage, imageops};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
};

pub const DPI: u32 = 360;
pub const HEAD_DOTS: u32 = 256;
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 40_000_000;
pub const MAX_WORK_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Fit {
    Contain,
    Cover,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Dither {
    Threshold,
    FloydSteinberg,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderOptions {
    pub content_length_mm: f64,
    pub fit: Fit,
    pub dither: Dither,
    pub threshold: u8,
}
#[derive(Debug)]
pub struct Rendered {
    pub png_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub source_sha256: String,
    pub options: RenderOptions,
}
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn width_for_length(mm: f64) -> Result<u32> {
    if !mm.is_finite() || mm <= 0.0 || mm > 1000.0 {
        return Err(AppError::invalid(
            "content length must be greater than 0 and at most 1000 mm",
        ));
    }
    let dots = (mm * f64::from(DPI) / 25.4).round();
    if dots < 1.0 {
        return Err(AppError::invalid("content length rounds to zero dots"));
    }
    Ok(dots as u32)
}
pub fn render(input: &Path, options: &RenderOptions) -> Result<Rendered> {
    let width = width_for_length(options.content_length_mm)?;
    let file = fs::File::open(input)
        .map_err(|e| AppError::invalid(format!("cannot read input PNG: {e}")))?;
    let mut data = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| AppError::invalid(format!("cannot read input PNG: {e}")))?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err(AppError::invalid("input exceeds 20 MiB"));
    }
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(AppError::invalid("input must be PNG"));
    }
    let reader = ImageReader::with_format(Cursor::new(&data), image::ImageFormat::Png);
    let (iw, ih) = reader
        .into_dimensions()
        .map_err(|e| AppError::invalid(format!("invalid PNG: {e}")))?;
    let pixels = u64::from(iw) * u64::from(ih);
    if iw == 0 || ih == 0 || pixels > MAX_PIXELS {
        return Err(AppError::invalid("PNG dimensions exceed limit"));
    }
    let out_pixels = u64::from(width) * u64::from(HEAD_DOTS);
    let scale = match options.fit {
        Fit::Contain => {
            (f64::from(width) / f64::from(iw)).min(f64::from(HEAD_DOTS) / f64::from(ih))
        }
        Fit::Cover => (f64::from(width) / f64::from(iw)).max(f64::from(HEAD_DOTS) / f64::from(ih)),
    };
    let scaled = |dimension: u32| -> u64 {
        let value = f64::from(dimension) * scale;
        match options.fit {
            Fit::Contain => value.round().max(1.0) as u64,
            Fit::Cover => value.ceil().max(1.0) as u64,
        }
    };
    let (resize_width, resize_height) = (scaled(iw), scaled(ih));
    // image's Lanczos resizer creates an RGBA f32 intermediate of input width
    // by output height; include it even when the final output is much smaller.
    if pixels
        .saturating_mul(16)
        .saturating_add(out_pixels.saturating_mul(20))
        .saturating_add(resize_width.saturating_mul(resize_height).saturating_mul(3))
        .saturating_add(
            u64::from(iw)
                .saturating_mul(resize_height)
                .saturating_mul(16),
        )
        .saturating_add(data.len() as u64)
        > MAX_WORK_BYTES
        || resize_width > u64::from(u32::MAX)
        || resize_height > u64::from(u32::MAX)
    {
        return Err(AppError::invalid(
            "PNG conversion exceeds 256 MiB work limit",
        ));
    }
    let mut reader = ImageReader::with_format(Cursor::new(&data), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_WORK_BYTES);
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(|e| AppError::invalid(format!("cannot decode PNG: {e}")))?
        .to_rgba8();
    let mut rgb = RgbImage::new(iw, ih);
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = u32::from(p[3]);
        let blend = |c: u8| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
        rgb.put_pixel(x, y, Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
    }
    drop(rgba);
    let mut canvas = RgbImage::from_pixel(width, HEAD_DOTS, Rgb([255, 255, 255]));
    match options.fit {
        Fit::Contain => {
            let w = (resize_width as u32).min(width);
            let h = (resize_height as u32).min(HEAD_DOTS);
            let resized = imageops::resize(&rgb, w, h, imageops::FilterType::Lanczos3);
            imageops::replace(
                &mut canvas,
                &resized,
                i64::from((width - w) / 2),
                i64::from((HEAD_DOTS - h) / 2),
            );
        }
        Fit::Cover => {
            let w = (resize_width as u32).max(width);
            let h = (resize_height as u32).max(HEAD_DOTS);
            let resized = imageops::resize(&rgb, w, h, imageops::FilterType::Lanczos3);
            canvas = imageops::crop_imm(
                &resized,
                (w - width) / 2,
                (h - HEAD_DOTS) / 2,
                width,
                HEAD_DOTS,
            )
            .to_image();
        }
    }
    let mut gray: Vec<f32> = canvas
        .pixels()
        .map(|p| 0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
        .collect();
    let mut mono = vec![255u8; out_pixels as usize];
    let w = width as usize;
    for y in 0..HEAD_DOTS as usize {
        for x in 0..w {
            let i = y * w + x;
            let old = gray[i].clamp(0.0, 255.0);
            let value = if old < f32::from(options.threshold) {
                0
            } else {
                255
            };
            mono[i] = value;
            if options.dither == Dither::FloydSteinberg {
                let e = old - f32::from(value);
                if x + 1 < w {
                    gray[i + 1] += e * 7.0 / 16.0;
                }
                if y + 1 < HEAD_DOTS as usize {
                    if x > 0 {
                        gray[i + w - 1] += e * 3.0 / 16.0;
                    }
                    gray[i + w] += e * 5.0 / 16.0;
                    if x + 1 < w {
                        gray[i + w + 1] += e / 16.0;
                    }
                }
            }
        }
    }
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width, HEAD_DOTS);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_pixel_dims(Some(png::PixelDimensions {
            xppu: 14173,
            yppu: 14173,
            unit: png::Unit::Meter,
        }));
        let mut writer = encoder
            .write_header()
            .map_err(|e| AppError::invalid(format!("PNG encoding failed: {e}")))?;
        writer
            .write_image_data(&mono)
            .map_err(|e| AppError::invalid(format!("PNG encoding failed: {e}")))?;
    }
    Ok(Rendered {
        png_bytes,
        width,
        height: HEAD_DOTS,
        source_sha256: sha256(&data),
        options: options.clone(),
    })
}
