//! Plain-text operations log, `urdwms.log` next to the database.
//!
//! The audit trail records what people did to the records; this records what
//! the program did — recognition calls, camera captures, errors shown on
//! screen — so "辨識一直失敗" can be answered by reading a file instead of
//! guessing. Kept under a few megabytes by rolling to `urdwms.log.1`.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static PATH: OnceLock<PathBuf> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

const MAX_BYTES: u64 = 4 * 1024 * 1024;

pub fn init(dir: &std::path::Path) {
    let _ = PATH.set(dir.join("urdwms.log"));
}

pub fn path() -> Option<PathBuf> {
    PATH.get().cloned()
}

pub fn write(level: &str, message: &str) {
    let Some(path) = PATH.get() else { return };
    let _guard = LOCK.lock();
    if path.metadata().is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    let line = format!("{} {level:<5} {}\n", crate::db::now(), message.replace('\n', " ⏎ "));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn info(message: impl AsRef<str>) {
    write("INFO", message.as_ref());
}

pub fn warn(message: impl AsRef<str>) {
    write("WARN", message.as_ref());
}

pub fn error(message: impl AsRef<str>) {
    write("ERROR", message.as_ref());
}
