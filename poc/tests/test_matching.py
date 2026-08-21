"""Matching decides the PoC's most important number, so its edges get pinned here."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from urdwms_poc.matching import Candidate, DeferReason, fifo_expected, match_candidates
from urdwms_poc.normalize import to_date_key

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
    def test_earliest_receipt_date_wins(self):
        self.assertEqual(fifo_expected(STOCK), ["L3"])

    def test_same_day_lots_are_equally_legal(self):
        same_day = STOCK + [Candidate("L4", "2026-04-10")]
        self.assertEqual(sorted(fifo_expected(same_day)), ["L3", "L4"])

    def test_no_stock(self):
        self.assertEqual(fifo_expected([]), [])


if __name__ == "__main__":
    unittest.main()
