use crate::{
    error::{AppError, Result},
    render::{self, DPI, HEAD_DOTS, MAX_FILE_BYTES, RenderOptions, Rendered},
    sr_mk2::decode_binary_raster,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
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
    pub raster_pixels: Vec<u8>,
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
fn read_file_limited(f: File, max: u64) -> Result<Vec<u8>> {
    let capacity = f.metadata().map(|m| m.len().min(max) as usize).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    f.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::invalid(format!("cannot read file: {e}")))?;
    if bytes.len() as u64 > max {
        return Err(AppError::invalid("file exceeds size limit"));
    }
    Ok(bytes)
}
fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>> {
    let f = File::open(path)
        .map_err(|e| AppError::invalid(format!("cannot open {}: {e}", path.display())))?;
    read_file_limited(f, max)
}
fn read_regular_no_symlink(path: &Path, max: u64) -> Result<Vec<u8>> {
    let sym_meta = fs::symlink_metadata(path)
        .map_err(|e| AppError::invalid(format!("cannot stat raster: {e}")))?;
    if !sym_meta.file_type().is_file() {
        return Err(AppError::invalid("raster must be a regular file"));
    }
    let file = File::open(path)
        .map_err(|e| AppError::invalid(format!("cannot open {}: {e}", path.display())))?;
    let fd_meta = file
        .metadata()
        .map_err(|e| AppError::invalid(format!("cannot stat raster: {e}")))?;
    if !fd_meta.file_type().is_file() {
        return Err(AppError::invalid("raster must be a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if sym_meta.dev() != fd_meta.dev() || sym_meta.ino() != fd_meta.ino() {
            return Err(AppError::invalid("raster changed while opening"));
        }
    }
    read_file_limited(file, max)
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
    // Create both files before writing either so an existing job_path never
    // causes a full PNG write followed by deletion.
    let mut raster = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| AppError::invalid(format!("cannot create raster: {e}")))?;
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(job_path)
    {
        Ok(f) => f,
        Err(e) => {
            drop(raster);
            let _ = fs::remove_file(output);
            return Err(AppError::invalid(format!("cannot create job: {e}")));
        }
    };
    if let Err(e) = raster.write_all(&rendered.png_bytes) {
        drop(raster);
        drop(file);
        let _ = fs::remove_file(job_path);
        let _ = fs::remove_file(output);
        return Err(AppError::invalid(format!("cannot write raster: {e}")));
    }
    if let Err(e) = file.write_all(&json) {
        drop(raster);
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
    let width = render::width_with_padding(&job.options)?;
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
    let png_bytes = read_regular_no_symlink(&raster_path, MAX_FILE_BYTES)?;
    if render::sha256(&png_bytes) != job.raster.sha256 {
        return Err(AppError::invalid("raster SHA-256 mismatch"));
    }
    let decoded = decode_binary_raster(&png_bytes, Some(width), None)?;
    Ok(ValidatedJob {
        job,
        png_bytes,
        raster_pixels: decoded.pixels,
    })
}
