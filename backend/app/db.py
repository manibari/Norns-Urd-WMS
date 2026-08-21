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
    -- Surrogate key: 型號不是每個品項都有 (驗收單上「脫氧劑」那列型號就是空的),
    -- 所以不能拿它當主鍵. 必填的是原物料名稱.
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT NOT NULL,   -- 原物料名稱. 必填
    model            TEXT,            -- 型號 (例 T6050BSW). 選填, 有填則不可重複
    spec             TEXT,
    unit             TEXT NOT NULL DEFAULT '箱',
    shelf_life_days  INTEGER,
    safety_stock     INTEGER NOT NULL DEFAULT 0,
    -- 每箱數量 + 單位. 單位隨品項而定 (膜是米, 袋是張, 劑是包), 所以跟著品項存.
    meters_per_box   INTEGER,         -- NULL = 不換算, 只能用箱數收貨
    pack_unit        TEXT,            -- 米 | 張 | 包 | 捲 ...
    -- 有沒有保存期限. 有的話收貨必須留下到期依據 (標示有效日期, 或製造日+保存天數).
    -- 驗收單註記: 肉乾真空膜不需填有效日期, 肉鬆產品要填 —— 這是品項的性質.
    has_expiry       INTEGER NOT NULL DEFAULT 0,
    -- 要不要讓這個品項走影像辨識. 有型號/料號只代表「認得出來」, 這個欄位是
    -- 「要不要用」—— 標籤太小、常辨錯的品項可以關掉, 直接走人工.
    -- 沒型號也沒料號的品項存 0: 開著卻認不出來是自相矛盾的狀態.
    use_recognition  INTEGER NOT NULL DEFAULT 1,
    sort_order       INTEGER,         -- 主檔顯示順序, 使用者可拖曳調整
    -- 箱上標籤印的完整料號 (例 2003.T7320BC-340X900-P1). 人不填這個, 影像辨識讀到的
    -- 是它, 用來對映回品項 (requirement 4 正規化對映表).
    supplier_code    TEXT,
    supplier         TEXT             -- 預設廠商, 收貨時帶出
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_item_model
    ON inventory_item(model) WHERE model IS NOT NULL;

CREATE TABLE IF NOT EXISTS inventory_lot (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id           INTEGER NOT NULL REFERENCES inventory_item(id),
    receipt_date      TEXT NOT NULL,      -- ISO. FIFO sort key. From the acceptance stamp, read at receiving.
    manufacture_date  TEXT,
    supplier_lot_code TEXT,
    -- 每批記實際廠商: 同一型號換供應商是真實情況, 驗收單也是每次填.
    supplier          TEXT,
    entered_meters    INTEGER,        -- 驗收單上填的數量原值, 保留供稽核比對
    entered_unit      TEXT,           -- 米 | 張 | 箱
    expiry_date       TEXT,           -- 標示(有效日期). 多數包材沒有, 留空是正常的
    -- 檢驗項目 JSON: 規格尺寸/標示製造日期/標示有效日期/外觀/顏色, 值為 true|false|null
    inspection        TEXT NOT NULL DEFAULT '{}',
    verdict           TEXT,           -- 合格 | 不合格. 不合格不進可領用庫存
    -- 雙簽: 填單的人與確認的人是不同的人, 這是這張單的控制點所在
    recorded_by       TEXT,
    confirmed_by      TEXT,
    remark            TEXT,
    -- 收進來幾箱 (不變) 與現在剩幾箱 (遞減). 兩個都留才分得出「還沒動」「領貨中」
    -- 「已領完」—— 只看剩餘量的話, 剩 3 箱可能是收 3 箱沒動, 也可能是收 10 箱領了 7.
    qty_received      INTEGER,
    qty_on_hand       INTEGER NOT NULL,
    created_at        TEXT NOT NULL,
    created_by        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS material_usage_scan (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id              INTEGER REFERENCES inventory_item(id),
    lot_id               INTEGER REFERENCES inventory_lot(id),
    qty                  INTEGER NOT NULL DEFAULT 1,   -- 這次領幾箱
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

-- 系統設定 (key-value). 目前只有影像來源, 但這類「整廠一份」的設定不該各自開表.
CREATE TABLE IF NOT EXISTS app_setting (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    updated_by TEXT
);

-- 角色顯示名稱. 角色「能做什麼」是系統設計(auth.PERMISSIONS, 寫在程式裡),
-- 角色「叫什麼」是各廠用語, 所以只有 label 進 DB.
CREATE TABLE IF NOT EXISTS app_role (
    code       TEXT PRIMARY KEY,   -- user | manager | admin
    label      TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0
);

-- 使用者與角色.
CREATE TABLE IF NOT EXISTS app_user (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE,   -- 登入帳號
    name          TEXT NOT NULL,          -- 顯示名/簽核名, 會寫進紀錄人與稽核軌跡
    -- 職位: 給人看的職稱 (倉管/廠長/品管/包裝線作業員). 與權限無關 —— 兩個職位
    -- 可以是同一個 role, 這正是把它們分開的理由.
    title         TEXT,
    role          TEXT NOT NULL,          -- user | manager | admin (權限層級)
    password_hash TEXT NOT NULL,
    active        INTEGER NOT NULL DEFAULT 1,
    must_change   INTEGER NOT NULL DEFAULT 0,  -- 管理者重設密碼後強制更改
    created_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS app_session (
    token      TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES app_user(id),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

-- 下拉選項的字典. 收貨與領用的每個選單都從這裡來, 不是寫死在程式或前端.
-- 這是 US-11 廠別配置的第一塊: 換一家工廠, 換這張表的內容, 不動程式.
CREATE TABLE IF NOT EXISTS dictionary (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    category   TEXT NOT NULL,   -- supplier | material_name | spec | staff | machine | packed_product | override_reason
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
