use crate::{
    error::{AppError, Result},
    render::{self, DPI, HEAD_DOTS, MAX_FILE_BYTES, RenderOptions, Rendered},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Raster {
    pub filename: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub schema_version: u32,
    pub model: String,
    pub profile_version: u32,
    pub tape_width_mm: u32,
    pub dpi: u32,
    pub content_length_mm: f64,
    pub raster: Raster,
    pub source_sha256: String,
    pub options: RenderOptions,
}
#[derive(Debug)]
pub struct ValidatedJob {
    pub job: Job,
    pub png_bytes: Vec<u8>,
}
fn filename(path: &Path) -> Result<String> {
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| AppError::invalid("raster filename must be UTF-8"))?;
    if name.is_empty() {
        return Err(AppError::invalid("missing raster filename"));
    }
    Ok(name.to_owned())
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>> {
    let f = fs::File::open(path)
        .map_err(|e| AppError::invalid(format!("cannot open {}: {e}", path.display())))?;
    let mut bytes = Vec::new();
    f.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::invalid(format!("cannot read file: {e}")))?;
    if bytes.len() as u64 > max {
        return Err(AppError::invalid("file exceeds size limit"));
    }
    Ok(bytes)
}
pub fn write_job(rendered: &Rendered, output: &Path, job_path: &Path) -> Result<Job> {
    let out_parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let job_parent = job_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let out_dir = fs::canonicalize(out_parent)
        .map_err(|e| AppError::invalid(format!("invalid output directory: {e}")))?;
    let job_dir = fs::canonicalize(job_parent)
        .map_err(|e| AppError::invalid(format!("invalid job directory: {e}")))?;
    if out_dir != job_dir {
        return Err(AppError::invalid("raster and job must be adjacent"));
    }
    let raster_name = filename(output)?;
    if raster_name == filename(job_path)? {
        return Err(AppError::invalid("raster and job paths must differ"));
    }
    let job = Job {
        schema_version: 1,
        model: "sr-mk2".into(),
        profile_version: 1,
        tape_width_mm: 24,
        dpi: DPI,
        content_length_mm: rendered.options.content_length_mm,
        raster: Raster {
            filename: raster_name,
            width: rendered.width,
            height: rendered.height,
            sha256: render::sha256(&rendered.png_bytes),
        },
        source_sha256: rendered.source_sha256.clone(),
        options: rendered.options.clone(),
    };
    let json = serde_json::to_vec_pretty(&job)
        .map_err(|e| AppError::invalid(format!("cannot serialize job: {e}")))?;
    let mut raster = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| AppError::invalid(format!("cannot create raster: {e}")))?;
    if let Err(e) = raster.write_all(&rendered.png_bytes) {
        drop(raster);
        let _ = fs::remove_file(output);
        return Err(AppError::invalid(format!("cannot write raster: {e}")));
    }
    drop(raster);
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(job_path)
    {
        Ok(f) => f,
        Err(e) => {
            let _ = fs::remove_file(output);
            return Err(AppError::invalid(format!("cannot create job: {e}")));
        }
    };
    if let Err(e) = file.write_all(&json) {
        drop(file);
        let _ = fs::remove_file(job_path);
        let _ = fs::remove_file(output);
        return Err(AppError::invalid(format!("cannot write job: {e}")));
    }
    Ok(job)
}
pub fn load_job(path: &Path) -> Result<ValidatedJob> {
    let json = read_limited(path, 1024 * 1024)?;
    let job: Job = serde_json::from_slice(&json)
        .map_err(|e| AppError::invalid(format!("invalid job: {e}")))?;
    if job.schema_version != 1
        || job.model != "sr-mk2"
        || job.profile_version != 1
        || job.tape_width_mm != 24
        || job.dpi != DPI
    {
        return Err(AppError::unsupported("unsupported job model or profile"));
    }
    let width = render::width_for_length(job.content_length_mm)?;
    if job.content_length_mm != job.options.content_length_mm
        || job.raster.width != width
        || job.raster.height != HEAD_DOTS
        || !valid_hash(&job.raster.sha256)
        || !valid_hash(&job.source_sha256)
    {
        return Err(AppError::invalid(
            "invalid job dimensions, options, or hashes",
        ));
    }
    let mut parts = Path::new(&job.raster.filename).components();
    if !matches!(parts.next(), Some(Component::Normal(_)))
        || parts.next().is_some()
        || job.raster.filename.contains(['/', '\\'])
    {
        return Err(AppError::invalid(
            "raster filename must be a single adjacent filename",
        ));
    }
    let raster_path = path
        .parent()
        .unwrap_or(Path::new("."))
        .join(&job.raster.filename);
    let meta = fs::symlink_metadata(&raster_path)
        .map_err(|e| AppError::invalid(format!("cannot stat raster: {e}")))?;
    if !meta.file_type().is_file() {
        return Err(AppError::invalid("raster must be a regular file"));
    }
    let png_bytes = read_limited(&raster_path, MAX_FILE_BYTES)?;
    if render::sha256(&png_bytes) != job.raster.sha256 {
        return Err(AppError::invalid("raster SHA-256 mismatch"));
    }
    let decoder = png::Decoder::new(std::io::BufReader::new(std::io::Cursor::new(&png_bytes)));
    let mut reader = decoder
        .read_info()
        .map_err(|e| AppError::invalid(format!("invalid raster PNG: {e}")))?;
    let info = reader.info();
    if info.width != width
        || info.height != HEAD_DOTS
        || info.color_type != png::ColorType::Grayscale
        || info.bit_depth != png::BitDepth::Eight
        || info.trns.is_some()
        || info.animation_control.is_some()
        || info.frame_control.is_some()
        || !matches!(
            info.pixel_dims,
            Some(png::PixelDimensions {
                xppu: 14173,
                yppu: 14173,
                unit: png::Unit::Meter
            })
        )
    {
        return Err(AppError::invalid(
            "raster PNG format, dimensions or resolution mismatch",
        ));
    }
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| AppError::invalid("invalid raster buffer size"))?;
    if size != (width * HEAD_DOTS) as usize {
        return Err(AppError::invalid("invalid raster buffer size"));
    }
    let mut pixels = vec![0; size];
    reader
        .next_frame(&mut pixels)
        .map_err(|e| AppError::invalid(format!("invalid raster PNG: {e}")))?;
    if pixels.iter().any(|&v| v != 0 && v != 255) {
        return Err(AppError::invalid("raster is not binary"));
    }
    Ok(ValidatedJob { job, png_bytes })
}
