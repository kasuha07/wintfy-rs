use crate::{config, platform::paths, util::redact::redact_secret};
use log::{LevelFilter, Log, Metadata, Record};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LOG_BYTES: u64 = 1024 * 1024;
const LOG_BACKUPS: usize = 3;
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LEVEL: AtomicUsize = AtomicUsize::new(LevelFilter::Info as usize);

pub struct FileLogger {
    path: PathBuf,
    file: Mutex<File>,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= current_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let Ok(mut file) = self.file.lock() else {
            return;
        };
        let ts = timestamp_seconds();
        let msg = redact_secret(&record.args().to_string());
        let _ = writeln!(
            file,
            "{ts} {:<5} [{}] {msg}",
            record.level(),
            record.target()
        );
        let _ = file.flush();
        if file.metadata().map(|m| m.len()).unwrap_or(0) > MAX_LOG_BYTES {
            drop(file);
            let _ = rotate_logs(&self.path);
            if let Ok(new_file) = open_log_file(&self.path)
                && let Ok(mut guard) = self.file.lock()
            {
                *guard = new_file;
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

pub fn init(level_name: &str) -> Result<PathBuf, String> {
    let path = paths::log_file()?;
    let level = config::parse_level(level_name).unwrap_or(LevelFilter::Info);
    if let Some(existing) = LOG_PATH.get() {
        set_level(level);
        return Ok(existing.clone());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create log directory {}: {err}", parent.display()))?;
    }
    rotate_logs_if_needed(&path)?;
    let file = open_log_file(&path)?;
    let logger = FileLogger {
        path: path.clone(),
        file: Mutex::new(file),
    };
    match log::set_boxed_logger(Box::new(logger)) {
        Ok(()) => {
            let _ = LOG_PATH.set(path.clone());
        }
        Err(_) => return Ok(path),
    }
    set_level(level);
    Ok(path)
}

pub fn init_fallback() -> Result<PathBuf, String> {
    init("info")
}

fn open_log_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| format!("failed to open log file {}: {err}", path.display()))
}

fn rotate_logs_if_needed(path: &Path) -> Result<(), String> {
    if path.metadata().map(|m| m.len()).unwrap_or(0) > MAX_LOG_BYTES {
        rotate_logs(path)?;
    }
    Ok(())
}

fn rotate_logs(path: &Path) -> Result<(), String> {
    for index in (1..=LOG_BACKUPS).rev() {
        let from = backup_path(path, index);
        let to = backup_path(path, index + 1);
        if from.exists() {
            if index == LOG_BACKUPS {
                let _ = fs::remove_file(&from);
            } else {
                let _ = fs::rename(&from, &to);
            }
        }
    }
    if path.exists() {
        fs::rename(path, backup_path(path, 1))
            .map_err(|err| format!("failed to rotate log {}: {err}", path.display()))?;
    }
    Ok(())
}

fn backup_path(path: &Path, index: usize) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{index}"));
    path.with_file_name(name)
}

fn timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn set_level(level: LevelFilter) {
    LOG_LEVEL.store(level as usize, Ordering::Relaxed);
    log::set_max_level(level);
}

fn current_level() -> LevelFilter {
    match LOG_LEVEL.load(Ordering::Relaxed) {
        0 => LevelFilter::Off,
        1 => LevelFilter::Error,
        2 => LevelFilter::Warn,
        3 => LevelFilter::Info,
        4 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    }
}
