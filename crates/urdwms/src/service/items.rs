//! Item master (型號).

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::json;

use super::{Store, get_bool, opt_trim};
use crate::auth::Session;
use crate::bail;
use crate::db::{self, log};
use crate::error::Res;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    pub id: i64,
    /// 原物料名稱. 必填 — 型號 is optional (the form's 脫氧劑 line has none).
    pub name: String,
    pub model: Option<String>,
    pub spec: Option<String>,
    pub unit: String,
    pub shelf_life_days: Option<i64>,
    pub safety_stock: i64,
    /// 每箱數量 — the unit follows the item (膜是米, 袋是張, 劑是包).
    pub meters_per_box: Option<i64>,
    pub pack_unit: Option<String>,
    pub has_expiry: bool,
    pub use_recognition: bool,
    pub sort_order: Option<i64>,
    pub supplier_code: Option<String>,
    pub supplier: Option<String>,
    /// "在庫" means drawable: rejected lots are never counted here.
    pub on_hand: i64,
    pub open_lots: i64,
    pub rejected_qty: i64,
    pub last_receipt_date: Option<String>,
    pub last_lot_at: Option<String>,
}

impl Item {
    pub fn label(&self) -> &str {
        self.model.as_deref().unwrap_or(&self.name)
    }

    pub fn on_hand_m(&self) -> Option<i64> {
        urdwms_core::units::meters_from_boxes(self.on_hand, self.meters_per_box)
    }

    /// Something on the label to match against.
    pub fn matchable(&self) -> bool {
        self.model.is_some() || self.supplier_code.as_deref().is_some_and(|c| !c.is_empty())
    }

    /// Matchable AND someone decided to use it — two different things.
    pub fn recognisable(&self) -> bool {
        self.use_recognition && self.matchable()
    }

    pub fn is_low(&self) -> bool {
        self.safety_stock > 0 && self.on_hand < self.safety_stock
    }

    fn from_row(r: &Row) -> rusqlite::Result<Item> {
        Ok(Item {
            id: r.get("id")?,
            name: r.get("name")?,
            model: r.get("model")?,
            spec: r.get("spec")?,
            unit: r.get::<_, Option<String>>("unit")?.unwrap_or_else(|| "箱".into()),
            shelf_life_days: r.get("shelf_life_days")?,
            safety_stock: r.get::<_, Option<i64>>("safety_stock")?.unwrap_or(0),
            meters_per_box: r.get("meters_per_box")?,
            pack_unit: r.get("pack_unit")?,
            has_expiry: get_bool(r, "has_expiry")?,
            use_recognition: get_bool(r, "use_recognition")?,
            sort_order: r.get("sort_order")?,
            supplier_code: r.get("supplier_code")?,
            supplier: r.get("supplier")?,
            on_hand: r.get("on_hand")?,
            open_lots: r.get("open_lots")?,
            rejected_qty: r.get("rejected_qty")?,
            last_receipt_date: r.get("last_receipt_date")?,
            last_lot_at: r.get("last_lot_at")?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemOrder {
    /// Hand-arranged — the master screen.
    Sort,
    /// Latest delivery on top — the stock screen.
    Recent,
}

/// Item master with stock figures.
///
/// Two screens want two orders and neither is wrong. Sorting in each screen
/// instead would mean each quietly disagreeing about what "first" means.
pub fn items(conn: &Connection, order: ItemOrder) -> Res<Vec<Item>> {
    let order_sql = match order {
        // Items with no lots sort last: an empty row at the top is noise.
        ItemOrder::Recent => "ORDER BY last_lot_at IS NULL, last_lot_at DESC, last_receipt_date DESC, i.name",
        ItemOrder::Sort => "ORDER BY COALESCE(i.sort_order, i.id)",
    };
    let sql = format!(
        "SELECT i.*,
           COALESCE(SUM(CASE WHEN COALESCE(l.verdict,'合格') <> '不合格' THEN l.qty_on_hand ELSE 0 END), 0) AS on_hand,
           COUNT(CASE WHEN l.qty_on_hand > 0 AND COALESCE(l.verdict,'合格') <> '不合格' THEN 1 END) AS open_lots,
           COALESCE(SUM(CASE WHEN l.verdict = '不合格' THEN l.qty_on_hand ELSE 0 END), 0) AS rejected_qty,
           MAX(l.receipt_date) AS last_receipt_date,
           MAX(l.created_at) AS last_lot_at
         FROM inventory_item i LEFT JOIN inventory_lot l ON l.item_id = i.id
         GROUP BY i.id {order_sql}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Item::from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Fields for creating or editing an item. `None` means "not given".
#[derive(Debug, Clone, Default)]
pub struct ItemInput {
    pub name: Option<String>,
    pub model: Option<String>,
    pub spec: Option<String>,
    pub safety_stock: Option<i64>,
    pub meters_per_box: Option<i64>,
    pub pack_unit: Option<String>,
    pub has_expiry: Option<bool>,
    pub use_recognition: Option<bool>,
    pub supplier_code: Option<String>,
    pub supplier: Option<String>,
}

/// A box code must identify exactly one item. Two items sharing one printed
/// code makes every photo of that box ambiguous for ever — the matcher
/// correctly refuses to guess, so recognition never works for either item.
pub(crate) fn assert_supplier_code_free(conn: &Connection, code: Option<&str>, exclude: Option<i64>) -> Res<()> {
    let code = code.map(str::trim).unwrap_or("");
    if code.is_empty() {
        return Ok(());
    }
    let clash: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT name, model FROM inventory_item WHERE supplier_code = ? AND id <> ?",
            params![code, exclude.unwrap_or(-1)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((name, model)) = clash {
        let label = model.unwrap_or(name);
        bail!(
            "箱上料號 {code} 已經是「{label}」的。同一個料號指到兩個品項的話，拍照永遠分不出是哪一個 —— 請確認哪一個才對。"
        );
    }
    Ok(())
}

pub fn create_item(store: &Store, user: &Session, input: &ItemInput) -> Res<i64> {
    let conn = &store.conn;
    user.require(conn, "item.manage")?;
    let Some(name) = opt_trim(&input.name) else { bail!("原物料名稱不可空白") };
    let model = opt_trim(&input.model);
    db::transaction(conn, |tx| {
        if let Some(m) = &model
            && tx.query_row("SELECT 1 FROM inventory_item WHERE model = ?", [m], |_| Ok(())).optional()?.is_some()
        {
            bail!("型號 {m} 已存在");
        }
        let code = opt_trim(&input.supplier_code);
        assert_supplier_code_free(tx, code.as_deref(), None)?;
        let recognise = input.use_recognition.unwrap_or(true) && (model.is_some() || code.is_some());
        tx.execute(
            "INSERT INTO inventory_item (name, model, spec, unit, safety_stock, meters_per_box, pack_unit,
               has_expiry, use_recognition, supplier_code, supplier)
             VALUES (?, ?, ?, '箱', ?, ?, ?, ?, ?, ?, ?)",
            params![
                name,
                model,
                opt_trim(&input.spec),
                input.safety_stock.unwrap_or(0),
                input.meters_per_box.filter(|&m| m > 0),
                opt_trim(&input.pack_unit),
                input.has_expiry.unwrap_or(false),
                recognise,
                code,
                opt_trim(&input.supplier)
            ],
        )?;
        let id = tx.last_insert_rowid();
        log(tx, &user.name, "item.create", json!({"id": id, "name": name, "model": model}))?;
        Ok(id)
    })
}

/// Edit the master record. Only the fields given are changed.
pub fn update_item(store: &Store, user: &Session, id: i64, input: &ItemInput) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "item.manage")?;
    let mut changes: Vec<(&str, rusqlite::types::Value)> = Vec::new();
    use rusqlite::types::Value as V;
    let text = |v: &Option<String>| opt_trim(v).map_or(V::Null, V::Text);
    if let Some(name) = &input.name {
        if name.trim().is_empty() {
            bail!("原物料名稱不可空白");
        }
        changes.push(("name", V::Text(name.trim().into())));
    }
    if input.model.is_some() {
        changes.push(("model", text(&input.model)));
    }
    if input.spec.is_some() {
        changes.push(("spec", text(&input.spec)));
    }
    if let Some(s) = input.safety_stock {
        if s < 0 {
            bail!("安全水位不可為負");
        }
        changes.push(("safety_stock", V::Integer(s)));
    }
    if let Some(m) = input.meters_per_box {
        // 0 clears the rate ("不換算").
        if m < 0 {
            bail!("每箱數量必須大於 0（不換算請留空）");
        }
        changes.push(("meters_per_box", if m == 0 { V::Null } else { V::Integer(m) }));
    }
    if input.pack_unit.is_some() {
        changes.push(("pack_unit", text(&input.pack_unit)));
    }
    if let Some(b) = input.has_expiry {
        changes.push(("has_expiry", V::Integer(b as i64)));
    }
    if let Some(b) = input.use_recognition {
        changes.push(("use_recognition", V::Integer(b as i64)));
    }
    if input.supplier_code.is_some() {
        changes.push(("supplier_code", text(&input.supplier_code)));
    }
    if input.supplier.is_some() {
        changes.push(("supplier", text(&input.supplier)));
    }
    if changes.is_empty() {
        bail!("沒有要更新的欄位");
    }
    db::transaction(conn, |tx| {
        if tx.query_row("SELECT 1 FROM inventory_item WHERE id = ?", [id], |_| Ok(())).optional()?.is_none() {
            bail!("品項不存在");
        }
        if let Some(model) = opt_trim(&input.model)
            && tx
                .query_row("SELECT 1 FROM inventory_item WHERE model = ? AND id <> ?", params![model, id], |_| Ok(()))
                .optional()?
                .is_some()
        {
            bail!("型號 {model} 已被其他品項使用");
        }
        if input.supplier_code.is_some() {
            assert_supplier_code_free(tx, opt_trim(&input.supplier_code).as_deref(), Some(id))?;
        }
        let set: Vec<String> = changes.iter().map(|(k, _)| format!("{k} = ?")).collect();
        let mut values: Vec<rusqlite::types::Value> = changes.iter().map(|(_, v)| v.clone()).collect();
        values.push(V::Integer(id));
        tx.execute(
            &format!("UPDATE inventory_item SET {} WHERE id = ?", set.join(", ")),
            rusqlite::params_from_iter(values),
        )?;
        let detail: serde_json::Map<String, serde_json::Value> =
            changes.iter().map(|(k, v)| (k.to_string(), sql_to_json(v))).collect();
        log(tx, &user.name, "item.update", json!({"id": id, "changed": detail}))?;
        Ok(())
    })
}

pub(crate) fn sql_to_json(v: &rusqlite::types::Value) -> serde_json::Value {
    use rusqlite::types::Value as V;
    match v {
        V::Null => serde_json::Value::Null,
        V::Integer(i) => json!(i),
        V::Real(f) => json!(f),
        V::Text(t) => json!(t),
        V::Blob(_) => json!("<blob>"),
    }
}

/// Delete a 型號 — only while no lot or usage record references it. The
/// master record is what a usage record's item_id resolves to, so removing
/// one with history behind it turns every traceability answer about it into a
/// dangling reference.
pub fn delete_item(store: &Store, user: &Session, id: i64) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "item.manage")?;
    db::transaction(conn, |tx| {
        let row: Option<(String, Option<String>)> = tx
            .query_row("SELECT name, model FROM inventory_item WHERE id = ?", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((name, model)) = row else { bail!("品項不存在") };
        let label = model.clone().unwrap_or_else(|| name.clone());
        let lots: i64 = tx.query_row("SELECT COUNT(*) FROM inventory_lot WHERE item_id = ?", [id], |r| r.get(0))?;
        let scans: i64 =
            tx.query_row("SELECT COUNT(*) FROM material_usage_scan WHERE item_id = ?", [id], |r| r.get(0))?;
        if lots > 0 || scans > 0 {
            bail!(
                "{label} 已有 {lots} 批進貨、{scans} 筆領用紀錄，刪掉會讓那些紀錄失去對應。不再使用請把安全水位設 0 並停止收貨。"
            );
        }
        tx.execute("DELETE FROM inventory_item WHERE id = ?", [id])?;
        log(tx, &user.name, "item.delete", json!({"id": id, "name": name, "model": model}))?;
        Ok(())
    })
}

/// Persist a hand-arranged order. That grouping (films together, the thing
/// touched daily at the top) is not derivable from any column, so it is stored.
pub fn reorder_items(store: &Store, user: &Session, ids: &[i64]) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "item.manage")?;
    db::transaction(conn, |tx| {
        for (position, id) in ids.iter().enumerate() {
            tx.execute("UPDATE inventory_item SET sort_order = ? WHERE id = ?", params![position as i64, id])?;
        }
        log(tx, &user.name, "item.reorder", json!({"count": ids.len()}))?;
        Ok(())
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemOptions {
    pub supplier: Vec<String>,
    pub material_name: Vec<String>,
    pub spec: Vec<String>,
}

/// Distinct 廠商 / 原物料名稱 / 規格 already in use. Derived rather than
/// stored: these are attributes of a 型號, so a parallel list would be two
/// copies of the same fact, free to disagree.
pub fn item_options(conn: &Connection) -> Res<ItemOptions> {
    let distinct = |column: &str| -> Res<Vec<String>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT {column} FROM inventory_item WHERE {column} IS NOT NULL AND {column} <> '' ORDER BY {column}"
        ))?;
        Ok(stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?)
    };
    let mut stmt = conn.prepare(
        "SELECT value FROM dictionary WHERE category = 'supplier' AND active = 1 ORDER BY sort_order, value",
    )?;
    let mut supplier: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    // Curated list first, then any supplier already on an item — so an existing
    // value never becomes unselectable just because someone tidied the list.
    for s in distinct("supplier")? {
        if !supplier.contains(&s) {
            supplier.push(s);
        }
    }
    Ok(ItemOptions { supplier, material_name: distinct("name")?, spec: distinct("spec")? })
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    #[test]
    fn stock_excludes_rejected_lots() {
        let s = store();
        let all = items(&s.conn, ItemOrder::Sort).unwrap();
        let big = all.iter().find(|i| i.model.as_deref() == Some("T6240BA")).unwrap();
        assert_eq!(big.on_hand, 1);
        assert_eq!(big.rejected_qty, 10);
    }

    #[test]
    fn a_box_code_cannot_name_two_items() {
        let s = store();
        let admin = session(&s, "peter");
        let input = ItemInput {
            name: Some("x".into()),
            supplier_code: Some("2003.T7320BC-340X900-P1".into()),
            ..Default::default()
        };
        assert!(create_item(&s, &admin, &input).unwrap_err().0.contains("已經是"));
    }

    #[test]
    fn an_item_with_history_cannot_be_deleted() {
        let s = store();
        let admin = session(&s, "peter");
        let id = items(&s.conn, ItemOrder::Sort).unwrap()[0].id;
        assert!(delete_item(&s, &admin, id).unwrap_err().0.contains("批進貨"));
        let fresh = create_item(&s, &admin, &ItemInput { name: Some("新膜".into()), ..Default::default() }).unwrap();
        delete_item(&s, &admin, fresh).unwrap();
    }

    #[test]
    fn operators_cannot_edit_the_master() {
        let s = store();
        let op = session(&s, "operator");
        let err = create_item(&s, &op, &ItemInput { name: Some("x".into()), ..Default::default() }).unwrap_err();
        assert!(err.0.contains("需要"), "{err}");
    }
}
