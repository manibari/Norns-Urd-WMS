//! Plant-wide settings: where images come from, which model reads them, when
//! an alert fires. Distinct from 基本資料, which is the data the work is
//! *about*; these decide how the software behaves.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use urdwms_core::camera::CameraConfig;
use urdwms_core::recognition::{DEFAULT_MODEL, GeminiProvider, RecognitionProvider, Unconfigured};

use super::Store;
use crate::auth::Session;
use crate::bail;
use crate::db::{log, now};
use crate::error::Res;

const CAMERA_KEY: &str = "camera";
const RECOGNITION_KEY: &str = "recognition";
const ALERT_KEY: &str = "alerts";
const API_KEY: &str = "recognition_api_key";

/// Measured on the field photos (docs/poc/recognition-poc-spec.md). Offered as
/// a choice because the trade-off is a factory's to make.
pub const RECOGNITION_MODELS: &[(&str, &str, &str)] = &[
    ("gemini-3.7-flash", "Flash（預設）", "中位 8.4 秒，實測讀值與 Pro 相同"),
    ("gemini-pro-latest", "Pro", "中位 19.1 秒，同樣讀對"),
    ("gemini-3.5-flash", "Flash 3.5", "較舊，實測把進貨日讀錯過"),
];

fn read<T: DeserializeOwned>(conn: &Connection, key: &str) -> Res<Option<T>> {
    let raw: Option<String> =
        conn.query_row("SELECT value FROM app_setting WHERE key = ?", [key], |r| r.get(0)).optional()?;
    Ok(raw.and_then(|r| serde_json::from_str(&r).ok()))
}

fn write<T: Serialize>(conn: &Connection, key: &str, value: &T, actor: &str) -> Res<()> {
    conn.execute(
        "INSERT INTO app_setting (key, value, updated_at, updated_by) VALUES (?,?,?,?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
        params![key, serde_json::to_string(value)?, now(), actor],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- camera

pub fn camera_config(conn: &Connection) -> Res<CameraConfig> {
    Ok(read::<CameraConfig>(conn, CAMERA_KEY)?.unwrap_or_default())
}

/// Save the image source. A blank password means "leave it alone", so a
/// settings save does not wipe a credential the form never showed.
pub fn save_camera(store: &Store, user: &Session, mut config: CameraConfig) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "dictionary.manage")?;
    match config.transport.as_str() {
        "folder" => {
            let folder = config.folder.trim().to_string();
            if config.enabled && folder.is_empty() {
                bail!("啟用資料夾來源必須填資料夾路徑");
            }
            // Checked on save rather than at capture: a path typo found at
            // 7am in front of the line is a much worse place to find it.
            if !folder.is_empty() && !std::path::Path::new(&folder).is_dir() {
                bail!("找不到這個資料夾：{folder}");
            }
            config.folder = folder;
        }
        "http" | "raw" => {
            config.host = config.host.trim().to_string();
            if config.enabled && config.host.is_empty() {
                bail!("啟用網路相機必須填位址");
            }
            if config.port == 0 {
                bail!("連接埠必須在 1–65535");
            }
        }
        _ => bail!("連線方式只能是 http、raw 或 folder"),
    }
    if config.password.is_empty() {
        config.password = camera_config(conn)?.password;
    }
    write(conn, CAMERA_KEY, &config, &user.name)?;
    let mut shown = serde_json::to_value(&config)?;
    shown.as_object_mut().map(|m| m.remove("password"));
    log(conn, &user.name, "camera.configure", shown)?;
    Ok(())
}

// ---------------------------------------------------------------- recognition

pub fn recognition_model(conn: &Connection) -> String {
    read::<Value>(conn, RECOGNITION_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
        .or_else(|| std::env::var("URDWMS_MODEL").ok())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

pub fn save_recognition_model(store: &Store, user: &Session, model: &str) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "dictionary.manage")?;
    if !RECOGNITION_MODELS.iter().any(|(m, _, _)| *m == model) {
        bail!("未知的模型 {model}");
    }
    write(conn, RECOGNITION_KEY, &json!({"model": model}), &user.name)?;
    log(conn, &user.name, "setting.recognition", json!({"model": model}))?;
    Ok(())
}

/// The environment wins (a deployment may inject it); otherwise the key an
/// admin entered in 系統設定. Never shown back on screen.
pub fn api_key(conn: &Connection) -> Option<String> {
    ["GEMINI_API_KEY", "GOOGLE_API_KEY"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
        .map(|k| k.trim().to_string())
        .or_else(|| read::<String>(conn, API_KEY).ok().flatten().filter(|v| !v.is_empty()))
}

pub fn api_key_from_env() -> bool {
    ["GEMINI_API_KEY", "GOOGLE_API_KEY"].iter().any(|k| std::env::var(k).is_ok_and(|v| !v.trim().is_empty()))
}

pub fn save_api_key(store: &Store, user: &Session, key: &str) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "dictionary.manage")?;
    // Keys are plain ASCII. A paste from a chat or PDF can carry a zero-width
    // or non-breaking character that is invisible on screen and turns the key
    // into "API key not valid".
    let key: String = key.chars().filter(|c| c.is_ascii_graphic()).collect();
    write(conn, API_KEY, &key, &user.name)?;
    log(conn, &user.name, "setting.api_key", json!({"set": !key.is_empty()}))?;
    Ok(())
}

pub fn provider(conn: &Connection) -> Box<dyn RecognitionProvider> {
    match api_key(conn) {
        Some(key) => Box::new(GeminiProvider::new(key, recognition_model(conn))),
        None => Box::new(Unconfigured),
    }
}

// ---------------------------------------------------------------- alerts

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AlertThresholds {
    pub expiry_days: i64,
    pub stale_days: i64,
    pub pending_hours: i64,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        AlertThresholds { expiry_days: 60, stale_days: 120, pending_hours: 24 }
    }
}

pub fn alert_thresholds(conn: &Connection) -> Res<AlertThresholds> {
    Ok(read(conn, ALERT_KEY)?.unwrap_or_default())
}

pub fn save_alert_thresholds(store: &Store, user: &Session, t: AlertThresholds) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "dictionary.manage")?;
    if t.expiry_days < 1 || t.stale_days < 1 || t.pending_hours < 1 {
        bail!("門檻都必須大於 0");
    }
    write(conn, ALERT_KEY, &t, &user.name)?;
    log(conn, &user.name, "setting.alerts", serde_json::to_value(t)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    #[test]
    fn a_blank_password_keeps_the_stored_one() {
        let s = store();
        let admin = session(&s, "peter");
        let mut c = CameraConfig {
            enabled: true,
            host: "169.254.187.1".into(),
            password: "secret".into(),
            ..Default::default()
        };
        save_camera(&s, &admin, c.clone()).unwrap();
        c.password.clear();
        c.port = 8080;
        save_camera(&s, &admin, c).unwrap();
        let stored = camera_config(&s.conn).unwrap();
        assert_eq!((stored.password.as_str(), stored.port), ("secret", 8080));
        let logged: String = s
            .conn
            .query_row("SELECT detail FROM audit_log WHERE action='camera.configure'", [], |r| r.get(0))
            .unwrap_or_default();
        assert!(!logged.contains("secret"));
    }

    #[test]
    fn reads_the_python_versions_camera_json() {
        let s = store();
        s.conn
            .execute(
                "INSERT INTO app_setting (key, value, updated_at) VALUES ('camera', ?, '')",
                [r#"{"enabled": true, "transport": "folder", "host": "", "port": 80, "path": "/snapshot.jpg", "username": "", "password": "", "trigger": "", "folder": "/tmp", "timeout": 8.0}"#],
            )
            .unwrap();
        let c = camera_config(&s.conn).unwrap();
        assert_eq!(c.transport, "folder");
        assert!(c.enabled);
    }
}
