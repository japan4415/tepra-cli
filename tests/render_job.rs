use image::{Rgba, RgbaImage};
use tepra_cli::{
    job::{load_job, write_job},
    render::{Dither, Fit, RenderOptions, render, width_for_length},
};

fn options() -> RenderOptions {
    RenderOptions {
        content_length_mm: 50.0,
        padding_right_mm: 0.0,
        fit: Fit::Contain,
        dither: Dither::Threshold,
        threshold: 128,
    }
}
fn pixels(bytes: &[u8]) -> image::GrayImage {
    image::load_from_memory(bytes).unwrap().to_luma8()
}
#[test]
fn right_padding_preserves_artwork_and_stays_white_for_all_render_modes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    let source = RgbaImage::from_fn(37, 13, |x, y| {
        let v = ((x * 7 + y * 11) % 256) as u8;
        Rgba([v, v, v, 255])
    });
    source.save(&input).unwrap();
    for fit in [Fit::Contain, Fit::Cover] {
        for dither in [Dither::Threshold, Dither::FloydSteinberg] {
            let mut opts = options();
            opts.fit = fit;
            opts.dither = dither;
            let original = pixels(&render(&input, &opts).unwrap().png_bytes);
            opts.padding_right_mm = 1.0;
            let padded = pixels(&render(&input, &opts).unwrap().png_bytes);
            assert_eq!(padded.dimensions(), (723, 256));
            for (x, y, pixel) in original.enumerate_pixels() {
                assert_eq!(padded.get_pixel(x, y), pixel);
            }
            assert!(
                padded
                    .enumerate_pixels()
                    .filter(|(x, _, _)| *x >= 709)
                    .all(|(_, _, p)| p[0] == 255)
            );
        }
    }
}

#[test]
fn padding_validation_and_job_compatibility() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]))
        .save(&input)
        .unwrap();
    let mut opts = options();
    for value in [-1.0, f64::NAN, f64::INFINITY, 951.0] {
        opts.padding_right_mm = value;
        assert!(render(&input, &opts).is_err());
    }
    for padding in [0.0, 0.001, 1.0, 950.0] {
        opts.padding_right_mm = padding;
        let rendered = render(&input, &opts).unwrap();
        let raster = dir.path().join(format!("raster-{padding}.png"));
        let job = dir.path().join(format!("job-{padding}.json"));
        write_job(&rendered, &raster, &job).unwrap();
        let mut doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&job).unwrap()).unwrap();
        if padding == 0.0 {
            // Old schema-1 jobs have no padding field; serialization also stays unchanged.
            assert!(doc["options"].get("padding_right_mm").is_none());
        }
        let loaded = load_job(&job).unwrap();
        assert_eq!(loaded.job.options.padding_right_mm, padding);
        assert_eq!(loaded.png_bytes, rendered.png_bytes);
        doc["options"]["padding_right_mm"] = 2.0.into();
        std::fs::write(&job, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert!(load_job(&job).is_err());
        doc["options"]["padding_right_mm"] = (-1.0).into();
        std::fs::write(&job, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert!(load_job(&job).is_err());
    }
}

#[test]
fn dimensions_alpha_fit_and_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    let mut source = RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]));
    source.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
    source.save(&input).unwrap();
    let rendered = render(&input, &options()).unwrap();
    assert_eq!((rendered.width, rendered.height), (709, 256));
    let image = pixels(&rendered.png_bytes);
    assert_eq!(image.get_pixel(0, 128)[0], 255);
    assert_eq!(image.get_pixel(240, 20)[0], 255);
    assert_eq!(image.get_pixel(450, 200)[0], 0);
    let dec = png::Decoder::new(std::io::BufReader::new(std::io::Cursor::new(
        &rendered.png_bytes,
    )));
    assert_eq!(
        dec.read_info().unwrap().info().pixel_dims.unwrap().xppu,
        14173
    );
    let mut cover = options();
    cover.fit = Fit::Cover;
    let covered = pixels(&render(&input, &cover).unwrap().png_bytes);
    assert_eq!(covered.get_pixel(708, 255)[0], 0);
}
#[test]
fn dither_and_limits() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    RgbaImage::from_pixel(4, 4, Rgba([128, 128, 128, 255]))
        .save(&input)
        .unwrap();
    let mut opts = options();
    opts.fit = Fit::Cover;
    let plain = pixels(&render(&input, &opts).unwrap().png_bytes);
    assert!(plain.pixels().all(|p| p[0] == 255));
    opts.dither = Dither::FloydSteinberg;
    let diffused = pixels(&render(&input, &opts).unwrap().png_bytes);
    assert!(diffused.pixels().any(|p| p[0] == 0));
    assert!(diffused.pixels().any(|p| p[0] == 255));
    for v in [0.0, -1.0, f64::NAN, f64::INFINITY, 1000.1, 0.001] {
        assert!(width_for_length(v).is_err());
    }
    std::fs::write(&input, vec![0; 20 * 1024 * 1024 + 1]).unwrap();
    assert!(render(&input, &opts).is_err());
}
#[test]
fn job_integrity_and_no_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]))
        .save(&input)
        .unwrap();
    let rendered = render(&input, &options()).unwrap();
    let raster = dir.path().join("raster.png");
    let job = dir.path().join("job.json");
    write_job(&rendered, &raster, &job).unwrap();
    assert_eq!(load_job(&job).unwrap().png_bytes, rendered.png_bytes);
    assert!(write_job(&rendered, &raster, &job).is_err());
    let mut doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&job).unwrap()).unwrap();
    doc["raster"]["filename"] = "../other.png".into();
    std::fs::write(&job, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert!(load_job(&job).is_err());
    doc["raster"]["filename"] = "raster.png".into();
    doc["model"] = "other".into();
    std::fs::write(&job, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert!(load_job(&job).is_err());
    doc["model"] = "sr-mk2".into();
    std::fs::write(&job, serde_json::to_vec(&doc).unwrap()).unwrap();
    std::fs::write(&raster, b"tampered").unwrap();
    assert!(load_job(&job).is_err());
}

#[test]
fn cover_preserves_center_and_aspect_ratio() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    let mut img = RgbaImage::from_pixel(2, 2, Rgba([255, 255, 255, 255]));
    for x in 0..2 {
        img.put_pixel(x, 1, Rgba([0, 0, 0, 255]));
    }
    img.save(&input).unwrap();
    let mut opts = options();
    opts.fit = Fit::Cover;
    let out = pixels(&render(&input, &opts).unwrap().png_bytes);
    assert_eq!(out.get_pixel(354, 30)[0], 255);
    assert_eq!(out.get_pixel(354, 225)[0], 0);
    assert_eq!(out.get_pixel(354, 120)[0], 255);
    assert_eq!(out.get_pixel(354, 136)[0], 0);
    for y in 0..2 {
        img.put_pixel(0, y, Rgba([255, 255, 255, 255]));
        img.put_pixel(1, y, Rgba([0, 0, 0, 255]));
    }
    img.save(&input).unwrap();
    opts.content_length_mm = 4.0;
    let out = pixels(&render(&input, &opts).unwrap().png_bytes);
    let mid = out.width() / 2;
    assert_eq!(out.get_pixel(3, 128)[0], 255);
    assert_eq!(out.get_pixel(out.width() - 4, 128)[0], 0);
    assert_eq!(out.get_pixel(mid - 4, 128)[0], 255);
    assert_eq!(out.get_pixel(mid + 4, 128)[0], 0);
    RgbaImage::from_pixel(1, 10_000, Rgba([0, 0, 0, 255]))
        .save(&input)
        .unwrap();
    opts.content_length_mm = 50.0;
    assert!(
        render(&input, &opts)
            .unwrap_err()
            .message
            .contains("work limit")
    );
}

#[test]
fn job_rejects_transparency_and_animation_even_with_matching_hash() {
    use tepra_cli::render::sha256;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]))
        .save(&input)
        .unwrap();
    let rendered = render(&input, &options()).unwrap();
    let raster = dir.path().join("raster.png");
    let job = dir.path().join("job.json");
    write_job(&rendered, &raster, &job).unwrap();
    for animated in [false, true] {
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, rendered.width, rendered.height);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_pixel_dims(Some(png::PixelDimensions {
                xppu: 14173,
                yppu: 14173,
                unit: png::Unit::Meter,
            }));
            if animated {
                enc.set_animated(1, 0).unwrap();
            } else {
                enc.set_trns(vec![0, 0]);
            }
            let mut writer = enc.write_header().unwrap();
            writer
                .write_image_data(&vec![0; (rendered.width * rendered.height) as usize])
                .unwrap();
        }
        std::fs::write(&raster, &bytes).unwrap();
        let mut doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&job).unwrap()).unwrap();
        doc["raster"]["sha256"] = sha256(&bytes).into();
        std::fs::write(&job, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert!(load_job(&job).is_err(), "animated={animated}");
    }
}
