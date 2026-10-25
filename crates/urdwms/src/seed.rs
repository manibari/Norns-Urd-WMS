//! First-run accounts, and the demo data set.
//!
//! The demo lot spread is deliberate: three lots of the film, received months
//! apart, so the FIFO check has something to actually catch. Without an older
//! lot on hand, every draw passes and the screen never shows what the product
//! is for.

use chrono::Duration;
use rusqlite::{Connection, params};
use serde_json::json;

use crate::auth::{ROLES, default_role_label, hash_password};
use crate::db::{self, log, now, today};
use crate::error::Res;

/// A fresh install has nobody to sign in as. Create one admin whose password
/// must be changed on first login — the printed default is only good once.
pub fn first_run(conn: &Connection) -> Res<Option<&'static str>> {
    seed_roles(conn)?;
    let users: i64 = conn.query_row("SELECT COUNT(*) FROM app_user", [], |r| r.get(0))?;
    if users > 0 {
        return Ok(None);
    }
    conn.execute(
        "INSERT INTO app_user (username, name, title, role, password_hash, must_change, created_at)
         VALUES ('admin', '系統管理者', '資訊', 'admin', ?, 1, ?)",
        params![hash_password(DEFAULT_ADMIN_PASSWORD), now()],
    )?;
    seed_dictionary(conn)?;
    log(conn, "seed", "seed.run", json!({"first_run": true}))?;
    Ok(Some(DEFAULT_ADMIN_PASSWORD))
}

pub const DEFAULT_ADMIN_PASSWORD: &str = "urdwms-admin";

fn seed_roles(conn: &Connection) -> Res<()> {
    for (order, code) in ROLES.iter().enumerate() {
        conn.execute(
            "INSERT OR IGNORE INTO app_role (code, label, sort_order) VALUES (?,?,?)",
            params![code, default_role_label(code), order as i64],
        )?;
    }
    Ok(())
}

/// Seeds the dropdowns that are not attributes of a 型號. Machines and
/// products come off the paper form's own notes, so they are this factory's
/// configuration, not the product's.
const DICTIONARY: &[(&str, &[&str])] = &[
    ("supplier", &["臺灣希悅爾", "弘東京", "益壽", "三洋包裝", "金泰"]),
    ("job_title", &["倉管", "廠長", "品管", "線長", "包裝線作業員", "資訊"]),
    ("pack_unit", &["米", "張", "包", "捲", "個", "公斤"]),
    ("machine", &["D-003", "D-004", "D-023", "D-027"]),
    ("packed_product", &["經典", "豬切", "牛切", "休閒豬", "金尊", "海香", "魚絲", "雞脆", "豬脆", "香酥"]),
    ("override_reason", &["試產指定批", "舊批破損不可用", "規格臨時變更", "急單", "其他"]),
];

fn seed_dictionary(conn: &Connection) -> Res<()> {
    for (category, values) in DICTIONARY {
        for (order, value) in values.iter().enumerate() {
            conn.execute(
                "INSERT OR IGNORE INTO dictionary (category, value, sort_order, created_at) VALUES (?,?,?,?)",
                params![category, value, order as i64, now()],
            )?;
        }
    }
    Ok(())
}

/// 名稱, 型號, 規格, 安全水位, 每箱數量, 單位, 有效期, 箱上完整料號, 廠商.
/// Names are the real ones off the form. 膜類不需填有效日期, 脫氧劑要 —
/// a property of the item, not an exception to a rule.
type ItemSeed = (
    &'static str,
    Option<&'static str>,
    &'static str,
    i64,
    i64,
    &'static str,
    bool,
    Option<&'static str>,
    &'static str,
);
const ITEMS: &[ItemSeed] = &[
    ("休閒豬上膜", Some("T7320BC"), "340mm x 900M", 3, 900, "米", false, Some("2003.T7320BC-340X900-P1"), "臺灣希悅爾"),
    (
        "大包裝肉乾上膜",
        Some("T6240BA"),
        "334mm x 600M",
        2,
        600,
        "米",
        false,
        Some("2003.T6240BA-334X600"),
        "臺灣希悅爾",
    ),
    ("真空切片休閒下膜", Some("T6050BSW"), "300mm x 600M", 2, 600, "米", false, None, "臺灣希悅爾"),
    // 無型號: 只能人工選, 不走影像辨識. 有保存期限: 收貨必須填有效期限.
    ("脫氧劑", None, "100cc", 5, 500, "包", true, None, "弘東京"),
];

/// 品項索引, 進貨日(幾天前), 製造日, 有效日, 原廠 ROLL#, 箱數, 單上數量, 單位, 判定, 備註
type LotSeed = (
    usize,
    i64,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    i64,
    i64,
    &'static str,
    &'static str,
    Option<&'static str>,
);
const LOTS: &[LotSeed] = &[
    (0, 164, "2025-09-26", None, Some("20250915-3081*61"), 2, 1800, "米", "合格", None),
    (0, 101, "2026-01-18", None, Some("20260105-2214*08"), 3, 2700, "米", "合格", None),
    (0, 9, "2026-06-02", None, Some("20260528-4417*22"), 4, 3600, "米", "合格", None),
    (1, 41, "2025-06-01", None, Some("20250520-1180*13"), 1, 600, "米", "合格", None),
    // 同一次到貨、兩個製造日 → 兩筆 (單上寫在同一格, 系統分批)
    (2, 2, "2025-08-19", None, Some("20250815-7781*30"), 8, 7200, "米", "合格", None),
    (2, 2, "2025-09-26", None, Some("20250920-7802*11"), 12, 10800, "米", "合格", None),
    // 檢驗不合格: 留紀錄, 不計在庫, FIFO 不指, 領不出來
    (1, 5, "2026-02-10", None, Some("20260205-3310*07"), 10, 6000, "米", "不合格", Some("外觀破損")),
    (3, 20, "2026-01-05", Some("2028-01-05"), None, 6, 3000, "包", "合格", None),
];

const USERS: &[(&str, &str, &str, &str, &str)] = &[
    ("kuo", "郭", "倉管", "manager", "urdwms2026"),
    ("huang", "黃揚文", "廠長", "manager", "urdwms2026"),
    ("operator", "線上作業員", "包裝線作業員", "user", "urdwms2026"),
    ("peter", "Peter", "資訊", "admin", "peter0821"),
];

/// Load the demo data set into an empty database. Refuses if lots exist, so
/// it can never be run over real records.
pub fn demo(conn: &Connection) -> Res<bool> {
    let existing: i64 = conn.query_row("SELECT COUNT(*) FROM inventory_lot", [], |r| r.get(0))?;
    if existing > 0 {
        return Ok(false);
    }
    db::transaction(conn, |tx| {
        seed_roles(tx)?;
        seed_dictionary(tx)?;
        let mut ids = Vec::new();
        for (name, model, spec, safety, per_box, unit, expiry, code, supplier) in ITEMS {
            tx.execute(
                "INSERT INTO inventory_item (name, model, spec, unit, safety_stock, meters_per_box, pack_unit,
                   has_expiry, use_recognition, supplier_code, supplier)
                 VALUES (?, ?, ?, '箱', ?, ?, ?, ?, ?, ?, ?)",
                params![
                    name,
                    model,
                    spec,
                    safety,
                    per_box,
                    unit,
                    expiry,
                    model.is_some() || code.is_some(),
                    code,
                    supplier
                ],
            )?;
            ids.push(tx.last_insert_rowid());
        }
        let pass =
            r#"{"規格尺寸": "相符", "標示製造日期": "已填", "標示有效日期": null, "外觀": "正常", "顏色": "正常"}"#;
        let fail = r#"{"規格尺寸": "相符", "標示製造日期": "已填", "標示有效日期": null, "外觀": "不正常", "外觀備註": "外箱破損", "顏色": "正常"}"#;
        for (idx, days_ago, made, expiry, roll, qty, entered, unit, verdict, remark) in LOTS {
            let receipt = (today() - Duration::days(*days_ago)).format("%Y-%m-%d").to_string();
            tx.execute(
                "INSERT INTO inventory_lot (item_id, receipt_date, manufacture_date, expiry_date, supplier_lot_code,
                   supplier, entered_meters, entered_unit, inspection, verdict, recorded_by, confirmed_by, remark,
                   qty_received, qty_on_hand, created_at, created_by)
                 VALUES (?,?,?,?,?,?,?,?,?,?,'郭','黃揚文',?,?,?,?,'seed')",
                params![
                    ids[*idx],
                    receipt,
                    made,
                    expiry,
                    roll,
                    ITEMS[*idx].8,
                    entered,
                    unit,
                    if *verdict == "合格" { pass } else { fail },
                    verdict,
                    remark,
                    qty,
                    qty,
                    now()
                ],
            )?;
        }
        for (username, name, title, role, password) in USERS {
            tx.execute(
                "INSERT OR IGNORE INTO app_user (username, name, title, role, password_hash, created_at) VALUES (?,?,?,?,?,?)",
                params![username, name, title, role, hash_password(password), now()],
            )?;
        }
        log(tx, "seed", "seed.run", json!({"items": ITEMS.len(), "lots": LOTS.len(), "users": USERS.len()}))?;
        Ok(true)
    })
}
