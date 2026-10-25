//! SQLite storage.
//!
//! The schema is the one the Python vertical slice used, column for column, so
//! an existing `urdwms.db` opens unchanged — the rewrite moves the runtime, not
//! the records.
//!
//! Two rules from the requirement are enforced at this layer rather than in
//! the screens, because they are the ones that must not be bypassable:
//!
//! 1. Deducting stock and writing the usage record happen in ONE transaction
//!    (requirement US-2). A partial failure that deducted stock without a
//!    record, or recorded a draw without deducting, corrupts the thing the
//!    product exists to produce.
//! 2. A blocked draw is still a complete record (requirement section 2.1).
//!    Rows never get deleted; state moves forward only.

use std::path::Path;

use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use serde_json::Value;

use crate::error::Res;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS inventory_item (
    -- Surrogate key: 型號不是每個品項都有 (驗收單上「脫氧劑」那列型號就是空的).
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT NOT NULL,   -- 原物料名稱. 必填
    model            TEXT,            -- 型號. 選填, 有填則不可重複
    spec             TEXT,
    unit             TEXT NOT NULL DEFAULT '箱',
    shelf_life_days  INTEGER,
    safety_stock     INTEGER NOT NULL DEFAULT 0,
    meters_per_box   INTEGER,         -- 每箱數量. NULL = 不換算
    pack_unit        TEXT,            -- 米 | 張 | 包 | 捲 ...
    has_expiry       INTEGER NOT NULL DEFAULT 0,
    use_recognition  INTEGER NOT NULL DEFAULT 1,
    sort_order       INTEGER,
    supplier_code    TEXT,            -- 箱上完整料號, 辨識對映用
    supplier         TEXT             -- 預設廠商
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_item_model
    ON inventory_item(model) WHERE model IS NOT NULL;

CREATE TABLE IF NOT EXISTS inventory_lot (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id           INTEGER NOT NULL REFERENCES inventory_item(id),
    receipt_date      TEXT NOT NULL,
    manufacture_date  TEXT,
    supplier_lot_code TEXT,
    supplier          TEXT,
    entered_meters    INTEGER,
    entered_unit      TEXT,
    expiry_date       TEXT,
    inspection        TEXT NOT NULL DEFAULT '{}',
    verdict           TEXT,           -- 合格 | 不合格
    recorded_by       TEXT,
    confirmed_by      TEXT,
    remark            TEXT,
    qty_received      INTEGER,
    qty_on_hand       INTEGER NOT NULL,
    created_at        TEXT NOT NULL,
    created_by        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS material_usage_scan (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id              INTEGER REFERENCES inventory_item(id),
    lot_id               INTEGER REFERENCES inventory_lot(id),
    qty                  INTEGER NOT NULL DEFAULT 1,
    status               TEXT NOT NULL,   -- posted | blocked_fifo | blocked_unreadable | overridden | voided
    captured_at          TEXT NOT NULL,
    captured_by          TEXT NOT NULL,
    image_path           TEXT,
    ocr_receipt_date     TEXT,
    ocr_confidence       REAL,
    ocr_notes            TEXT,
    match_distance       REAL,
    fifo_expected_lot_id INTEGER,
    fifo_expected_date   TEXT,
    field_values         TEXT NOT NULL DEFAULT '{}',
    detail_pending       INTEGER NOT NULL DEFAULT 0,
    override_by          TEXT,
    override_reason      TEXT,
    created_at           TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS app_setting (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    updated_by TEXT
);

CREATE TABLE IF NOT EXISTS app_role (
    code       TEXT PRIMARY KEY,   -- user | manager | admin
    label      TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS app_user (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE,
    name          TEXT NOT NULL,
    title         TEXT,
    role          TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    active        INTEGER NOT NULL DEFAULT 1,
    must_change   INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS app_session (
    token      TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES app_user(id),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS dictionary (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    category   TEXT NOT NULL,
    value      TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    active     INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    UNIQUE (category, value)
);

CREATE TABLE IF NOT EXISTS audit_log (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    at      TEXT NOT NULL,
    actor   TEXT NOT NULL,
    action  TEXT NOT NULL,
    detail  TEXT NOT NULL DEFAULT '{}'
);
"#;

/// Columns added after the first release. SQLite has no IF NOT EXISTS for ADD
/// COLUMN, so each is applied only when absent — an older database keeps
/// working instead of requiring a wipe.
const ADDED_COLUMNS: &[(&str, &str, &str)] = &[
    ("inventory_item", "meters_per_box", "meters_per_box INTEGER"),
    ("inventory_item", "supplier_code", "supplier_code TEXT"),
    ("inventory_item", "supplier", "supplier TEXT"),
    ("inventory_lot", "supplier", "supplier TEXT"),
    ("inventory_lot", "entered_meters", "entered_meters INTEGER"),
    ("inventory_lot", "entered_unit", "entered_unit TEXT"),
    ("inventory_lot", "expiry_date", "expiry_date TEXT"),
    ("inventory_lot", "inspection", "inspection TEXT NOT NULL DEFAULT '{}'"),
    ("inventory_lot", "verdict", "verdict TEXT"),
    ("inventory_lot", "recorded_by", "recorded_by TEXT"),
    ("inventory_lot", "confirmed_by", "confirmed_by TEXT"),
    ("inventory_lot", "remark", "remark TEXT"),
    ("app_user", "title", "title TEXT"),
    ("inventory_item", "pack_unit", "pack_unit TEXT"),
    ("inventory_item", "has_expiry", "has_expiry INTEGER NOT NULL DEFAULT 0"),
    ("inventory_item", "use_recognition", "use_recognition INTEGER NOT NULL DEFAULT 1"),
    ("material_usage_scan", "qty", "qty INTEGER NOT NULL DEFAULT 1"),
    ("inventory_item", "sort_order", "sort_order INTEGER"),
    ("inventory_lot", "qty_received", "qty_received INTEGER"),
];

/// Local time with offset, second precision — the format every stored
/// timestamp already uses, so string comparison keeps working on old rows.
pub fn now() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

pub fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

pub fn today_iso() -> String {
    today().format("%Y-%m-%d").to_string()
}

pub fn open(path: &Path) -> Res<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    // WAL lets the recognition worker read while the screen writes; the busy
    // timeout covers the rare moment both want to write.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

pub fn init(conn: &Connection) -> Res<()> {
    conn.execute_batch(SCHEMA)?;
    for (table, column, ddl) in ADDED_COLUMNS {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let present: Vec<String> = stmt.query_map([], |r| r.get::<_, String>(1))?.collect::<Result<_, _>>()?;
        if !present.iter().any(|c| c == column) {
            conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {ddl}"), [])?;
        }
    }
    Ok(())
}

/// One unit of work. Stock movement and its record commit together or not at all.
pub fn transaction<T>(conn: &Connection, work: impl FnOnce(&Transaction) -> Res<T>) -> Res<T> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let out = work(&tx)?;
    tx.commit()?;
    Ok(out)
}

pub fn log(conn: &Connection, actor: &str, action: &str, detail: Value) -> Res<()> {
    conn.execute(
        "INSERT INTO audit_log (at, actor, action, detail) VALUES (?, ?, ?, ?)",
        params![now(), actor, action, detail.to_string()],
    )?;
    Ok(())
}
