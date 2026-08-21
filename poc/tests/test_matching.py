"""Matching decides the PoC's most important number, so its edges get pinned here."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "core"))

from urdwms_core.matching import (
    Candidate, DeferReason, fifo_basis, fifo_expected, fifo_target, match_candidates,
)
from urdwms_core.normalize import to_date_key

STOCK = [
    Candidate("L1", "2026-08-12"),
    Candidate("L2", "2026-06-03"),
    Candidate("L3", "2026-04-10"),
]


class Matching(unittest.TestCase):
    def test_clean_read_locks(self):
        result = match_candidates(to_date_key("2026-08-12"), STOCK)
        self.assertTrue(result.locked)
        self.assertEqual(result.lot_id, "L1")

    def test_ocr_damage_is_rescued_by_the_candidate_set(self):
        # Open-ended OCR would have to reject this. Against a closed set it is safe.
        result = match_candidates(to_date_key("2026-08-l2"), STOCK)
        self.assertTrue(result.locked)
        self.assertEqual(result.lot_id, "L1")

    def test_a_hallucinated_date_matches_nothing(self):
        # The central claim of requirement section 2.2, as an executable test.
        result = match_candidates(to_date_key("2026-01-15"), STOCK)
        self.assertFalse(result.locked)
        self.assertEqual(result.reason, DeferReason.NO_CANDIDATE_IN_RANGE)

    def test_two_close_lots_defer_rather_than_guess(self):
        close = [Candidate("A", "2026-08-12"), Candidate("B", "2026-08-13")]
        result = match_candidates(to_date_key("2026-08-1X"), close, min_margin=1.0)
        self.assertFalse(result.locked)
        self.assertEqual(result.reason, DeferReason.AMBIGUOUS)

    def test_margin_can_be_tightened_to_trade_hits_for_safety(self):
        close = [Candidate("A", "2026-08-12"), Candidate("B", "2026-08-13")]
        loose = match_candidates(to_date_key("2026-08-l2"), close, min_margin=1.0)
        strict = match_candidates(to_date_key("2026-08-l2"), close, min_margin=1.5)
        self.assertTrue(loose.locked)
        self.assertFalse(strict.locked)

    def test_empty_stock_defers(self):
        result = match_candidates(to_date_key("2026-08-12"), [])
        self.assertFalse(result.locked)
        self.assertEqual(result.reason, DeferReason.NO_CANDIDATES)

    def test_no_recognition_defers(self):
        result = match_candidates(None, STOCK)
        self.assertFalse(result.locked)
        self.assertEqual(result.reason, DeferReason.NO_RECOGNITION)


class Fifo(unittest.TestCase):
    """FIFO orders by 製造日期 — actual stock age, not arrival order."""

    # Deliberately scrambled: the oldest film arrived last.
    STOCK = [
        Candidate("new", "2026-03-10", "2026-01-20"),
        Candidate("old", "2026-08-12", "2025-06-15"),
        Candidate("mid", "2026-05-12", "2025-11-30"),
    ]

    def test_earliest_manufacture_date_wins_not_earliest_receipt(self):
        # "old" arrived most recently but was made first, so it goes first.
        self.assertEqual(fifo_expected(self.STOCK), ["old"])
        self.assertEqual(fifo_target(self.STOCK), "old")

    def test_lots_made_the_same_day_are_equally_legal(self):
        same_day = [
            Candidate("a", "2026-04-10", "2025-09-26"),
            Candidate("b", "2026-07-01", "2025-09-26"),
        ]
        self.assertEqual(sorted(fifo_expected(same_day)), ["a", "b"])

    def test_same_manufacture_date_breaks_on_receipt_date(self):
        same_day = [
            Candidate("later", "2026-07-01", "2025-09-26"),
            Candidate("earlier", "2026-04-10", "2025-09-26"),
        ]
        self.assertEqual(fifo_target(same_day), "earlier")

    def test_undated_stock_does_not_jump_the_queue(self):
        # Pointing at a lot nobody can date, over one provably older, would be
        # worse than the paper process this replaces.
        mixed = [
            Candidate("undated", "2026-01-01", None),
            Candidate("dated", "2026-08-01", "2025-12-01"),
        ]
        self.assertEqual(fifo_expected(mixed), ["dated"])
        self.assertEqual(fifo_target(mixed), "dated")

    def test_falls_back_to_receipt_date_when_nothing_is_dated(self):
        # Otherwise a shelf of undated stock would have no guidance at all.
        undated = [
            Candidate("later", "2026-08-01", None),
            Candidate("earlier", "2026-02-01", None),
        ]
        self.assertEqual(fifo_expected(undated), ["earlier"])
        self.assertEqual(fifo_target(undated), "earlier")

    def test_basis_reports_which_field_guidance_rests_on(self):
        self.assertEqual(fifo_basis(self.STOCK), "製造日期")
        self.assertEqual(fifo_basis([Candidate("x", "2026-01-01", None)]), "進貨日期")

    def test_no_stock(self):
        self.assertEqual(fifo_expected([]), [])
        self.assertIsNone(fifo_target([]))


if __name__ == "__main__":
    unittest.main()


class MatchOnManufactureDate(unittest.TestCase):
    """製造日 identifies the lot too — and it is the printed field, not the stamp.

    Real case that motivated this (2026-08-21, IMG off the line): the box read
    章 2026-08-12 / 製造 2025年09月26日. The stamp matched no lot on file; the
    printed manufacture date matched exactly. Matching on the stamp alone left
    the operator picking the lot by hand on a box the system had already
    identified.
    """

    STOCK = [
        Candidate("13", "2026-06-09", "2025-09-26"),
        Candidate("14", "2026-07-07", "2026-03-22"),
    ]

    def test_printed_manufacture_date_locks_when_the_stamp_matches_nothing(self):
        result = match_candidates(to_date_key("2026-08-12"), self.STOCK,
                                  manufacture=to_date_key("2025年09月26日"))
        self.assertTrue(result.locked)
        self.assertEqual(result.lot_id, "13")
        self.assertEqual(result.matched_on, "manufacture_date")

    def test_receipt_date_still_locks_when_no_manufacture_date_was_read(self):
        result = match_candidates(to_date_key("2026-07-07"), self.STOCK)
        self.assertTrue(result.locked)
        self.assertEqual(result.lot_id, "14")
        self.assertEqual(result.matched_on, "receipt_date")

    def test_receipt_date_separates_two_lots_made_the_same_day(self):
        # 製造日 cannot tell these apart, which is exactly what the stamp is for.
        same_day = [Candidate("a", "2026-06-09", "2026-03-22"),
                    Candidate("b", "2026-07-07", "2026-03-22")]
        result = match_candidates(to_date_key("2026-07-07"), same_day,
                                  manufacture=to_date_key("2026-03-22"))
        self.assertTrue(result.locked)
        self.assertEqual(result.lot_id, "b")
        self.assertEqual(result.matched_on, "receipt_date")

    def test_a_manufacture_date_matching_nothing_does_not_force_a_lock(self):
        # The closed candidate set is the safety net: a date never received
        # must defer, not snap to the nearest lot (§2.2).
        result = match_candidates(None, self.STOCK, manufacture=to_date_key("2019-01-01"))
        self.assertFalse(result.locked)
        self.assertEqual(result.reason, DeferReason.NO_CANDIDATE_IN_RANGE)

    def test_undated_lots_are_skipped_by_the_manufacture_pass_not_matched_blindly(self):
        undated = [Candidate("x", "2026-06-09", None)]
        result = match_candidates(None, undated, manufacture=to_date_key("2025-09-26"))
        self.assertFalse(result.locked)

    def test_reading_neither_date_defers(self):
        result = match_candidates(None, self.STOCK)
        self.assertFalse(result.locked)
        self.assertEqual(result.reason, DeferReason.NO_RECOGNITION)
