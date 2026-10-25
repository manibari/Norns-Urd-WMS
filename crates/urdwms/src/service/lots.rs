//! Lots: receiving (US-1), corrections, deletion, and the FIFO view of stock.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Map, Value, json};
use urdwms_core::matching::{Candidate, fifo_basis, fifo_expected, fifo_target};
use urdwms_core::normalize::to_date_key;
use urdwms_core::units::boxes_from_meters;

use super::{Store, get_bool, json_map, opt_trim};
use crate::auth::Session;
use crate::bail;
use crate::db::{self, log, now, today, today_iso};
use crate::error::Res;

pub const PASS: &str = "合格";
pub const FAIL: &str = "不合格";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lot {
    pub id: i64,
    pub item_id: i64,
    pub receipt_date: String,
    pub manufacture_date: Option<String>,
    pub supplier_lot_code: Option<String>,
    pub supplier: Option<String>,
    pub entered_meters: Option<i64>,
    pub entered_unit: Option<String>,
    pub expiry_date: Option<String>,
    pub inspection: Map<String, Value>,
    pub verdict: Option<String>,
    pub recorded_by: Option<String>,
    pub confirmed_by: Option<String>,
    pub remark: Option<String>,
    pub qty_received: i64,
    pub qty_on_hand: i64,
    pub created_at: String,
    pub created_by: String,
    pub item_name: String,
    pub item_model: Option<String>,
    pub item_spec: Option<String>,
    pub item_has_expiry: bool,
    /// One lot gets the badge. Judgement still accepts every same-day lot —
    /// but a screen marking two lots "應領" has told nobody which box to pick up.
    pub is_fifo_next: bool,
    pub fifo_also_ok: bool,
    /// Which field the guidance rests on. A fallback to receipt date is a
    /// weaker claim and should look like one.
    pub fifo_basis: &'static str,
    pub draw_count: i64,
    pub qty_drawn: i64,
    pub days_left: Option<i64>,
}

impl Lot {
    pub fn item_label(&self) -> &str {
        self.item_model.as_deref().unwrap_or(&self.item_name)
    }

    pub fn rejected(&self) -> bool {
        self.verdict.as_deref() == Some(FAIL)
    }

    pub fn drawable(&self) -> bool {
        self.qty_on_hand > 0 && !self.rejected()
    }

    /// Received (fixed) and on hand (falling) are both kept, which is what
    /// tells "untouched" from "being drawn" from "finished" — 3 left could be
    /// 3 received and untouched, or 10 received and 7 drawn.
    pub fn state(&self) -> &'static str {
        if self.qty_on_hand <= 0 {
            "已領完"
        } else if self.qty_on_hand < self.qty_received {
            "領貨中"
        } else {
            "未動用"
        }
    }

    fn from_row(r: &Row) -> rusqlite::Result<Lot> {
        let on_hand: i64 = r.get("qty_on_hand")?;
        let received = r.get::<_, Option<i64>>("qty_received")?.unwrap_or(on_hand);
        let expiry: Option<String> = r.get("expiry_date")?;
        Ok(Lot {
            id: r.get("id")?,
            item_id: r.get("item_id")?,
            receipt_date: r.get("receipt_date")?,
            manufacture_date: r.get("manufacture_date")?,
            supplier_lot_code: r.get("supplier_lot_code")?,
            supplier: r.get("supplier")?,
            entered_meters: r.get("entered_meters")?,
            entered_unit: r.get("entered_unit")?,
            days_left: days_until(expiry.as_deref()),
            expiry_date: expiry,
            inspection: json_map(r.get("inspection")?),
            verdict: r.get("verdict")?,
            recorded_by: r.get("recorded_by")?,
            confirmed_by: r.get("confirmed_by")?,
            remark: r.get("remark")?,
            qty_received: received,
            qty_on_hand: on_hand,
            qty_drawn: (received - on_hand).max(0),
            created_at: r.get("created_at")?,
            created_by: r.get("created_by")?,
            item_name: r.get("item_name")?,
            item_model: r.get("item_model")?,
            item_spec: r.get("item_spec")?,
            item_has_expiry: get_bool(r, "item_has_expiry")?,
            ..Default::default()
        })
    }
}

/// The expiry is the date recorded at receiving, full stop. Inferring it from
/// a shelf life would produce a date nobody wrote down and nobody can check
/// against the box.
pub fn days_until(date: Option<&str>) -> Option<i64> {
    let d = chrono::NaiveDate::parse_from_str(date?, "%Y-%m-%d").ok()?;
    Some((d - today()).num_days())
}

/// The drawable lots of one item, as the matcher sees them. A rejected lot is
/// not a candidate: it stays on the books but FIFO must never point anyone at
/// it, and nobody may draw it.
pub fn candidates(conn: &Connection, item_id: i64) -> Res<Vec<Candidate>> {
    let mut stmt = conn.prepare(
        "SELECT id, receipt_date, manufacture_date FROM inventory_lot
         WHERE item_id = ? AND qty_on_hand > 0 AND COALESCE(verdict, '合格') <> '不合格'
         ORDER BY receipt_date",
    )?;
    let rows = stmt
        .query_map([item_id], |r| {
            Ok(Candidate {
                lot_id: r.get::<_, i64>(0)?.to_string(),
                receipt_date: r.get(1)?,
                manufacture_date: r.get::<_, Option<String>>(2)?.filter(|s| !s.is_empty()),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn lots(conn: &Connection, item_id: Option<i64>) -> Res<Vec<Lot>> {
    let mut sql = String::from(
        "SELECT l.*, i.name AS item_name, i.model AS item_model, i.spec AS item_spec,
                i.has_expiry AS item_has_expiry
         FROM inventory_lot l JOIN inventory_item i ON i.id = l.item_id",
    );
    if item_id.is_some() {
        sql.push_str(" WHERE l.item_id = ?1");
    }
    sql.push_str(" ORDER BY i.name, i.model, l.receipt_date");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows: Vec<Lot> = match item_id {
        Some(id) => stmt.query_map([id], Lot::from_row)?.collect::<Result<_, _>>()?,
        None => stmt.query_map([], Lot::from_row)?.collect::<Result<_, _>>()?,
    };

    let mut drawn: HashMap<i64, i64> = HashMap::new();
    let mut stmt =
        conn.prepare("SELECT lot_id, COUNT(*) FROM material_usage_scan WHERE lot_id IS NOT NULL GROUP BY lot_id")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (lot, n) = row?;
        drawn.insert(lot, n);
    }

    let mut pools: HashMap<i64, (Vec<String>, Option<String>, &'static str)> = HashMap::new();
    for lot in &mut rows {
        if let std::collections::hash_map::Entry::Vacant(e) = pools.entry(lot.item_id) {
            let pool = candidates(conn, lot.item_id)?;
            e.insert((fifo_expected(&pool), fifo_target(&pool), fifo_basis(&pool)));
        }
        let (expected, target, basis) = &pools[&lot.item_id];
        let id = lot.id.to_string();
        lot.is_fifo_next = target.as_deref() == Some(id.as_str());
        lot.fifo_also_ok = expected.contains(&id) && !lot.is_fifo_next;
        lot.fifo_basis = basis;
        lot.draw_count = drawn.get(&lot.id).copied().unwrap_or(0);
    }
    Ok(rows)
}

/// One line off the acceptance form (包材驗收單).
///
/// Field order on paper: 進貨日期 / 廠商名稱 / 原物料名稱 / 型號 / 數量. An
/// existing item is chosen by `item_id`; a new one is created here from
/// `item_name` (required) and `model` (optional), because receiving is where
/// an item first appears and the warehouse should not be sent to a separate
/// master-data screen first.
#[derive(Debug, Clone, Default)]
pub struct LotInput {
    pub item_id: Option<i64>,
    pub receipt_date: String,
    pub supplier: Option<String>,
    pub manufacture_date: Option<String>,
    pub supplier_lot_code: Option<String>,
    /// Boxes, when counted in boxes.
    pub qty: Option<i64>,
    /// The quantity as written on the form (米/張/包) — the normal path.
    /// Converted to whole boxes; boxes remain the ledger unit.
    pub qty_entered: Option<i64>,
    pub item_name: Option<String>,
    pub model: Option<String>,
    pub spec: Option<String>,
    pub meters_per_box: Option<i64>,
    pub pack_unit: Option<String>,
    pub has_expiry: Option<bool>,
    pub supplier_code: Option<String>,
    pub expiry_date: Option<String>,
    pub entered_unit: String,
    pub inspection: Map<String, Value>,
    pub verdict: Option<String>,
    pub confirmed_by: Option<String>,
    pub remark: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LotCreated {
    pub id: i64,
    pub item_id: i64,
    pub receipt_date: String,
    pub created_item: bool,
    pub qty: i64,
    pub verdict: Option<String>,
    /// Same name in both boxes is reported, not blocked — blocking would just
    /// get a second name borrowed, and then the record lies (risk R2).
    pub same_signer: bool,
    /// Metres that do not divide into whole boxes are a real discrepancy the
    /// warehouse should see, never rounded away.
    pub conversion_note: Option<String>,
    /// Surfaced rather than auto-merged: whether two identical lines are one
    /// lot or two is the warehouse's call (US-1).
    pub duplicate_lot_exists: bool,
}

fn clean_date(raw: &Option<String>, label: &str) -> Res<Option<String>> {
    let Some(raw) = opt_trim(raw) else { return Ok(None) };
    match to_date_key(Some(&raw)).and_then(|d| d.iso) {
        Some(iso) => Ok(Some(iso)),
        None => bail!("{label}格式無法辨識：{raw}"),
    }
}

/// US-1 receiving. A future receipt date is almost always a typo, so it is refused.
pub fn create_lot(store: &Store, user: &Session, input: &LotInput) -> Res<LotCreated> {
    let conn = &store.conn;
    user.require(conn, "lot.create")?;
    let Some(receipt) = to_date_key(Some(&input.receipt_date)).and_then(|d| d.iso) else {
        bail!("進貨日格式無法辨識");
    };
    if receipt > today_iso() {
        bail!("進貨日 {receipt} 在未來，請確認是否打錯");
    }
    if input.qty_entered.is_none() && input.qty.unwrap_or(0) < 1 {
        bail!("數量至少 1 箱");
    }
    let manufacture = clean_date(&input.manufacture_date, "製造日期")?;
    let expiry = clean_date(&input.expiry_date, "有效期限")?;
    if let Some(v) = &input.verdict
        && v != PASS
        && v != FAIL
    {
        bail!("判定只能是合格或不合格");
    }

    db::transaction(conn, |tx| {
        let known = match input.item_id {
            Some(id) => {
                tx.query_row("SELECT id FROM inventory_item WHERE id = ?", [id], |r| r.get::<_, i64>(0)).optional()?
            }
            None => None,
        };
        let mut created_item = false;
        let item_id = match known {
            Some(id) => id,
            None => {
                let Some(name) = opt_trim(&input.item_name) else { bail!("新品項請填原物料名稱") };
                let model = opt_trim(&input.model);
                if let Some(m) = &model
                    && tx
                        .query_row("SELECT 1 FROM inventory_item WHERE model = ?", [m], |_| Ok(()))
                        .optional()?
                        .is_some()
                {
                    bail!("型號 {m} 已存在，請直接選那個品項");
                }
                let code = opt_trim(&input.supplier_code);
                super::items::assert_supplier_code_free(tx, code.as_deref(), None)?;
                tx.execute(
                    "INSERT INTO inventory_item (name, model, spec, unit, safety_stock, meters_per_box, pack_unit,
                       has_expiry, use_recognition, supplier_code, supplier)
                     VALUES (?, ?, ?, '箱', 0, ?, ?, ?, ?, ?, ?)",
                    params![
                        name,
                        model,
                        opt_trim(&input.spec),
                        input.meters_per_box.filter(|&m| m > 0),
                        opt_trim(&input.pack_unit),
                        input.has_expiry.unwrap_or(false),
                        model.is_some() || code.is_some(),
                        code,
                        opt_trim(&input.supplier)
                    ],
                )?;
                let id = tx.last_insert_rowid();
                created_item = true;
                log(tx, &user.name, "item.create", json!({"id": id, "name": name, "model": model}))?;
                id
            }
        };

        // One delivery can arrive as several manufacture-date batches, so same
        // item + same receipt date is NORMAL. Only an identical manufacture
        // date looks like the same lot entered twice — warning on the normal
        // case would train people to dismiss the warning.
        let duplicate = tx
            .query_row(
                "SELECT 1 FROM inventory_lot WHERE item_id = ? AND receipt_date = ?
                   AND COALESCE(manufacture_date, '') = COALESCE(?, '')",
                params![item_id, receipt, manufacture],
                |_| Ok(()),
            )
            .optional()?
            .is_some();

        let (label, has_expiry, rate): (String, bool, Option<i64>) = tx.query_row(
            "SELECT COALESCE(model, name), has_expiry, meters_per_box FROM inventory_item WHERE id = ?",
            [item_id],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?)),
        )?;
        // An item flagged as having a shelf life must arrive with its printed
        // 有效日期 — otherwise the expiry alert quietly never fires for it.
        if has_expiry && expiry.is_none() {
            bail!("{label} 有效期，請填「有效期限」。");
        }

        let mut qty = input.qty.unwrap_or(0);
        let mut conversion = None;
        if let Some(entered) = input.qty_entered {
            let Some(c) = boxes_from_meters(entered, rate) else {
                bail!("此品項尚未設定每箱數量，無法用單上數量收貨");
            };
            if c.boxes < 1 {
                bail!(
                    "{} 不足一箱（每箱 {}）。庫存以箱為單位，不做部分入庫。",
                    urdwms_core::units::thousands(entered),
                    urdwms_core::units::thousands(c.per_box)
                );
            }
            qty = c.boxes;
            conversion = Some(c);
        }

        tx.execute(
            "INSERT INTO inventory_lot (item_id, receipt_date, manufacture_date, expiry_date, supplier_lot_code,
               supplier, entered_meters, entered_unit, inspection, verdict, recorded_by, confirmed_by, remark,
               qty_received, qty_on_hand, created_at, created_by)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                item_id,
                receipt,
                manufacture,
                expiry,
                opt_trim(&input.supplier_lot_code),
                opt_trim(&input.supplier),
                input.qty_entered,
                input.entered_unit,
                Value::Object(input.inspection.clone()).to_string(),
                input.verdict,
                // 記錄人 is who is signed in, not a name picked from a list.
                user.name,
                opt_trim(&input.confirmed_by),
                opt_trim(&input.remark),
                qty,
                qty,
                now(),
                user.name
            ],
        )?;
        let id = tx.last_insert_rowid();
        log(
            tx,
            &user.name,
            "lot.create",
            json!({"lot_id": id, "item_id": item_id, "receipt_date": receipt, "qty": qty,
                   "entered": input.qty_entered, "verdict": input.verdict,
                   "recorded_by": user.name, "confirmed_by": input.confirmed_by}),
        )?;
        Ok(LotCreated {
            id,
            item_id,
            receipt_date: receipt.clone(),
            created_item,
            qty,
            verdict: input.verdict.clone(),
            same_signer: opt_trim(&input.confirmed_by).as_deref() == Some(user.name.as_str()),
            conversion_note: conversion.and_then(|c| c.note()),
            duplicate_lot_exists: duplicate,
        })
    })
}

/// Admin correction of a receiving line. Every field here is something a
/// person can mistype off the paper form.
#[derive(Debug, Clone, Default)]
pub struct LotPatch {
    pub receipt_date: Option<String>,
    pub manufacture_date: Option<String>,
    pub expiry_date: Option<String>,
    pub supplier: Option<String>,
    pub qty_on_hand: Option<i64>,
    pub verdict: Option<String>,
    pub remark: Option<String>,
}

/// Correct a receiving line (US-1: "可修正，但寫入 audit_log，不可靜默改").
///
/// The audit entry carries before and after values, because "someone changed
/// the receipt date" is useless without knowing what it was. Returns how many
/// posted draws already hang off this lot.
pub fn update_lot(store: &Store, user: &Session, id: i64, patch: &LotPatch) -> Res<i64> {
    let conn = &store.conn;
    user.require(conn, "lot.edit")?;
    let mut changes: Vec<(&str, Value)> = Vec::new();
    for (field, label, raw) in [
        ("receipt_date", "進貨日", &patch.receipt_date),
        ("manufacture_date", "製造日期", &patch.manufacture_date),
        ("expiry_date", "有效期限", &patch.expiry_date),
    ] {
        if raw.is_some() {
            changes.push((field, clean_date(raw, label)?.map_or(Value::Null, Value::String)));
        }
    }
    if let Some(s) = &patch.supplier {
        changes.push(("supplier", opt_trim(&Some(s.clone())).map_or(Value::Null, Value::String)));
    }
    if let Some(r) = &patch.remark {
        changes.push(("remark", opt_trim(&Some(r.clone())).map_or(Value::Null, Value::String)));
    }
    if let Some(v) = &patch.verdict {
        if v != PASS && v != FAIL {
            bail!("判定只能是合格或不合格");
        }
        changes.push(("verdict", json!(v)));
    }
    if let Some(q) = patch.qty_on_hand {
        if q < 0 {
            bail!("在庫數量不可為負");
        }
        changes.push(("qty_on_hand", json!(q)));
    }
    if changes.is_empty() {
        bail!("沒有要更新的欄位");
    }
    if let Some((_, Value::String(r))) = changes.iter().find(|(k, _)| *k == "receipt_date") {
        if *r > today_iso() {
            bail!("進貨日不可在未來");
        }
    } else if changes.iter().any(|(k, v)| *k == "receipt_date" && v.is_null()) {
        bail!("進貨日不可空白");
    }

    db::transaction(conn, |tx| {
        let before: Option<Map<String, Value>> = tx
            .query_row(
                "SELECT receipt_date, manufacture_date, expiry_date, supplier, remark, verdict, qty_on_hand
                 FROM inventory_lot WHERE id = ?",
                [id],
                |r| {
                    let mut m = Map::new();
                    for (i, k) in ["receipt_date", "manufacture_date", "expiry_date", "supplier", "remark", "verdict"]
                        .iter()
                        .enumerate()
                    {
                        m.insert(k.to_string(), r.get::<_, Option<String>>(i)?.map_or(Value::Null, Value::String));
                    }
                    m.insert("qty_on_hand".into(), json!(r.get::<_, i64>(6)?));
                    Ok(m)
                },
            )
            .optional()?;
        let Some(before) = before else { bail!("批次不存在") };
        let drawn: i64 = tx.query_row(
            "SELECT COUNT(*) FROM material_usage_scan WHERE lot_id = ? AND status IN ('posted','overridden')",
            [id],
            |r| r.get(0),
        )?;
        let set: Vec<String> = changes.iter().map(|(k, _)| format!("{k} = ?")).collect();
        let mut values: Vec<rusqlite::types::Value> = changes
            .iter()
            .map(|(_, v)| match v {
                Value::Null => rusqlite::types::Value::Null,
                Value::Number(n) => rusqlite::types::Value::Integer(n.as_i64().unwrap_or(0)),
                Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                other => rusqlite::types::Value::Text(other.to_string()),
            })
            .collect();
        values.push(rusqlite::types::Value::Integer(id));
        tx.execute(
            &format!("UPDATE inventory_lot SET {} WHERE id = ?", set.join(", ")),
            rusqlite::params_from_iter(values),
        )?;
        let before_changed: Map<String, Value> =
            changes.iter().map(|(k, _)| (k.to_string(), before.get(*k).cloned().unwrap_or(Value::Null))).collect();
        let after: Map<String, Value> = changes.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        log(
            tx,
            &user.name,
            "lot.update",
            json!({"lot_id": id, "before": before_changed, "after": after, "posted_draws": drawn}),
        )?;
        Ok(drawn)
    })
}

/// Delete a receiving line — only while nothing has been drawn from it.
///
/// A lot that has been issued from is referenced by usage records; deleting
/// it would leave them pointing at nothing, and the traceability answer is
/// the entire reason the system exists. The caller is pointed at setting the
/// quantity to zero instead, which keeps the chain intact.
pub fn delete_lot(store: &Store, user: &Session, id: i64) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "lot.delete")?;
    db::transaction(conn, |tx| {
        let row: Option<Value> = tx
            .query_row(
                "SELECT item_id, receipt_date, manufacture_date, qty_on_hand, supplier, verdict FROM inventory_lot WHERE id = ?",
                [id],
                |r| {
                    Ok(json!({"item_id": r.get::<_, i64>(0)?, "receipt_date": r.get::<_, String>(1)?,
                              "manufacture_date": r.get::<_, Option<String>>(2)?, "qty_on_hand": r.get::<_, i64>(3)?,
                              "supplier": r.get::<_, Option<String>>(4)?, "verdict": r.get::<_, Option<String>>(5)?}))
                },
            )
            .optional()?;
        let Some(row) = row else { bail!("批次不存在") };
        let drawn: i64 =
            tx.query_row("SELECT COUNT(*) FROM material_usage_scan WHERE lot_id = ?", [id], |r| r.get(0))?;
        if drawn > 0 {
            bail!(
                "這批已有 {drawn} 筆領用紀錄，刪掉會讓那些紀錄指向不存在的批次，追溯就斷了。要讓它退出流通請把在庫數量改成 0。"
            );
        }
        tx.execute("DELETE FROM inventory_lot WHERE id = ?", [id])?;
        log(tx, &user.name, "lot.delete", json!({"lot_id": id, "deleted": row}))?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::super::{ItemOrder, items};
    use super::*;

    fn input(item_id: i64) -> LotInput {
        LotInput {
            item_id: Some(item_id),
            receipt_date: today_iso(),
            qty: Some(2),
            entered_unit: "箱數".into(),
            verdict: Some(PASS.into()),
            ..Default::default()
        }
    }

    fn item_id(s: &Store, model: &str) -> i64 {
        items(&s.conn, ItemOrder::Sort).unwrap().into_iter().find(|i| i.model.as_deref() == Some(model)).unwrap().id
    }

    #[test]
    fn fifo_badge_sits_on_the_oldest_manufacture_date() {
        let s = store();
        let film = item_id(&s, "T7320BC");
        let rows = lots(&s.conn, Some(film)).unwrap();
        let next: Vec<_> = rows.iter().filter(|l| l.is_fifo_next).collect();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].manufacture_date.as_deref(), Some("2025-09-26"));
    }

    #[test]
    fn receiving_converts_and_reports_the_remainder() {
        let s = store();
        let kuo = session(&s, "kuo");
        let film = item_id(&s, "T7320BC");
        let mut lot = input(film);
        lot.qty = None;
        lot.qty_entered = Some(9500);
        lot.manufacture_date = Some("2026-07-01".into());
        let made = create_lot(&s, &kuo, &lot).unwrap();
        assert_eq!(made.qty, 10);
        assert!(made.conversion_note.unwrap().contains("500"));
    }

    #[test]
    fn future_receipt_dates_and_missing_expiry_are_refused() {
        let s = store();
        let kuo = session(&s, "kuo");
        let film = item_id(&s, "T7320BC");
        let mut lot = input(film);
        lot.receipt_date = "2999-01-01".into();
        assert!(create_lot(&s, &kuo, &lot).unwrap_err().0.contains("未來"));

        let desiccant = items(&s.conn, ItemOrder::Sort).unwrap().into_iter().find(|i| i.name == "脫氧劑").unwrap().id;
        assert!(create_lot(&s, &kuo, &input(desiccant)).unwrap_err().0.contains("有效期限"));
    }

    #[test]
    fn a_new_item_is_created_from_the_receiving_line() {
        let s = store();
        let kuo = session(&s, "kuo");
        let lot = LotInput {
            item_name: Some("鋁袋".into()),
            receipt_date: today_iso(),
            qty: Some(3),
            entered_unit: "箱數".into(),
            confirmed_by: Some("郭".into()),
            ..Default::default()
        };
        let made = create_lot(&s, &kuo, &lot).unwrap();
        assert!(made.created_item);
        assert!(made.same_signer, "郭 signing their own line is reported");
    }

    #[test]
    fn corrections_are_audited_with_before_and_after() {
        let s = store();
        let peter = session(&s, "peter");
        let film = item_id(&s, "T7320BC");
        let lot = lots(&s.conn, Some(film)).unwrap()[0].clone();
        update_lot(&s, &peter, lot.id, &LotPatch { qty_on_hand: Some(0), ..Default::default() }).unwrap();
        let detail: String = s
            .conn
            .query_row("SELECT detail FROM audit_log WHERE action = 'lot.update' ORDER BY id DESC", [], |r| r.get(0))
            .unwrap();
        assert!(detail.contains("\"before\""), "{detail}");
        assert!(detail.contains(&format!("\"qty_on_hand\":{}", lot.qty_on_hand)), "{detail}");
    }
}
