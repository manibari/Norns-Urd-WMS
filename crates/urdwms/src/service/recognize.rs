//! Photo -> which item, which lot, and whether FIFO allows it.
//!
//! Nothing is written and no stock moves here. The confirmation step stays,
//! because "this is not the box I picked up" has no other line of defence
//! (M7 architecture, constraint 4).

use std::time::Instant;

use rusqlite::Connection;
use urdwms_core::matching::{fifo_basis, fifo_target, match_item_code, match_lot};
use urdwms_core::normalize::to_date_key;
use urdwms_core::recognition::{Recognition, RecognitionProvider};

use super::{Store, lots::candidates, scans::fifo_verdict};
use crate::error::Res;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LockedLot {
    pub lot_id: i64,
    pub receipt_date: String,
    pub manufacture_date: Option<String>,
    pub qty_on_hand: i64,
}

/// What the issuing screen is shown after a photo.
#[derive(Debug, Clone, Default)]
pub struct Proposal {
    pub image_path: Option<String>,
    pub recognition: Recognition,
    pub timing_ms: u128,
    /// From a camera folder: the file and the camera's own save time.
    pub source_name: Option<String>,
    pub source_time: Option<String>,
    pub item_id: Option<i64>,
    pub item_name: Option<String>,
    pub item_model: Option<String>,
    /// `supplier_code` | `model_in_label` | `manual`
    pub item_matched_on: Option<&'static str>,
    pub item_defer_reason: Option<&'static str>,
    pub decision: &'static str,
    pub defer_reason: Option<&'static str>,
    /// Which date on the box named the lot.
    pub lot_matched_on: Option<&'static str>,
    pub match_distance: Option<f64>,
    pub locked_lot: Option<LockedLot>,
    pub fifo_ok: Option<bool>,
    pub fifo_expected_date: Option<String>,
    pub fifo_target_lot_id: Option<i64>,
    pub fifo_basis: &'static str,
}

impl Proposal {
    pub fn item_label(&self) -> Option<&str> {
        self.item_model.as_deref().or(self.item_name.as_deref())
    }

    /// The photo names a manufacture date, but no lot on hand carries it: the
    /// box in hand is not on the books. Different from a FIFO miss — following
    /// the preselected lot would create a record pointing at the wrong lot.
    pub fn photo_disagrees(&self) -> bool {
        self.recognition.manufacture_date.is_some()
            && self.decision == "defer"
            && self.defer_reason == Some("no_candidate")
    }
}

/// Recognise a saved image and match it against stock.
///
/// Shared by every capture source: whatever produced the bytes, everything
/// downstream is identical — which is what makes a fixed camera an addition
/// rather than a second implementation of issuing.
pub fn recognise(
    conn: &Connection,
    provider: &dyn RecognitionProvider,
    image: &[u8],
    image_path: Option<String>,
) -> Res<Proposal> {
    let started = Instant::now();
    let reading = provider.recognize(image);
    let timing_ms = started.elapsed().as_millis();

    // Items with recognition switched off are excluded from matching entirely —
    // that is what switching it off means.
    let mut stmt = conn.prepare("SELECT id, model, supplier_code FROM inventory_item WHERE use_recognition = 1")?;
    let master = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?.to_string(), r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let item_match = match_item_code(reading.item_code.as_deref(), &master, reading.model_code.as_deref());

    let mut proposal =
        Proposal { image_path, timing_ms, decision: "defer", fifo_basis: "製造日期", ..Default::default() };
    if item_match.locked() {
        let id: i64 = item_match.item_id.as_deref().unwrap_or("0").parse().unwrap_or(0);
        proposal.item_matched_on = item_match.matched_on;
        lot_lookup(conn, id, &reading, &mut proposal)?;
    } else {
        proposal.item_defer_reason = item_match.reason.map(|r| r.as_str());
        proposal.defer_reason = Some("item_unresolved");
    }
    proposal.recognition = reading;
    Ok(proposal)
}

/// Re-run the lot match after a human picks the 型號 recognition could not.
/// Deliberately does not touch the image: recognition already ran and is
/// billed, and a second call could return a *different* reading.
pub fn resolve_item(conn: &Connection, proposal: &mut Proposal, item_id: i64) -> Res<()> {
    let reading = proposal.recognition.clone();
    proposal.item_matched_on = Some("manual");
    proposal.item_defer_reason = None;
    proposal.defer_reason = None;
    lot_lookup(conn, item_id, &reading, proposal)
}

/// Match the dates read off the box against this item's drawable lots.
/// 製造日 identifies the lot when it can (it is the FIFO key and it is
/// printed); 進貨日 settles it when two lots were made the same day.
fn lot_lookup(conn: &Connection, item_id: i64, reading: &Recognition, out: &mut Proposal) -> Res<()> {
    let pool = candidates(conn, item_id)?;
    let (name, model): (Option<String>, Option<String>) = conn
        .query_row("SELECT name, model FROM inventory_item WHERE id = ?", [item_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap_or((None, None));
    let result = match_lot(
        to_date_key(reading.receipt_date.as_deref()).as_ref(),
        &pool,
        to_date_key(reading.manufacture_date.as_deref()).as_ref(),
    );
    out.item_id = Some(item_id);
    out.item_name = name;
    out.item_model = model;
    out.decision = result.decision.as_str();
    out.defer_reason = result.reason.map(|r| r.as_str());
    out.lot_matched_on = result.matched_on;
    out.match_distance = result.best_distance;
    out.fifo_target_lot_id = fifo_target(&pool).and_then(|s| s.parse().ok());
    out.fifo_basis = fifo_basis(&pool);
    out.locked_lot = None;
    out.fifo_ok = None;
    out.fifo_expected_date = None;
    if let (true, Some(lot_id)) = (result.locked(), result.lot_id.as_deref()) {
        let (ok, expected) = fifo_verdict(&pool, lot_id);
        let id: i64 = lot_id.parse().unwrap_or(0);
        out.locked_lot = conn
            .query_row(
                "SELECT receipt_date, manufacture_date, qty_on_hand FROM inventory_lot WHERE id = ?",
                [id],
                |r| {
                    Ok(LockedLot {
                        lot_id: id,
                        receipt_date: r.get(0)?,
                        manufacture_date: r.get(1)?,
                        qty_on_hand: r.get(2)?,
                    })
                },
            )
            .ok();
        out.fifo_ok = Some(ok);
        out.fifo_expected_date = expected;
    }
    Ok(())
}

/// Convenience used by the issuing worker: save the photo, then recognise it.
pub fn recognise_and_store(
    store: &Store,
    provider: &dyn RecognitionProvider,
    image: &[u8],
    suffix: &str,
) -> Res<Proposal> {
    let path = store.save_image(image, suffix)?;
    recognise(&store.conn, provider, image, Some(path))
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    struct Fixed(Recognition);
    impl RecognitionProvider for Fixed {
        fn recognize(&self, _: &[u8]) -> Recognition {
            self.0.clone()
        }
    }

    fn reading(model: &str, made: &str) -> Fixed {
        Fixed(Recognition { model_code: Some(model.into()), manufacture_date: Some(made.into()), ..Default::default() })
    }

    #[test]
    fn the_oldest_box_is_ok() {
        let s = store();
        let p = recognise(&s.conn, &reading("T7320BC", "2025年09月26日"), b"", None).unwrap();
        assert_eq!(p.decision, "lock");
        assert_eq!(p.fifo_ok, Some(true));
        assert_eq!(p.lot_matched_on, Some("manufacture_date"));
    }

    #[test]
    fn a_newer_box_is_wrong_and_names_the_right_one() {
        let s = store();
        let p = recognise(&s.conn, &reading("T7320BC", "02.06.2026"), b"", None).unwrap();
        // 02.06.2026 is ambiguous day-first, but still keys onto 2026-06-02.
        assert_eq!(p.decision, "lock", "{p:?}");
        assert_eq!(p.fifo_ok, Some(false));
        assert_eq!(p.fifo_expected_date.as_deref(), Some("2025-09-26"));
    }

    #[test]
    fn a_box_not_on_the_books_is_flagged() {
        let s = store();
        let p = recognise(&s.conn, &reading("T7320BC", "2019-01-01"), b"", None).unwrap();
        assert!(p.photo_disagrees());
    }

    #[test]
    fn an_unknown_model_defers_to_a_human_who_can_resolve_it() {
        let s = store();
        let mut p = recognise(&s.conn, &reading("ZZZ999", "2025-09-26"), b"", None).unwrap();
        assert_eq!(p.item_id, None);
        assert_eq!(p.item_defer_reason, Some("no_item_match"));
        let film: i64 =
            s.conn.query_row("SELECT id FROM inventory_item WHERE model = 'T7320BC'", [], |r| r.get(0)).unwrap();
        resolve_item(&s.conn, &mut p, film).unwrap();
        assert_eq!(p.item_matched_on, Some("manual"));
        assert_eq!(p.fifo_ok, Some(true));
    }

    #[test]
    fn a_provider_failure_still_produces_a_usable_proposal() {
        let s = store();
        let p = recognise(&s.conn, &Fixed(Recognition::failed("offline")), b"", None).unwrap();
        assert_eq!(p.item_id, None);
        assert_eq!(p.recognition.error.as_deref(), Some("offline"));
    }
}
