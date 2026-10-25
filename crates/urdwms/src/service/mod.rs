//! Everything the screens can ask for, and every rule that guards it.
//!
//! This is the old HTTP API with the HTTP taken out: each function checks the
//! same permission the endpoint did and returns the same answer, so the
//! behaviour the PoC measured is the behaviour on screen.

mod alerts;
mod dictionary;
mod items;
mod lots;
mod recognize;
mod scans;
mod settings;
mod users;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, Row};
use serde_json::{Map, Value};

pub use alerts::*;
pub use dictionary::*;
pub use items::*;
pub use lots::*;
pub use recognize::*;
pub use scans::*;
pub use settings::*;
pub use users::*;

use crate::db;
use crate::error::Res;

/// One open database plus where its photos live.
pub struct Store {
    pub conn: Connection,
    pub data_dir: PathBuf,
}

impl Store {
    pub fn open(data_dir: &Path) -> Res<Store> {
        let conn = db::open(&data_dir.join("urdwms.db"))?;
        db::init(&conn)?;
        std::fs::create_dir_all(data_dir.join("uploads"))?;
        Ok(Store { conn, data_dir: data_dir.to_path_buf() })
    }

    /// A store with no file behind it, for tests.
    #[cfg(test)]
    pub fn in_memory(data_dir: &Path) -> Res<Store> {
        let conn = Connection::open_in_memory()?;
        db::init(&conn)?;
        std::fs::create_dir_all(data_dir.join("uploads"))?;
        Ok(Store { conn, data_dir: data_dir.to_path_buf() })
    }

    pub fn uploads(&self) -> PathBuf {
        self.data_dir.join("uploads")
    }

    /// Image paths are stored as `/uploads/<name>` (the format the web version
    /// wrote, so old records still resolve).
    pub fn resolve_image(&self, stored: &str) -> PathBuf {
        let name = stored.trim_start_matches('/').trim_start_matches("uploads/");
        self.uploads().join(name)
    }

    /// Save a photo into uploads and return its stored path. A copy is kept
    /// even when the original sits in a camera folder: that folder is the
    /// vendor software's to rotate, and a traceability record whose photo
    /// lives somewhere we do not control is not a record.
    pub fn save_image(&self, data: &[u8], suffix: &str) -> Res<String> {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%6f");
        let ext = match urdwms_core::camera::media_type(data) {
            "image/png" => "png",
            "image/bmp" => "bmp",
            "image/webp" => "webp",
            _ => "jpg",
        };
        let name = format!("{stamp}{suffix}.{ext}");
        std::fs::write(self.uploads().join(&name), data)?;
        Ok(format!("/uploads/{name}"))
    }
}

pub(crate) fn json_map(text: Option<String>) -> Map<String, Value> {
    text.and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

pub(crate) fn opt_trim(value: &Option<String>) -> Option<String> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

pub(crate) fn get_bool(row: &Row, column: &str) -> rusqlite::Result<bool> {
    Ok(row.get::<_, Option<i64>>(column)?.unwrap_or(0) != 0)
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::auth::Session;

    pub fn store() -> Store {
        let dir = std::env::temp_dir().join(format!("urdwms-test-{}-{}", std::process::id(), rand_suffix()));
        let store = Store::in_memory(&dir).unwrap();
        crate::seed::demo(&store.conn).unwrap();
        store
    }

    fn rand_suffix() -> u64 {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        u64::from_le_bytes(b)
    }

    pub fn session(store: &Store, username: &str) -> Session {
        users::session_for(&store.conn, username).unwrap()
    }
}
