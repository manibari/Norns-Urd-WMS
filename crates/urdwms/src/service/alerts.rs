//! US-8 alerts plus the pending-detail reminder from US-6.
//!
//! An empty result still reports when it was checked: a blank panel cannot
//! distinguish "nothing is wrong" from "the check stopped running" (M7
//! architecture, constraint 2).

use rusqlite::Connection;

use super::lots::days_until;
use super::settings::{AlertThresholds, alert_thresholds};
use crate::db::{now, today};
use crate::error::Res;

#[derive(Debug, Clone, PartialEq)]
pub struct LotAlert {
    pub lot_id: i64,
    pub name: String,
    pub model: Option<String>,
    pub supplier: Option<String>,
    pub receipt_date: String,
    pub manufacture_date: Option<String>,
    pub expiry_date: Option<String>,
    pub qty_on_hand: i64,
    pub remark: Option<String>,
    pub recorded_by: Option<String>,
    pub confirmed_by: Option<String>,
    pub days_left: Option<i64>,
    pub age_days: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LowStock {
    pub item_id: i64,
    pub name: String,
    pub model: Option<String>,
    pub on_hand: i64,
    pub safety_stock: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingDetail {
    pub scan_id: i64,
    pub label: Option<String>,
    pub captured_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alerts {
    pub checked_at: String,
    pub thresholds: AlertThresholds,
    pub expiring: Vec<LotAlert>,
    pub stale: Vec<LotAlert>,
    pub low_stock: Vec<LowStock>,
    pub pending_detail: Vec<PendingDetail>,
    pub rejected: Vec<LotAlert>,
}

impl Alerts {
    pub fn total(&self) -> usize {
        self.expiring.len() + self.stale.len() + self.low_stock.len() + self.pending_detail.len() + self.rejected.len()
    }
}

fn lot_alerts(conn: &Connection, where_sql: &str) -> Res<Vec<LotAlert>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT l.id, i.name, i.model, l.supplier, l.receipt_date, l.manufacture_date, l.expiry_date,
                l.qty_on_hand, l.remark, l.recorded_by, l.confirmed_by
         FROM inventory_lot l JOIN inventory_item i ON i.id = l.item_id WHERE {where_sql}
         ORDER BY l.receipt_date"
    ))?;
    let today = today();
    let rows = stmt
        .query_map([], |r| {
            let receipt: String = r.get(4)?;
            let expiry: Option<String> = r.get(6)?;
            let age = chrono::NaiveDate::parse_from_str(&receipt, "%Y-%m-%d").map_or(0, |d| (today - d).num_days());
            Ok(LotAlert {
                lot_id: r.get(0)?,
                name: r.get(1)?,
                model: r.get(2)?,
                supplier: r.get(3)?,
                receipt_date: receipt,
                manufacture_date: r.get(5)?,
                days_left: days_until(expiry.as_deref()),
                expiry_date: expiry,
                qty_on_hand: r.get(7)?,
                remark: r.get(8)?,
                recorded_by: r.get(9)?,
                confirmed_by: r.get(10)?,
                age_days: age,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn alerts(conn: &Connection) -> Res<Alerts> {
    let t = alert_thresholds(conn)?;
    let drawable = lot_alerts(conn, "l.qty_on_hand > 0 AND COALESCE(l.verdict, '合格') <> '不合格'")?;
    // Only the date someone actually recorded. An alert fired off an inferred
    // date would be an alert about a number the system made up.
    let expiring = drawable.iter().filter(|l| l.days_left.is_some_and(|d| d <= t.expiry_days)).cloned().collect();
    let stale = drawable.iter().filter(|l| l.age_days >= t.stale_days).cloned().collect();
    let rejected = lot_alerts(conn, "l.verdict = '不合格' AND l.qty_on_hand > 0")?;

    let mut stmt = conn.prepare(
        "SELECT i.id, i.name, i.model, i.safety_stock, COALESCE(SUM(l.qty_on_hand), 0) AS on_hand
         FROM inventory_item i LEFT JOIN inventory_lot l ON l.item_id = i.id
           AND COALESCE(l.verdict, '合格') <> '不合格'
         WHERE i.safety_stock > 0 GROUP BY i.id HAVING on_hand < i.safety_stock",
    )?;
    let low_stock = stmt
        .query_map([], |r| {
            Ok(LowStock {
                item_id: r.get(0)?,
                name: r.get(1)?,
                model: r.get(2)?,
                safety_stock: r.get(3)?,
                on_hand: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let cutoff =
        (chrono::Local::now() - chrono::Duration::hours(t.pending_hours)).format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut stmt = conn.prepare(
        "SELECT s.id, COALESCE(i.model, i.name), s.captured_at FROM material_usage_scan s
         LEFT JOIN inventory_item i ON i.id = s.item_id
         WHERE s.detail_pending = 1 AND s.status IN ('posted','overridden') AND s.captured_at < ?
         ORDER BY s.id",
    )?;
    let pending_detail = stmt
        .query_map([cutoff], |r| Ok(PendingDetail { scan_id: r.get(0)?, label: r.get(1)?, captured_at: r.get(2)? }))?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Alerts { checked_at: now(), thresholds: t, expiring, stale, low_stock, pending_detail, rejected })
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    #[test]
    fn demo_data_raises_the_expected_alerts() {
        let s = store();
        let a = alerts(&s.conn).unwrap();
        assert_eq!(a.rejected.len(), 1, "the damaged delivery");
        assert!(a.stale.iter().any(|l| l.model.as_deref() == Some("T7320BC")), "164-day-old film");
        assert!(a.low_stock.iter().any(|l| l.model.as_deref() == Some("T6240BA")), "1 box against a level of 2");
        assert!(a.checked_at.len() >= 19);
    }
}
