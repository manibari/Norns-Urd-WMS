//! Pick a lot out of the closed candidate set, or refuse to.
//!
//! The matcher has two knobs and they guard different failures:
//!
//! * `max_distance` — how far a recognised date may sit from a candidate
//!   before we stop believing it at all. Too loose and a hallucinated date
//!   lands on a real lot.
//! * `min_margin` — how much better the best candidate must be than the
//!   runner-up. Without this, two lots received days apart turn a
//!   one-character OCR slip into a confident pick of the wrong lot — a false
//!   hit, which posts a FIFO violation in silence.
//!
//! Refusing (`Defer`) is cheap: the operator picks the lot by hand (US-4). A
//! false hit is not recoverable, so both knobs are biased toward refusing.

use crate::normalize::{DateKey, confusion_distance, to_date_key};

pub const MAX_DISTANCE: f64 = 1.5;
pub const MIN_MARGIN: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Lock,
    Defer,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Lock => "lock",
            Decision::Defer => "defer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemDeferReason {
    /// label code unreadable
    NoCodeRead,
    /// read a code, nothing in the master matches
    NoItemMatch,
    /// more than one 型號 could be it
    AmbiguousItem,
}

impl ItemDeferReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemDeferReason::NoCodeRead => "no_code_read",
            ItemDeferReason::NoItemMatch => "no_item_match",
            ItemDeferReason::AmbiguousItem => "ambiguous_item",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemMatch {
    pub decision: Decision,
    pub item_id: Option<String>,
    /// `"supplier_code"` | `"model_in_label"`
    pub matched_on: Option<&'static str>,
    pub reason: Option<ItemDeferReason>,
    pub contenders: Vec<String>,
}

impl ItemMatch {
    pub fn locked(&self) -> bool {
        self.decision == Decision::Lock
    }

    fn lock(item_id: &str, matched_on: &'static str) -> Self {
        ItemMatch {
            decision: Decision::Lock,
            item_id: Some(item_id.to_string()),
            matched_on: Some(matched_on),
            reason: None,
            contenders: vec![],
        }
    }

    fn defer(reason: ItemDeferReason, contenders: Vec<String>) -> Self {
        ItemMatch { decision: Decision::Defer, item_id: None, matched_on: None, reason: Some(reason), contenders }
    }
}

fn compact(value: &str) -> String {
    value.to_uppercase().replace(['-', '.', ' ', '_'], "")
}

/// One row of the item master as the matcher sees it: (id, 型號, 箱上完整料號).
pub type MasterRow = (String, Option<String>, Option<String>);

/// Work out which item a label belongs to.
///
/// Recognition reports the 型號 it read (`T6284BA`) and, when the label
/// carries one, the full supplier part number it sits inside
/// (`2003.T7320BC-340X900-P1`). Both are searched: an exact hit on the full
/// code first, then the 型號 as a substring.
///
/// 型號 is optional (the acceptance form's 脫氧劑 line has none), so an item
/// with neither cannot be matched from a label at all. That is a correct
/// outcome, not a failure — the operator picks it by hand.
///
/// Lock only on a unique answer. `T6050B` and `T6050BSW` both sit inside the
/// same label string, and picking the first one silently draws stock from the
/// wrong item — so an ambiguous read defers to a human instead of guessing.
pub fn match_item_code(label_code: Option<&str>, items: &[MasterRow], model_code: Option<&str>) -> ItemMatch {
    let read: Vec<&str> = [model_code, label_code].into_iter().flatten().filter(|s| !s.is_empty()).collect();
    if read.join(" ").trim().is_empty() {
        return ItemMatch::defer(ItemDeferReason::NoCodeRead, vec![]);
    }

    let label = compact(label_code.filter(|s| !s.is_empty()).or(model_code).unwrap_or(""));

    let exact: Vec<&String> = items
        .iter()
        .filter(|(_, _, code)| code.as_deref().is_some_and(|c| !c.is_empty() && compact(c) == label))
        .map(|(id, _, _)| id)
        .collect();
    match exact.len() {
        1 => return ItemMatch::lock(exact[0], "supplier_code"),
        n if n > 1 => {
            return ItemMatch::defer(ItemDeferReason::AmbiguousItem, exact.into_iter().cloned().collect());
        }
        _ => {}
    }

    // Fall back to finding the 型號 inside what was read: the 型號 on its own,
    // and the full string it may have been embedded in. Prefer the longest
    // match: `T6050BSW` beats `T6050B` on a label containing both, because the
    // longer one is the more specific reading, not a coincidence.
    let haystacks: Vec<String> =
        [model_code, label_code].into_iter().flatten().filter(|s| !s.is_empty()).map(compact).collect();
    let contained: Vec<(&String, String)> = items
        .iter()
        .filter_map(|(id, model, _)| {
            let m = compact(model.as_deref()?);
            (!m.is_empty() && haystacks.iter().any(|h| h.contains(&m))).then_some((id, m))
        })
        .collect();
    if contained.is_empty() {
        return ItemMatch::defer(ItemDeferReason::NoItemMatch, vec![]);
    }
    if contained.len() == 1 {
        return ItemMatch::lock(contained[0].0, "model_in_label");
    }
    let longest = contained.iter().map(|(_, m)| m.chars().count()).max().unwrap_or(0);
    let best: Vec<&String> =
        contained.iter().filter(|(_, m)| m.chars().count() == longest).map(|(id, _)| *id).collect();
    if best.len() == 1 {
        return ItemMatch::lock(best[0], "model_in_label");
    }
    let mut contenders: Vec<String> = contained.iter().map(|(id, _)| (*id).clone()).collect();
    contenders.sort();
    ItemMatch::defer(ItemDeferReason::AmbiguousItem, contenders)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferReason {
    /// model returned nothing usable
    NoRecognition,
    /// below the configured threshold
    LowConfidence,
    /// nothing within max_distance
    NoCandidateInRange,
    /// two candidates too close together
    Ambiguous,
    /// item has no stock on hand
    NoCandidates,
}

impl DeferReason {
    pub fn as_str(self) -> &'static str {
        match self {
            DeferReason::NoRecognition => "no_recognition",
            DeferReason::LowConfidence => "low_confidence",
            DeferReason::NoCandidateInRange => "no_candidate",
            DeferReason::Ambiguous => "ambiguous",
            DeferReason::NoCandidates => "empty_candidate_set",
        }
    }
}

/// One lot of this item currently on hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub lot_id: String,
    /// ISO, authoritative — it came from receiving, not from a photo.
    pub receipt_date: String,
    /// The FIFO key; printed by the supplier.
    pub manufacture_date: Option<String>,
}

impl Candidate {
    pub fn new(lot_id: impl Into<String>, receipt_date: impl Into<String>, manufacture_date: Option<&str>) -> Self {
        Candidate {
            lot_id: lot_id.into(),
            receipt_date: receipt_date.into(),
            manufacture_date: manufacture_date.map(str::to_string),
        }
    }

    pub fn key(&self) -> String {
        to_date_key(Some(&self.receipt_date)).map_or_else(|| self.receipt_date.clone(), |d| d.key)
    }

    pub fn manufacture_key(&self) -> Option<String> {
        let raw = self.manufacture_date.as_deref().filter(|s| !s.is_empty())?;
        Some(to_date_key(Some(raw)).map_or_else(|| raw.to_string(), |d| d.key))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    pub decision: Decision,
    pub lot_id: Option<String>,
    pub best_distance: Option<f64>,
    pub runner_up_distance: Option<f64>,
    pub reason: Option<DeferReason>,
    /// Which date on the box identified the lot. A lock off the printed 製造日
    /// is a stronger claim than one off the hand-stamped 進貨日.
    pub matched_on: Option<&'static str>,
}

impl MatchResult {
    pub fn locked(&self) -> bool {
        self.decision == Decision::Lock
    }

    fn defer(reason: DeferReason) -> Self {
        MatchResult {
            decision: Decision::Defer,
            lot_id: None,
            best_distance: None,
            runner_up_distance: None,
            reason: Some(reason),
            matched_on: None,
        }
    }
}

fn match_on(
    recognized: &DateKey,
    candidates: &[Candidate],
    key_of: impl Fn(&Candidate) -> Option<String>,
    field: &'static str,
    max_distance: f64,
    min_margin: f64,
) -> MatchResult {
    let mut scored: Vec<(f64, &Candidate)> =
        candidates.iter().filter_map(|c| key_of(c).map(|k| (confusion_distance(&recognized.key, &k), c))).collect();
    if scored.is_empty() {
        return MatchResult { matched_on: Some(field), ..MatchResult::defer(DeferReason::NoCandidates) };
    }
    // Stable: equal distances keep candidate order.
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (best_distance, best) = scored[0];
    let runner_up_distance = scored.get(1).map(|s| s.0);

    let deferred = |reason| MatchResult {
        decision: Decision::Defer,
        lot_id: None,
        best_distance: Some(best_distance),
        runner_up_distance,
        reason: Some(reason),
        matched_on: Some(field),
    };
    if best_distance > max_distance {
        return deferred(DeferReason::NoCandidateInRange);
    }
    if runner_up_distance.is_some_and(|r| r - best_distance < min_margin) {
        return deferred(DeferReason::Ambiguous);
    }
    MatchResult {
        decision: Decision::Lock,
        lot_id: Some(best.lot_id.clone()),
        best_distance: Some(best_distance),
        runner_up_distance,
        reason: None,
        matched_on: Some(field),
    }
}

/// Identify which lot the photographed box is, from the dates on it.
///
/// 製造日 is tried first: it is the field FIFO sorts on, and it is
/// machine-printed by the supplier, whereas the receipt date is a hand-applied
/// rubber stamp — the hardest thing on the box to read (§6 R1).
///
/// Falling through to the receipt date is not just a backstop: when two lots
/// share a manufacture date the 製造日 pass is genuinely ambiguous, and the
/// receipt date is exactly what tells them apart.
///
/// Both passes run against the closed candidate set of in-stock lots (§2.2),
/// so a misread lands on a real lot or on nothing — never on a date that was
/// never received.
pub fn match_candidates(
    recognized: Option<&DateKey>,
    candidates: &[Candidate],
    manufacture: Option<&DateKey>,
    max_distance: f64,
    min_margin: f64,
) -> MatchResult {
    if candidates.is_empty() {
        return MatchResult::defer(DeferReason::NoCandidates);
    }
    if recognized.is_none() && manufacture.is_none() {
        return MatchResult::defer(DeferReason::NoRecognition);
    }

    let mut attempts = Vec::new();
    if let Some(m) = manufacture {
        let attempt = match_on(m, candidates, Candidate::manufacture_key, "manufacture_date", max_distance, min_margin);
        if attempt.locked() {
            return attempt;
        }
        attempts.push(attempt);
    }
    if let Some(r) = recognized {
        let attempt = match_on(r, candidates, |c| Some(c.key()), "receipt_date", max_distance, min_margin);
        if attempt.locked() {
            return attempt;
        }
        attempts.push(attempt);
    }

    // Report the near miss over the blank one: "nothing was close" is more
    // actionable than "that field was empty on every lot".
    attempts.sort_by(|a, b| {
        let rank = |r: &MatchResult| (r.reason == Some(DeferReason::NoCandidates), r.best_distance.unwrap_or(9e9));
        let (ra, rb) = (rank(a), rank(b));
        ra.0.cmp(&rb.0).then(ra.1.total_cmp(&rb.1))
    });
    attempts.into_iter().next().unwrap_or_else(|| MatchResult::defer(DeferReason::NoRecognition))
}

/// Default knobs, as used by the issuing flow.
pub fn match_lot(recognized: Option<&DateKey>, candidates: &[Candidate], manufacture: Option<&DateKey>) -> MatchResult {
    match_candidates(recognized, candidates, manufacture, MAX_DISTANCE, MIN_MARGIN)
}

/// FIFO orders by 製造日期 — how old the stock actually is.
///
/// A lot with no manufacture date sorts LAST, not first: unknown age must not
/// jump ahead of known age. Receipt date breaks ties, then lot id so the order
/// is stable.
fn sort_key(c: &Candidate) -> (bool, &str, &str, &str) {
    (
        c.manufacture_date.is_none(),
        c.manufacture_date.as_deref().unwrap_or(""),
        c.receipt_date.as_str(),
        c.lot_id.as_str(),
    )
}

/// Lot ids FIFO accepts: every lot sharing the earliest 製造日期.
///
/// This is the JUDGEMENT: lots made the same day are equally legal, so drawing
/// any of them passes. Use [`fifo_target`] for what to TELL someone to take —
/// a screen marking two lots "應領" has said nothing.
///
/// With no manufacture date anywhere, this falls back to earliest receipt
/// date: a shelf of undated stock would otherwise get no guidance at all.
pub fn fifo_expected(candidates: &[Candidate]) -> Vec<String> {
    if candidates.is_empty() {
        return vec![];
    }
    let dated: Vec<&Candidate> =
        candidates.iter().filter(|c| c.manufacture_date.as_deref().is_some_and(|d| !d.is_empty())).collect();
    if dated.is_empty() {
        let earliest = candidates.iter().map(|c| c.receipt_date.as_str()).min().unwrap();
        return candidates.iter().filter(|c| c.receipt_date == earliest).map(|c| c.lot_id.clone()).collect();
    }
    let earliest = dated.iter().filter_map(|c| c.manufacture_date.as_deref()).min().unwrap();
    dated.into_iter().filter(|c| c.manufacture_date.as_deref() == Some(earliest)).map(|c| c.lot_id.clone()).collect()
}

/// The single lot to point at: earliest 製造日期, then earliest 進貨日期.
///
/// Judgement stays permissive because refusing a same-age lot would block a
/// draw that is genuinely fine; guidance has to name one box, because "take
/// either of these two" is not an instruction anyone can act on while holding
/// a roll of film.
pub fn fifo_target(candidates: &[Candidate]) -> Option<String> {
    let accepted = fifo_expected(candidates);
    candidates
        .iter()
        .filter(|c| accepted.contains(&c.lot_id))
        .min_by(|a, b| sort_key(a).cmp(&sort_key(b)))
        .map(|c| c.lot_id.clone())
}

/// Which field the current guidance actually rests on. Guidance derived from
/// receipt date because nothing carries a manufacture date is a weaker claim,
/// and saying so is the difference between a stated fallback and a silent
/// downgrade.
pub fn fifo_basis(candidates: &[Candidate]) -> &'static str {
    if candidates.iter().any(|c| c.manufacture_date.as_deref().is_some_and(|d| !d.is_empty())) {
        "製造日期"
    } else {
        "進貨日期"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dk(raw: &str) -> DateKey {
        to_date_key(Some(raw)).unwrap()
    }

    fn stock() -> Vec<Candidate> {
        vec![
            Candidate::new("L1", "2026-08-12", None),
            Candidate::new("L2", "2026-06-03", None),
            Candidate::new("L3", "2026-04-10", None),
        ]
    }

    #[test]
    fn clean_read_locks() {
        let r = match_lot(Some(&dk("2026-08-12")), &stock(), None);
        assert!(r.locked());
        assert_eq!(r.lot_id.as_deref(), Some("L1"));
    }

    #[test]
    fn ocr_damage_is_rescued_by_the_candidate_set() {
        let r = match_lot(Some(&dk("2026-08-l2")), &stock(), None);
        assert!(r.locked());
        assert_eq!(r.lot_id.as_deref(), Some("L1"));
    }

    #[test]
    fn a_hallucinated_date_matches_nothing() {
        let r = match_lot(Some(&dk("2026-01-15")), &stock(), None);
        assert!(!r.locked());
        assert_eq!(r.reason, Some(DeferReason::NoCandidateInRange));
    }

    #[test]
    fn two_close_lots_defer_rather_than_guess() {
        let close = vec![Candidate::new("A", "2026-08-12", None), Candidate::new("B", "2026-08-13", None)];
        let r = match_candidates(Some(&dk("2026-08-1X")), &close, None, MAX_DISTANCE, 1.0);
        assert!(!r.locked());
        assert_eq!(r.reason, Some(DeferReason::Ambiguous));
    }

    #[test]
    fn margin_can_be_tightened_to_trade_hits_for_safety() {
        let close = vec![Candidate::new("A", "2026-08-12", None), Candidate::new("B", "2026-08-13", None)];
        assert!(match_candidates(Some(&dk("2026-08-l2")), &close, None, MAX_DISTANCE, 1.0).locked());
        assert!(!match_candidates(Some(&dk("2026-08-l2")), &close, None, MAX_DISTANCE, 1.5).locked());
    }

    #[test]
    fn empty_stock_defers() {
        let r = match_lot(Some(&dk("2026-08-12")), &[], None);
        assert_eq!(r.reason, Some(DeferReason::NoCandidates));
    }

    #[test]
    fn no_recognition_defers() {
        let r = match_lot(None, &stock(), None);
        assert_eq!(r.reason, Some(DeferReason::NoRecognition));
    }

    fn scrambled() -> Vec<Candidate> {
        vec![
            Candidate::new("new", "2026-03-10", Some("2026-01-20")),
            Candidate::new("old", "2026-08-12", Some("2025-06-15")),
            Candidate::new("mid", "2026-05-12", Some("2025-11-30")),
        ]
    }

    #[test]
    fn earliest_manufacture_date_wins_not_earliest_receipt() {
        assert_eq!(fifo_expected(&scrambled()), vec!["old"]);
        assert_eq!(fifo_target(&scrambled()).as_deref(), Some("old"));
    }

    #[test]
    fn lots_made_the_same_day_are_equally_legal() {
        let same = vec![
            Candidate::new("a", "2026-04-10", Some("2025-09-26")),
            Candidate::new("b", "2026-07-01", Some("2025-09-26")),
        ];
        let mut got = fifo_expected(&same);
        got.sort();
        assert_eq!(got, vec!["a", "b"]);
    }

    #[test]
    fn same_manufacture_date_breaks_on_receipt_date() {
        let same = vec![
            Candidate::new("later", "2026-07-01", Some("2025-09-26")),
            Candidate::new("earlier", "2026-04-10", Some("2025-09-26")),
        ];
        assert_eq!(fifo_target(&same).as_deref(), Some("earlier"));
    }

    #[test]
    fn undated_stock_does_not_jump_the_queue() {
        let mixed = vec![
            Candidate::new("undated", "2026-01-01", None),
            Candidate::new("dated", "2026-08-01", Some("2025-12-01")),
        ];
        assert_eq!(fifo_expected(&mixed), vec!["dated"]);
        assert_eq!(fifo_target(&mixed).as_deref(), Some("dated"));
    }

    #[test]
    fn falls_back_to_receipt_date_when_nothing_is_dated() {
        let undated = vec![Candidate::new("later", "2026-08-01", None), Candidate::new("earlier", "2026-02-01", None)];
        assert_eq!(fifo_expected(&undated), vec!["earlier"]);
        assert_eq!(fifo_target(&undated).as_deref(), Some("earlier"));
    }

    #[test]
    fn basis_reports_which_field_guidance_rests_on() {
        assert_eq!(fifo_basis(&scrambled()), "製造日期");
        assert_eq!(fifo_basis(&[Candidate::new("x", "2026-01-01", None)]), "進貨日期");
    }

    #[test]
    fn no_stock() {
        assert!(fifo_expected(&[]).is_empty());
        assert!(fifo_target(&[]).is_none());
    }

    fn by_manufacture() -> Vec<Candidate> {
        vec![
            Candidate::new("13", "2026-06-09", Some("2025-09-26")),
            Candidate::new("14", "2026-07-07", Some("2026-03-22")),
        ]
    }

    #[test]
    fn printed_manufacture_date_locks_when_the_stamp_matches_nothing() {
        let r = match_lot(Some(&dk("2026-08-12")), &by_manufacture(), Some(&dk("2025年09月26日")));
        assert!(r.locked());
        assert_eq!(r.lot_id.as_deref(), Some("13"));
        assert_eq!(r.matched_on, Some("manufacture_date"));
    }

    #[test]
    fn day_first_label_date_matches_its_own_lot() {
        let r = match_lot(None, &by_manufacture(), Some(&dk("22.03.2026")));
        assert!(r.locked());
        assert_eq!(r.lot_id.as_deref(), Some("14"));
    }

    #[test]
    fn receipt_date_still_locks_when_no_manufacture_date_was_read() {
        let r = match_lot(Some(&dk("2026-07-07")), &by_manufacture(), None);
        assert!(r.locked());
        assert_eq!(r.lot_id.as_deref(), Some("14"));
        assert_eq!(r.matched_on, Some("receipt_date"));
    }

    #[test]
    fn receipt_date_separates_two_lots_made_the_same_day() {
        let same = vec![
            Candidate::new("a", "2026-06-09", Some("2026-03-22")),
            Candidate::new("b", "2026-07-07", Some("2026-03-22")),
        ];
        let r = match_lot(Some(&dk("2026-07-07")), &same, Some(&dk("2026-03-22")));
        assert!(r.locked());
        assert_eq!(r.lot_id.as_deref(), Some("b"));
        assert_eq!(r.matched_on, Some("receipt_date"));
    }

    #[test]
    fn a_manufacture_date_matching_nothing_does_not_force_a_lock() {
        let r = match_lot(None, &by_manufacture(), Some(&dk("2019-01-01")));
        assert!(!r.locked());
        assert_eq!(r.reason, Some(DeferReason::NoCandidateInRange));
    }

    #[test]
    fn undated_lots_are_skipped_by_the_manufacture_pass() {
        let undated = vec![Candidate::new("x", "2026-06-09", None)];
        assert!(!match_lot(None, &undated, Some(&dk("2025-09-26"))).locked());
    }

    #[test]
    fn reading_neither_date_defers() {
        assert_eq!(match_lot(None, &by_manufacture(), None).reason, Some(DeferReason::NoRecognition));
    }

    fn master() -> Vec<MasterRow> {
        let row = |id: &str, model: Option<&str>, code: Option<&str>| {
            (id.to_string(), model.map(str::to_string), code.map(str::to_string))
        };
        vec![
            row("1", Some("T7320BC"), Some("2003.T7320BC-340X900-P1")),
            row("2", Some("T6240BA"), Some("2003.T6240BA-334X600")),
            row("3", Some("T6050BSW"), None),
            row("4", None, None),
        ]
    }

    fn rows(pairs: &[(&str, &str)]) -> Vec<MasterRow> {
        pairs.iter().map(|(id, m)| (id.to_string(), Some(m.to_string()), None)).collect()
    }

    #[test]
    fn exact_supplier_code() {
        let r = match_item_code(Some("2003.T7320BC-340X900-P1"), &master(), None);
        assert!(r.locked());
        assert_eq!(r.item_id.as_deref(), Some("1"));
        assert_eq!(r.matched_on, Some("supplier_code"));
    }

    #[test]
    fn punctuation_and_case_are_ignored() {
        let r = match_item_code(Some("2003 t7320bc 340x900 p1"), &master(), None);
        assert_eq!(r.item_id.as_deref(), Some("1"));
    }

    #[test]
    fn model_found_inside_an_unregistered_label() {
        let r = match_item_code(Some("2003.T6050BSW-300X600"), &master(), None);
        assert_eq!(r.item_id.as_deref(), Some("3"));
        assert_eq!(r.matched_on, Some("model_in_label"));
    }

    #[test]
    fn longer_model_wins_over_its_own_prefix() {
        let r =
            match_item_code(Some("2003.T6050BSW-300X600"), &rows(&[("short", "T6050B"), ("long", "T6050BSW")]), None);
        assert_eq!(r.item_id.as_deref(), Some("long"));
    }

    #[test]
    fn a_prefix_of_another_model_is_not_ambiguity() {
        let r = match_item_code(Some("XX-T6050BSW"), &rows(&[("full", "T6050BSW"), ("partial", "6050BSW")]), None);
        assert_eq!(r.item_id.as_deref(), Some("full"));
    }

    #[test]
    fn two_equally_specific_models_defer() {
        let r = match_item_code(Some("T6050BS / T6350BS"), &rows(&[("a", "T6050BS"), ("b", "T6350BS")]), None);
        assert!(!r.locked());
        assert_eq!(r.reason, Some(ItemDeferReason::AmbiguousItem));
        assert_eq!(r.contenders, vec!["a", "b"]);
    }

    #[test]
    fn unknown_label() {
        let r = match_item_code(Some("9999.XXXX-1"), &master(), None);
        assert_eq!(r.reason, Some(ItemDeferReason::NoItemMatch));
    }

    #[test]
    fn an_item_with_no_model_is_simply_unmatchable() {
        let r = match_item_code(Some("2003.SOMETHING-ELSE"), &[("4".into(), None, None)], None);
        assert_eq!(r.reason, Some(ItemDeferReason::NoItemMatch));
    }

    #[test]
    fn model_read_on_its_own() {
        let r = match_item_code(None, &master(), Some("T6240BA"));
        assert_eq!(r.item_id.as_deref(), Some("2"));
    }

    #[test]
    fn model_and_full_code_together() {
        let r = match_item_code(Some("2003.T7320BC-340X900-P1"), &master(), Some("T7320BC"));
        assert_eq!(r.item_id.as_deref(), Some("1"));
    }

    #[test]
    fn a_model_not_in_the_master_defers() {
        let r = match_item_code(None, &master(), Some("T6284BA"));
        assert_eq!(r.reason, Some(ItemDeferReason::NoItemMatch));
    }

    #[test]
    fn unreadable_label() {
        for value in [None, Some(""), Some("   ")] {
            let r = match_item_code(value, &master(), None);
            assert_eq!(r.reason, Some(ItemDeferReason::NoCodeRead), "{value:?}");
        }
    }
}
