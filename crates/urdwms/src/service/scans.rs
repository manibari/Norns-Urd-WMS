//! Issuing (US-2/3/4), override (US-5), records, traceability and the audit trail.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Map, Value, json};
use urdwms_core::matching::{Candidate, fifo_expected};

use super::{Store, json_map, lots::candidates};
use crate::auth::Session;
use crate::bail;
use crate::db::{self, log, now};
use crate::error::Res;

/// FIFO compares 製造日期, so lots made the same day are equally legal.
///
/// The date returned is what the operator is told to look for, so it must be
/// the same field the judgement used — telling someone to fetch a receipt date
/// while grading them on manufacture date is how a rule stops being followed.
pub fn fifo_verdict(candidates: &[Candidate], lot_id: &str) -> (bool, Option<String>) {
    let expected = fifo_expected(candidates);
    let Some(first) = expected.first() else { return (false, None) };
    let target = candidates.iter().find(|c| &c.lot_id == first).unwrap();
    let date = target.manufacture_date.clone().unwrap_or_else(|| target.receipt_date.clone());
    (expected.iter().any(|e| e == lot_id), Some(date))
}

#[derive(Debug, Clone, Default)]
pub struct ScanInput {
    pub item_id: i64,
    pub lot_id: Option<i64>,
    /// Usually one box, occasionally more.
    pub qty: i64,
    pub image_path: Option<String>,
    pub ocr_receipt_date: Option<String>,
    pub ocr_confidence: Option<f64>,
    pub ocr_notes: Option<String>,
    pub match_distance: Option<f64>,
    /// 包裝機台 / 產品名稱 / 使用步徑 …
    pub fields: Map<String, Value>,
    pub detail_pending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanOutcome {
    pub id: i64,
    /// `posted` | `blocked_fifo` | `blocked_unreadable`
    pub status: &'static str,
    pub fifo_expected_date: Option<String>,
}

fn insert_scan(
    conn: &Connection,
    input: &ScanInput,
    status: &str,
    lot_id: Option<i64>,
    expected: (Option<i64>, Option<String>),
    user: &Session,
) -> Res<i64> {
    conn.execute(
        "INSERT INTO material_usage_scan (item_id, lot_id, status, captured_at, captured_by, image_path,
           ocr_receipt_date, ocr_confidence, ocr_notes, match_distance, fifo_expected_lot_id, fifo_expected_date,
           field_values, detail_pending, qty, created_at)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            input.item_id,
            lot_id,
            status,
            now(),
            user.name,
            input.image_path,
            input.ocr_receipt_date,
            input.ocr_confidence,
            input.ocr_notes,
            input.match_distance,
            expected.0,
            expected.1,
            Value::Object(input.fields.clone()).to_string(),
            input.detail_pending,
            input.qty,
            now()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Record the draw. Always.
///
/// A FIFO violation writes a complete record and moves no stock. It is not an
/// error — it is the deliverable (requirement section 2.1): the caller did
/// nothing wrong, and the record exists either way.
pub fn create_scan(store: &Store, user: &Session, input: &ScanInput) -> Res<ScanOutcome> {
    let conn = &store.conn;
    user.require(conn, "issue.create")?;
    db::transaction(conn, |tx| {
        let pool = candidates(tx, input.item_id)?;
        let Some(lot_id) = input.lot_id else {
            let id = insert_scan(tx, input, "blocked_unreadable", None, (None, None), user)?;
            log(tx, &user.name, "scan.blocked_unreadable", json!({"scan_id": id}))?;
            return Ok(ScanOutcome { id, status: "blocked_unreadable", fifo_expected_date: None });
        };
        if input.qty < 1 {
            bail!("領用數量至少 1 箱");
        }
        let on_hand: Option<i64> =
            tx.query_row("SELECT qty_on_hand FROM inventory_lot WHERE id = ?", [lot_id], |r| r.get(0)).optional()?;
        let Some(on_hand) = on_hand else { bail!("批次不存在") };
        if on_hand < 1 {
            bail!("該批次已無庫存");
        }
        if input.qty > on_hand {
            // Refused rather than clamped: the operator and the books disagree,
            // and silently issuing fewer would hide that.
            bail!("這批只剩 {on_hand} 箱，領不了 {} 箱。若實際數量不符，請找管理者修正批次數量。", input.qty);
        }

        let (ok, expected_date) = fifo_verdict(&pool, &lot_id.to_string());
        let expected_id = fifo_expected(&pool).first().and_then(|s| s.parse::<i64>().ok());
        if !ok {
            let id = insert_scan(tx, input, "blocked_fifo", Some(lot_id), (expected_id, expected_date.clone()), user)?;
            log(
                tx,
                &user.name,
                "scan.blocked_fifo",
                json!({"scan_id": id, "took_lot": lot_id, "expected_lot": expected_id}),
            )?;
            return Ok(ScanOutcome { id, status: "blocked_fifo", fifo_expected_date: expected_date });
        }

        // Stock movement and record, one transaction (US-2).
        let id = insert_scan(tx, input, "posted", Some(lot_id), (expected_id, expected_date), user)?;
        tx.execute(
            "UPDATE inventory_lot SET qty_on_hand = qty_on_hand - ? WHERE id = ? AND qty_on_hand >= ?",
            params![input.qty, lot_id, input.qty],
        )?;
        log(tx, &user.name, "scan.posted", json!({"scan_id": id, "lot_id": lot_id, "qty": input.qty}))?;
        Ok(ScanOutcome { id, status: "posted", fifo_expected_date: None })
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scan {
    pub id: i64,
    pub item_id: Option<i64>,
    pub lot_id: Option<i64>,
    pub qty: i64,
    pub status: String,
    pub captured_at: String,
    pub captured_by: String,
    pub image_path: Option<String>,
    pub ocr_receipt_date: Option<String>,
    pub ocr_confidence: Option<f64>,
    pub ocr_notes: Option<String>,
    pub match_distance: Option<f64>,
    pub fifo_expected_date: Option<String>,
    pub field_values: Map<String, Value>,
    pub detail_pending: bool,
    pub override_by: Option<String>,
    pub override_reason: Option<String>,
    pub receipt_date: Option<String>,
    pub manufacture_date: Option<String>,
    pub item_label: Option<String>,
}

impl Scan {
    pub fn field(&self, key: &str) -> Option<String> {
        match self.field_values.get(key)? {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Null => None,
            Value::String(_) => None,
            other => Some(other.to_string()),
        }
    }

    fn from_row(r: &Row) -> rusqlite::Result<Scan> {
        let model: Option<String> = r.get("item_model")?;
        let name: Option<String> = r.get("item_name")?;
        Ok(Scan {
            id: r.get("id")?,
            item_id: r.get("item_id")?,
            lot_id: r.get("lot_id")?,
            qty: r.get::<_, Option<i64>>("qty")?.unwrap_or(1),
            status: r.get("status")?,
            captured_at: r.get("captured_at")?,
            captured_by: r.get("captured_by")?,
            image_path: r.get("image_path")?,
            ocr_receipt_date: r.get("ocr_receipt_date")?,
            ocr_confidence: r.get("ocr_confidence")?,
            ocr_notes: r.get("ocr_notes")?,
            match_distance: r.get("match_distance")?,
            fifo_expected_date: r.get("fifo_expected_date")?,
            field_values: json_map(r.get("field_values")?),
            detail_pending: r.get::<_, Option<i64>>("detail_pending")?.unwrap_or(0) != 0,
            override_by: r.get("override_by")?,
            override_reason: r.get("override_reason")?,
            receipt_date: r.get("receipt_date")?,
            manufacture_date: r.get("manufacture_date")?,
            item_label: model.or(name),
        })
    }
}

/// Usage records, newest first. Blocked draws are listed alongside posted
/// ones, not filed away as errors — the system can block the books but not
/// the physical box (risk R3), so they belong in traceability too.
pub fn scans(conn: &Connection, lot_id: Option<i64>, limit: i64) -> Res<Vec<Scan>> {
    let mut stmt = conn.prepare(
        "SELECT s.*, l.receipt_date, l.manufacture_date, i.name AS item_name, i.model AS item_model
         FROM material_usage_scan s
         LEFT JOIN inventory_lot l ON l.id = s.lot_id
         LEFT JOIN inventory_item i ON i.id = s.item_id
         WHERE (?1 IS NULL OR s.lot_id = ?1)
         ORDER BY s.id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![lot_id, limit], Scan::from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn status_label(status: &str) -> &'static str {
    match status {
        "posted" => "已登錄",
        "overridden" => "已覆核放行",
        "blocked_fifo" => "非 FIFO",
        "blocked_unreadable" => "待人工補批次",
        "voided" => "已作廢",
        _ => "未知",
    }
}

/// US-5. Without this the floor learns to stop photographing boxes entirely.
pub fn override_scan(store: &Store, user: &Session, scan_id: i64, reason: &str) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "scan.override")?;
    let reason = reason.trim();
    if reason.is_empty() {
        bail!("覆核必須填原因");
    }
    db::transaction(conn, |tx| {
        let row: Option<(String, Option<i64>, i64)> = tx
            .query_row("SELECT status, lot_id, qty FROM material_usage_scan WHERE id = ?", [scan_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<i64>>(2)?.unwrap_or(1)))
            })
            .optional()?;
        let Some((status, lot_id, qty)) = row else { bail!("紀錄不存在") };
        if status != "blocked_fifo" {
            bail!("只有非 FIFO 的紀錄可覆核，目前為「{}」", status_label(&status));
        }
        let on_hand: i64 =
            tx.query_row("SELECT qty_on_hand FROM inventory_lot WHERE id = ?", [lot_id], |r| r.get(0))?;
        if on_hand < qty {
            bail!("這批現在只剩 {on_hand} 箱，不夠放行 {qty} 箱。請先確認實際數量。");
        }
        tx.execute(
            "UPDATE material_usage_scan SET status = 'overridden', override_by = ?, override_reason = ? WHERE id = ?",
            params![user.name, reason, scan_id],
        )?;
        tx.execute(
            "UPDATE inventory_lot SET qty_on_hand = qty_on_hand - ? WHERE id = ? AND qty_on_hand >= ?",
            params![qty, lot_id, qty],
        )?;
        log(tx, &user.name, "scan.overridden", json!({"scan_id": scan_id, "reason": reason}))?;
        Ok(())
    })
}

/// Fill in 產品名稱 and friends after the fact (US-6: 可事後補).
pub fn update_scan_detail(store: &Store, user: &Session, scan_id: i64, fields: &Map<String, Value>) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "issue.detail")?;
    db::transaction(conn, |tx| {
        let current: Option<String> = tx
            .query_row("SELECT field_values FROM material_usage_scan WHERE id = ?", [scan_id], |r| r.get(0))
            .optional()?;
        let Some(current) = current else { bail!("紀錄不存在") };
        let mut merged = json_map(Some(current));
        for (k, v) in fields {
            merged.insert(k.clone(), v.clone());
        }
        let pending = merged.get("產品名稱").and_then(Value::as_str).is_none_or(str::is_empty);
        tx.execute(
            "UPDATE material_usage_scan SET field_values = ?, detail_pending = ? WHERE id = ?",
            params![Value::Object(merged).to_string(), pending, scan_id],
        )?;
        log(tx, &user.name, "scan.detail", json!({"scan_id": scan_id, "fields": fields}))?;
        Ok(())
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    pub id: i64,
    pub at: String,
    pub actor: String,
    pub action: String,
    pub detail: Value,
}

/// The change trail. US-1 requires corrections to be recorded rather than
/// silent; a trail nobody can read is silent in every way that matters.
pub fn audit(store: &Store, user: &Session, prefix: Option<&str>, limit: i64) -> Res<Vec<AuditEntry>> {
    let conn = &store.conn;
    user.require(conn, "audit.read")?;
    let mut stmt = conn.prepare(
        "SELECT id, at, actor, action, detail FROM audit_log
         WHERE (?1 IS NULL OR action LIKE ?1 || '%') ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![prefix, limit], |r| {
            Ok(AuditEntry {
                id: r.get(0)?,
                at: r.get(1)?,
                actor: r.get(2)?,
                action: r.get(3)?,
                detail: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(Value::Null),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn action_label(action: &str) -> &str {
    match action {
        "auth.login" => "登入",
        "auth.logout" => "登出",
        "auth.password_change" => "更改密碼",
        "lot.create" => "收貨建批",
        "lot.update" => "批次修正",
        "lot.delete" => "批次刪除",
        "item.create" => "新增型號",
        "item.update" => "型號修正",
        "item.delete" => "型號刪除",
        "item.reorder" => "型號排序",
        "dictionary.create" => "選項新增",
        "dictionary.update" => "選項修改",
        "dictionary.revive" => "選項啟用",
        "dictionary.delete" => "選項刪除",
        "role.rename" => "角色改名",
        "user.create" => "新增人員",
        "user.update" => "人員異動",
        "scan.posted" => "領用登錄",
        "scan.blocked_fifo" => "非 FIFO 擋下",
        "scan.blocked_unreadable" => "待人工補批次",
        "scan.overridden" => "覆核放行",
        "scan.detail" => "補登明細",
        "camera.configure" => "影像來源設定",
        "setting.recognition" => "辨識模型設定",
        "setting.alerts" => "提醒門檻設定",
        "setting.api_key" => "辨識金鑰設定",
        "seed.run" => "初始資料",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::super::{ItemOrder, items, lots};
    use super::*;

    fn film(s: &Store) -> (i64, Vec<super::super::Lot>) {
        let id = items(&s.conn, ItemOrder::Sort)
            .unwrap()
            .into_iter()
            .find(|i| i.model.as_deref() == Some("T7320BC"))
            .unwrap()
            .id;
        (id, lots(&s.conn, Some(id)).unwrap())
    }

    #[test]
    fn the_fifo_lot_posts_and_deducts_in_one_go() {
        let s = store();
        let op = session(&s, "operator");
        let (item, rows) = film(&s);
        let next = rows.iter().find(|l| l.is_fifo_next).unwrap();
        let out =
            create_scan(&s, &op, &ScanInput { item_id: item, lot_id: Some(next.id), qty: 1, ..Default::default() })
                .unwrap();
        assert_eq!(out.status, "posted");
        let after = lots(&s.conn, Some(item)).unwrap().into_iter().find(|l| l.id == next.id).unwrap();
        assert_eq!(after.qty_on_hand, next.qty_on_hand - 1);
    }

    #[test]
    fn a_later_lot_is_recorded_blocked_and_moves_no_stock_until_overridden() {
        let s = store();
        let op = session(&s, "operator");
        let (item, rows) = film(&s);
        let newest = rows.iter().max_by_key(|l| l.manufacture_date.clone()).unwrap();
        let out =
            create_scan(&s, &op, &ScanInput { item_id: item, lot_id: Some(newest.id), qty: 1, ..Default::default() })
                .unwrap();
        assert_eq!(out.status, "blocked_fifo");
        assert_eq!(out.fifo_expected_date.as_deref(), Some("2025-09-26"));
        let still = lots(&s.conn, Some(item)).unwrap().into_iter().find(|l| l.id == newest.id).unwrap();
        assert_eq!(still.qty_on_hand, newest.qty_on_hand);

        assert!(override_scan(&s, &op, out.id, "急單").is_err(), "operators cannot override");
        let kuo = session(&s, "kuo");
        assert!(override_scan(&s, &kuo, out.id, "  ").is_err(), "a reason is required");
        override_scan(&s, &kuo, out.id, "急單").unwrap();
        let after = lots(&s.conn, Some(item)).unwrap().into_iter().find(|l| l.id == newest.id).unwrap();
        assert_eq!(after.qty_on_hand, newest.qty_on_hand - 1);
        assert!(override_scan(&s, &kuo, out.id, "急單").is_err(), "only once");
    }

    #[test]
    fn drawing_more_than_exists_is_refused_not_clamped() {
        let s = store();
        let op = session(&s, "operator");
        let (item, rows) = film(&s);
        let next = rows.iter().find(|l| l.is_fifo_next).unwrap();
        let err =
            create_scan(&s, &op, &ScanInput { item_id: item, lot_id: Some(next.id), qty: 99, ..Default::default() })
                .unwrap_err();
        assert!(err.0.contains("只剩"));
    }

    #[test]
    fn detail_can_be_filled_in_later() {
        let s = store();
        let op = session(&s, "operator");
        let (item, rows) = film(&s);
        let next = rows.iter().find(|l| l.is_fifo_next).unwrap();
        let out = create_scan(
            &s,
            &op,
            &ScanInput { item_id: item, lot_id: Some(next.id), qty: 1, detail_pending: true, ..Default::default() },
        )
        .unwrap();
        let mut fields = Map::new();
        fields.insert("產品名稱".into(), json!("經典"));
        update_scan_detail(&s, &op, out.id, &fields).unwrap();
        let row = scans(&s.conn, None, 10).unwrap().into_iter().find(|r| r.id == out.id).unwrap();
        assert!(!row.detail_pending);
        assert_eq!(row.field("產品名稱").as_deref(), Some("經典"));
    }
}
