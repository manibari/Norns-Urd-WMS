"""The four label date formats are real, observed on one box (requirement section 2A/2B)."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from urdwms_poc.normalize import confusion_distance, normalize_item_code, to_date_key


class DateFormats(unittest.TestCase):
    def test_all_four_field_formats_reach_the_same_key(self):
        for raw in ("2025年09月26日", "26-Sep-25", "2025-09-26", "20250926"):
            with self.subTest(raw=raw):
                parsed = to_date_key(raw)
                assert parsed is not None
                self.assertEqual(parsed.key, "20250926")
                self.assertEqual(parsed.iso, "2025-09-26")

    def test_stamp_format(self):
        parsed = to_date_key("2026-08-12")
        assert parsed is not None
        self.assertEqual(parsed.iso, "2026-08-12")

    def test_ocr_damage_degrades_instead_of_vanishing(self):
        # The whole design rests on this: an unreadable value must still be
        # comparable, because the matcher can rescue it from the candidate set.
        parsed = to_date_key("2026-08-l2")
        assert parsed is not None
        self.assertEqual(parsed.key, "202608l2")
        self.assertIsNone(parsed.iso)
        self.assertFalse(parsed.is_clean)

    def test_a_partly_unreadable_field_never_looks_clean(self):
        # Regression. A narrow character class used to let the regex match a
        # PREFIX of a damaged day and drop the rest, so `2026-08-1X` parsed as
        # a confident `2026-08-01`. Unreadable dressed up as readable is the
        # exact shape of a false hit: if 08-01 is in stock, the matcher locks
        # it and nothing anywhere reports a problem.
        parsed = to_date_key("2026-08-1X")
        assert parsed is not None
        self.assertIsNone(parsed.iso, "a damaged field must not produce a clean date")
        self.assertEqual(parsed.key, "2026081X")

    def test_impossible_dates_are_not_clean(self):
        parsed = to_date_key("2026-19-45")
        assert parsed is not None
        self.assertIsNone(parsed.iso)

    def test_nothing_date_shaped(self):
        for raw in ("", "   ", None, "高阻氧食品包裝拉伸膜"):
            with self.subTest(raw=raw):
                self.assertIsNone(to_date_key(raw))


class ConfusionDistance(unittest.TestCase):
    def test_ocr_confusable_substitution_is_half_price(self):
        self.assertEqual(confusion_distance("202608l2", "20260812"), 0.5)
        self.assertEqual(confusion_distance("2O26O812", "20260812"), 1.0)

    def test_a_genuinely_different_digit_costs_full(self):
        self.assertEqual(confusion_distance("20260892", "20260812"), 1.0)

    def test_a_different_month_is_far(self):
        self.assertGreaterEqual(confusion_distance("20260112", "20260812"), 1.0)

    def test_identical(self):
        self.assertEqual(confusion_distance("20260812", "20260812"), 0.0)


class ItemCodes(unittest.TestCase):
    ALIASES = {"T7320-P1": "2003.T7320BC-340X900-P1"}

    def test_form_abbreviation_resolves_to_canonical(self):
        self.assertEqual(normalize_item_code("T7320-P1", self.ALIASES), "2003.T7320BC-340X900-P1")

    def test_canonical_passes_through(self):
        self.assertEqual(
            normalize_item_code("2003.T7320BC-340X900-P1", self.ALIASES),
            "2003.T7320BC-340X900-P1",
        )

    def test_whitespace_and_case(self):
        self.assertEqual(normalize_item_code(" t7320-p1 ", self.ALIASES), "2003.T7320BC-340X900-P1")

    def test_unknown_code_is_returned_not_dropped(self):
        # An unconfigured code must survive to the matcher, which will fail to
        # find candidates and defer — that is a visible outcome. Silently
        # returning None here would look like a recognition failure instead.
        self.assertEqual(normalize_item_code("2003.T6240BA-334X600", self.ALIASES), "2003.T6240BA-334X600")


if __name__ == "__main__":
    unittest.main()
