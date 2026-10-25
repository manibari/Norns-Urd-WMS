//! Dropdown values that are not attributes of something else.
//!
//! 廠商 / 原物料名稱 / 規格 of a 型號 are read off the item master instead —
//! a separate table would hold the same facts twice. Records store the chosen
//! value as text, not a reference, so neither deactivating nor deleting a
//! value blanks it out of old records.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use super::Store;
use crate::auth::Session;
use crate::bail;
use crate::db::{self, log, now};
use crate::error::Res;

pub const CATEGORIES: &[(&str, &str)] = &[
    ("supplier", "廠商"),
    ("job_title", "職位名稱"),
    ("pack_unit", "計量單位"),
    ("machine", "包裝機台"),
    ("packed_product", "產品名稱"),
    ("override_reason", "非 FIFO 覆核原因"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct DictEntry {
    pub id: i64,
    pub category: String,
    pub value: String,
    pub sort_order: i64,
    pub active: bool,
}

pub fn dictionary(conn: &Connection, category: &str, include_inactive: bool) -> Res<Vec<DictEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, category, value, sort_order, active FROM dictionary
         WHERE category = ?1 AND (?2 OR active = 1) ORDER BY sort_order, value",
    )?;
    let rows = stmt
        .query_map(params![category, include_inactive], |r| {
            Ok(DictEntry {
                id: r.get(0)?,
                category: r.get(1)?,
                value: r.get(2)?,
                sort_order: r.get(3)?,
                active: r.get::<_, i64>(4)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Active values only — what a dropdown offers.
pub fn values(conn: &Connection, category: &str) -> Vec<String> {
    dictionary(conn, category, false).map(|d| d.into_iter().map(|e| e.value).collect()).unwrap_or_default()
}

/// Add a value. Re-adding a retired value revives the original row so
/// historical records keep pointing at the same entry. Returns whether it was
/// a revival.
pub fn add_entry(store: &Store, user: &Session, category: &str, value: &str) -> Res<bool> {
    let conn = &store.conn;
    user.require(conn, "dictionary.manage")?;
    add_entry_unchecked(conn, &user.name, category, value)
}

/// Used by receiving's "新增廠商" too: a value typed at the counter goes back
/// into the list instead of living only on that one screen.
pub(crate) fn add_entry_unchecked(conn: &Connection, actor: &str, category: &str, value: &str) -> Res<bool> {
    if !CATEGORIES.iter().any(|(c, _)| *c == category) {
        bail!("未知的字典類別 {category}");
    }
    let value = value.trim();
    if value.is_empty() {
        bail!("值不可空白");
    }
    db::transaction(conn, |tx| {
        let existing: Option<(i64, bool)> = tx
            .query_row(
                "SELECT id, active FROM dictionary WHERE category = ? AND value = ?",
                params![category, value],
                |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
            )
            .optional()?;
        if let Some((id, active)) = existing {
            if active {
                bail!("「{value}」已存在");
            }
            tx.execute("UPDATE dictionary SET active = 1 WHERE id = ?", [id])?;
            log(tx, actor, "dictionary.revive", json!({"id": id, "value": value}))?;
            return Ok(true);
        }
        let order: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM dictionary WHERE category = ?",
            [category],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO dictionary (category, value, sort_order, created_at) VALUES (?,?,?,?)",
            params![category, value, order, now()],
        )?;
        log(tx, actor, "dictionary.create", json!({"category": category, "value": value}))?;
        Ok(false)
    })
}

pub fn rename_entry(store: &Store, user: &Session, id: i64, value: &str) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "dictionary.manage")?;
    let value = value.trim();
    if value.is_empty() {
        bail!("值不可空白");
    }
    db::transaction(conn, |tx| {
        let category: Option<String> =
            tx.query_row("SELECT category FROM dictionary WHERE id = ?", [id], |r| r.get(0)).optional()?;
        let Some(category) = category else { bail!("字典項目不存在") };
        if tx
            .query_row(
                "SELECT 1 FROM dictionary WHERE category = ? AND value = ? AND id <> ?",
                params![category, value, id],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            bail!("「{value}」已存在");
        }
        tx.execute("UPDATE dictionary SET value = ? WHERE id = ?", params![value, id])?;
        log(tx, &user.name, "dictionary.update", json!({"id": id, "value": value}))?;
        Ok(())
    })
}

pub fn set_entry_active(store: &Store, user: &Session, id: i64, active: bool) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "dictionary.manage")?;
    db::transaction(conn, |tx| {
        if tx.execute("UPDATE dictionary SET active = ? WHERE id = ?", params![active, id])? == 0 {
            bail!("字典項目不存在");
        }
        log(tx, &user.name, "dictionary.update", json!({"id": id, "active": active}))?;
        Ok(())
    })
}

pub fn delete_entry(store: &Store, user: &Session, id: i64) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "dictionary.manage")?;
    db::transaction(conn, |tx| {
        let row: Option<(String, String)> = tx
            .query_row("SELECT category, value FROM dictionary WHERE id = ?", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((category, value)) = row else { bail!("字典項目不存在") };
        tx.execute("DELETE FROM dictionary WHERE id = ?", [id])?;
        log(tx, &user.name, "dictionary.delete", json!({"category": category, "value": value}))?;
        Ok(())
    })
}

/// Receiving may add a supplier on the spot; that needs only `lot.create`.
pub fn add_supplier_from_receiving(store: &Store, user: &Session, value: &str) -> Res<()> {
    user.require(&store.conn, "lot.create")?;
    match add_entry_unchecked(&store.conn, &user.name, "supplier", value) {
        Ok(_) => Ok(()),
        Err(e) if e.0.contains("已存在") => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    #[test]
    fn retiring_then_re_adding_revives_the_same_row() {
        let s = store();
        let admin = session(&s, "peter");
        let entry = dictionary(&s.conn, "machine", true).unwrap().remove(0);
        set_entry_active(&s, &admin, entry.id, false).unwrap();
        assert!(!values(&s.conn, "machine").contains(&entry.value));
        assert!(add_entry(&s, &admin, "machine", &entry.value).unwrap());
        assert_eq!(dictionary(&s.conn, "machine", true).unwrap().iter().filter(|e| e.value == entry.value).count(), 1);
    }

    #[test]
    fn unknown_categories_and_blanks_are_refused() {
        let s = store();
        let admin = session(&s, "peter");
        assert!(add_entry(&s, &admin, "colour", "red").is_err());
        assert!(add_entry(&s, &admin, "machine", "  ").is_err());
    }
}
