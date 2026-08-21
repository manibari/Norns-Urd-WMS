"""Which 型號 is this box? Same rule as the lot matcher: unique or defer."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "core"))

from urdwms_core.matching import ItemDeferReason, match_item_code

# (id, 型號, 箱上完整料號)。型號可以是 None —— 驗收單上「脫氧劑」那列就沒有型號。
MASTER = [
    ("1", "T7320BC", "2003.T7320BC-340X900-P1"),
    ("2", "T6240BA", "2003.T6240BA-334X600"),
    ("3", "T6050BSW", None),
    ("4", None, None),          # 脫氧劑: 無型號、無標籤料號
]


class ItemMatching(unittest.TestCase):
    def test_exact_supplier_code(self):
        result = match_item_code("2003.T7320BC-340X900-P1", MASTER)
        self.assertTrue(result.locked)
        self.assertEqual(result.item_id, "1")
        self.assertEqual(result.matched_on, "supplier_code")

    def test_punctuation_and_case_are_ignored(self):
        result = match_item_code("2003 t7320bc 340x900 p1", MASTER)
        self.assertTrue(result.locked)
        self.assertEqual(result.item_id, "1")

    def test_model_found_inside_an_unregistered_label(self):
        # T6050BSW has no supplier_code on file, but the label still names it.
        result = match_item_code("2003.T6050BSW-300X600", MASTER)
        self.assertTrue(result.locked)
        self.assertEqual(result.item_id, "3")
        self.assertEqual(result.matched_on, "model_in_label")

    def test_longer_model_wins_over_its_own_prefix(self):
        # A label naming T6050BSW also contains T6050B. Picking the shorter one
        # would draw stock from a different item and never say so.
        master = [("short", "T6050B", None), ("long", "T6050BSW", None)]
        result = match_item_code("2003.T6050BSW-300X600", master)
        self.assertTrue(result.locked)
        self.assertEqual(result.item_id, "long")

    def test_a_prefix_of_another_model_is_not_ambiguity(self):
        # Only one reading is specific; the shorter code is a substring artefact.
        master = [("full", "T6050BSW", None), ("partial", "6050BSW", None)]
        result = match_item_code("XX-T6050BSW", master)
        self.assertTrue(result.locked)
        self.assertEqual(result.item_id, "full")

    def test_two_equally_specific_models_defer(self):
        # 肉乾上下膜 are a pair (form note 2), so a label or sheet naming both is
        # realistic. Neither reading is more specific — a human decides.
        master = [("a", "T6050BS", None), ("b", "T6350BS", None)]
        result = match_item_code("T6050BS / T6350BS", master)
        self.assertFalse(result.locked)
        self.assertEqual(result.reason, ItemDeferReason.AMBIGUOUS_ITEM)
        self.assertEqual(set(result.contenders), {"a", "b"})

    def test_unknown_label(self):
        result = match_item_code("9999.XXXX-1", MASTER)
        self.assertFalse(result.locked)
        self.assertEqual(result.reason, ItemDeferReason.NO_ITEM_MATCH)

    def test_an_item_with_no_model_is_simply_unmatchable(self):
        # 型號 is optional, so some items can never be identified from a label.
        # Not a failure — the operator picks it by hand (US-4).
        result = match_item_code("2003.SOMETHING-ELSE", [("4", None, None)])
        self.assertFalse(result.locked)
        self.assertEqual(result.reason, ItemDeferReason.NO_ITEM_MATCH)

    def test_model_read_on_its_own(self):
        # Recognition reports the 型號 it saw; the label may carry nothing longer.
        result = match_item_code(None, MASTER, model_code="T6240BA")
        self.assertTrue(result.locked)
        self.assertEqual(result.item_id, "2")

    def test_model_and_full_code_together(self):
        result = match_item_code("2003.T7320BC-340X900-P1", MASTER, model_code="T7320BC")
        self.assertTrue(result.locked)
        self.assertEqual(result.item_id, "1")

    def test_a_model_not_in_the_master_defers(self):
        # T6284BA is a real code off the form, but if it is not registered the
        # system must say so rather than pick something adjacent.
        result = match_item_code(None, MASTER, model_code="T6284BA")
        self.assertFalse(result.locked)
        self.assertEqual(result.reason, ItemDeferReason.NO_ITEM_MATCH)

    def test_unreadable_label(self):
        for value in (None, "", "   "):
            with self.subTest(value=value):
                result = match_item_code(value, MASTER)
                self.assertFalse(result.locked)
                self.assertEqual(result.reason, ItemDeferReason.NO_CODE_READ)


if __name__ == "__main__":
    unittest.main()
