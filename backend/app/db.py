"""SQLite storage for the vertical-slice demo.

Two rules from the requirement are enforced here rather than in the API layer,
because they are the ones that must not be bypassable:

  1. Deducting stock and writing the usage record happen in ONE transaction
     (requirement US-2). A partial failure that deducted stock without a record,
     or recorded a draw without deducting, corrupts the thing the product exists
     to produce.
  2. A blocked draw is still a complete record (requirement section 2.1). Rows
     never get deleted; state moves forward only.
"""

from __future__ import annotations

import json
import sqlite3
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path

DB_PATH = Path(__file__).resolve().parents[1] / "urdwms.db"

SCHEMA = """
CREATE TABLE IF NOT EXISTS inventory_item (
    -- 型號. 驗收單上填的就是這個 (例 T6050BSW), 也是米數對照表的 key.
    item_code        TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    spec             TEXT,
    unit             TEXT NOT NULL DEFAULT '箱',
    shelf_life_days  INTEGER,
    safety_stock     INTEGER NOT NULL DEFAULT 0,
    meters_per_box   INTEGER,         -- 每箱米數. NULL = 此品項不用米數換算
    -- 箱上標籤印的完整料號 (例 2003.T7320BC-340X900-P1). 人不填這個, 影像辨識讀到的
    -- 是它, 所以留著做型號對映 (requirement 4 正規化對映表).
    supplier_code    TEXT,
    supplier         TEXT             -- 預設廠商, 收貨時帶出
);

CREATE TABLE IF NOT EXISTS inventory_lot (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    item_code         TEXT NOT NULL REFERENCES inventory_item(item_code),
    receipt_date      TEXT NOT NULL,      -- ISO. FIFO sort key. From the acceptance stamp, read at receiving.
    manufacture_date  TEXT,
    supplier_lot_code TEXT,
    -- 每批記實際廠商: 同一型號換供應商是真實情況, 驗收單也是每次填.
    supplier          TEXT,
    entered_meters    INTEGER,        -- 驗收單上填的米數原值, 保留供稽核比對
    qty_on_hand       INTEGER NOT NULL,
    created_at        TEXT NOT NULL,
    created_by        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS material_usage_scan (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    item_code            TEXT,
    lot_id               INTEGER REFERENCES inventory_lot(id),
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

CREATE TABLE IF NOT EXISTS audit_log (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    at      TEXT NOT NULL,
    actor   TEXT NOT NULL,
    action  TEXT NOT NULL,
    detail  TEXT NOT NULL DEFAULT '{}'
);
"""


def now() -> str:
    return datetime.now(timezone.utc).astimezone().isoformat(timespec="seconds")


def connect() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH, isolation_level=None)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys = ON")
    return conn


@contextmanager
def transaction():
    """One unit of work. Stock movement and its record commit together or not at all."""
    conn = connect()
    try:
        conn.execute("BEGIN IMMEDIATE")
        yield conn
        conn.execute("COMMIT")
    except Exception:
        conn.execute("ROLLBACK")
        raise
    finally:
        conn.close()


# Columns added after the first release. SQLite has no IF NOT EXISTS for ADD
# COLUMN, so each is applied only when absent — keeps an existing demo database
# working instead of requiring a wipe.
_ADDED_COLUMNS: tuple[tuple[str, str, str], ...] = (
    ("inventory_item", "meters_per_box", "meters_per_box INTEGER"),
    ("inventory_item", "supplier_code", "supplier_code TEXT"),
    ("inventory_item", "supplier", "supplier TEXT"),
    ("inventory_lot", "supplier", "supplier TEXT"),
    ("inventory_lot", "entered_meters", "entered_meters INTEGER"),
)


def init_db() -> None:
    DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    conn = connect()
    try:
        conn.executescript(SCHEMA)
        for table, column, ddl in _ADDED_COLUMNS:
            present = {r["name"] for r in conn.execute(f"PRAGMA table_info({table})")}
            if column not in present:
                conn.execute(f"ALTER TABLE {table} ADD COLUMN {ddl}")
    finally:
        conn.close()


def log(conn: sqlite3.Connection, actor: str, action: str, detail: dict) -> None:
    conn.execute(
        "INSERT INTO audit_log (at, actor, action, detail) VALUES (?, ?, ?, ?)",
        (now(), actor, action, json.dumps(detail, ensure_ascii=False)),
    )
