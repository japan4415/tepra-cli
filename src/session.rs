//! A per-user process lock and durable, conservative print-attempt records.
use crate::error::{AppError, Result};
use fs2::FileExt;
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct PrintSession {
    _lock: File,
    record: File,
    pub attempt_id: String,
    pub record_path: PathBuf,
}

pub fn state_directory() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")));
    base.filter(|p| p.is_absolute())
        .map(|p| p.join("tepra-cli"))
        .ok_or_else(|| {
            AppError::new(
                "state_directory_unavailable",
                "No absolute user state directory is available",
                4,
            )
        })
}

impl PrintSession {
    pub fn begin(directory: &Path, job_id: &str, device_id: &str) -> Result<Self> {
        fs::create_dir_all(directory).map_err(record_error)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("print.lock"))
            .map_err(record_error)?;
        FileExt::try_lock_exclusive(&lock).map_err(|e| {
            AppError::new(
                "printer_busy",
                format!("Another tepra process is printing, or the lock is unavailable: {e}"),
                4,
            )
        })?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| AppError::new("clock_error", e.to_string(), 4))?
            .as_nanos();
        let attempt_id = format!("{stamp}-{}", std::process::id());
        let record_path = directory.join(format!("{attempt_id}.jsonl"));
        let record = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&record_path)
            .map_err(record_error)?;
        let mut session = Self {
            _lock: lock,
            record,
            attempt_id,
            record_path,
        };
        session.append(&json!({"event":"attempt_started", "attempt_id":session.attempt_id,
            "job_id":job_id,"device_id":device_id,"state":"unknown",
            "note":"If no result follows, the process stopped before it recorded a final outcome. Do not automatically reprint."}))?;
        Ok(session)
    }

    pub fn append(&mut self, value: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.record, value).map_err(record_error)?;
        self.record.write_all(b"\n").map_err(record_error)?;
        self.record.sync_all().map_err(record_error)
    }
}

fn record_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(
        "print_record_unavailable",
        format!("Cannot persist print attempt: {error}"),
        4,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_serializes_attempts_and_records_survive() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = PrintSession::begin(dir.path(), "job", "printer").unwrap();
        let err = PrintSession::begin(dir.path(), "second", "printer")
            .err()
            .unwrap();
        assert_eq!(err.code, "printer_busy");
        first
            .append(&json!({"event":"result", "state":"completed"}))
            .unwrap();
        let path = first.record_path.clone();
        drop(first);
        let record = fs::read_to_string(path).unwrap();
        let rows: Vec<Value> = record
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(rows[0]["state"], "unknown");
        assert_eq!(rows[1]["state"], "completed");
        assert!(PrintSession::begin(dir.path(), "second", "printer").is_ok());
    }
}
