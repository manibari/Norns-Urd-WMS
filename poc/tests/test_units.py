"""Metres are an input aid; boxes are the ledger. The remainder must stay visible."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "core"))

from urdwms_core.units import boxes_from_meters, meters_from_boxes


class Conversion(unittest.TestCase):
    def test_exact_multiple(self):
        result = boxes_from_meters(9000, 900)
        assert result is not None
        self.assertEqual(result.boxes, 10)
        self.assertTrue(result.exact)
        self.assertIsNone(result.note)

    def test_remainder_is_reported_not_rounded(self):
        # 9,500 m is not 10.5 boxes. It is 10 boxes and 500 m that do not add up,
        # which the person receiving the delivery should see.
        result = boxes_from_meters(9500, 900)
        assert result is not None
        self.assertEqual(result.boxes, 10)
        self.assertEqual(result.remainder_m, 500)
        self.assertFalse(result.exact)
        assert result.note is not None
        self.assertIn("500", result.note)

    def test_less_than_one_box(self):
        result = boxes_from_meters(400, 900)
        assert result is not None
        self.assertEqual(result.boxes, 0)
        self.assertEqual(result.remainder_m, 400)

    def test_unconfigured_item_refuses_to_guess(self):
        for rate in (None, 0, -1):
            with self.subTest(rate=rate):
                self.assertIsNone(boxes_from_meters(9000, rate))

    def test_zero_and_negative_metres(self):
        result = boxes_from_meters(0, 900)
        assert result is not None
        self.assertEqual((result.boxes, result.remainder_m), (0, 0))

    def test_display_direction(self):
        self.assertEqual(meters_from_boxes(3, 900), 2700)
        self.assertIsNone(meters_from_boxes(3, None))


if __name__ == "__main__":
    unittest.main()
