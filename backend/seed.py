"""Seed the demo with the real item codes and a stock situation worth demonstrating.

The lot spread is deliberate: three lots of the film, received months apart, so
the FIFO check has something to actually catch. Without an older lot on hand,
every draw passes and the screen never shows what the product is for.
"""

from __future__ import annotations

import sys
from datetime import date, timedelta
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from app.db import init_db, log, now, transaction

TODAY = date.today()

# 型號 is the key — it is what the acceptance form asks for and what the metres
# table is written against. The long code beside it is what is printed on the
# box; it exists so recognition can map a label back to a 型號.
ITEMS = [
    # 型號, 品名, 規格, 保存天數, 安全水位, 每箱米數, 箱上完整料號, 廠商
    ("T7320BC", "高阻氧食品包裝拉伸膜", "340mm x 900M", 540, 3, 900,
     "2003.T7320BC-340X900-P1", "臺灣希悅爾"),
    ("T6240BA", "食品包裝拉伸膜", "334mm x 600M", 540, 2, 600,
     "2003.T6240BA-334X600", "臺灣希悅爾"),
    ("T6050BSW", "食品包裝拉伸膜", "300mm x 600M", 540, 2, 600, None, "臺灣希悅爾"),
]

# Each tuple is one line off the acceptance form. The set is chosen to make the
# situations the product exists for visible on a fresh install:
#   - three lots of T7320BC spread over months, so FIFO has something to catch
#   - one delivery arriving as TWO manufacture-date batches (the form's last row)
#   - one lot failing inspection, which must stay on the books but out of stock
LOTS = [
    # 型號, 進貨日, 製造日, 有效日, 原廠 ROLL#, 箱數, 單上數量, 單位, 判定, 備註
    ("T7320BC",  TODAY - timedelta(days=164), "2025-09-26", None, "20250915-3081*61", 2, 1800, "米", "合格", None),
    ("T7320BC",  TODAY - timedelta(days=101), "2026-01-18", None, "20260105-2214*08", 3, 2700, "米", "合格", None),
    ("T7320BC",  TODAY - timedelta(days=9),   "2026-06-02", None, "20260528-4417*22", 4, 3600, "米", "合格", None),
    ("T6240BA",  TODAY - timedelta(days=41),  "2025-06-01", None, "20250520-1180*13", 1,  600, "米", "合格", None),
    # 同一次到貨、兩個製造日 → 兩筆 (單上寫在同一格, 系統分批)
    ("T6050BSW", TODAY - timedelta(days=2),   "2025-08-19", None, "20250815-7781*30", 8, 7200, "米", "合格", None),
    ("T6050BSW", TODAY - timedelta(days=2),   "2025-09-26", None, "20250920-7802*11", 12, 10800, "米", "合格", None),
    # 檢驗不合格: 留紀錄, 不計在庫, FIFO 不指, 領不出來
    ("T6240BA",  TODAY - timedelta(days=5),   "2026-02-10", None, "20260205-3310*07", 10, 6000, "米", "不合格", "外觀破損"),
]

INSPECTION_PASS = '{"規格尺寸": true, "標示製造日期": true, "標示有效日期": null, "外觀": true, "顏色": true}'
INSPECTION_FAIL = '{"規格尺寸": true, "標示製造日期": true, "標示有效日期": null, "外觀": false, "顏色": true}'


# Seeds the dropdowns. Machines and products come off the paper form's notes
# (「包裝機台編號：1號 D-003…」, 「肉乾類產品須填寫包裝產品（例如：經典、豬切…）」),
# so they are this factory's configuration, not the product's.
DICTIONARY = {
    "supplier": ["臺灣希悅爾", "弘東京", "益壽"],
    "material_name": ["高阻氧食品包裝拉伸膜", "食品包裝拉伸膜", "真空切片休閒下膜",
                      "大包裝肉乾上膜", "大包裝肉乾下膜", "休閒豬上膜", "脫氧劑"],
    "spec": ["340mm x 900M", "334mm x 600M", "300mm x 600M", "40x75x280mm"],
    "staff": ["郭", "黃揚文", "黃海山"],
    "machine": ["D-003", "D-004", "D-023", "D-027"],
    "packed_product": ["經典", "豬切", "牛切", "休閒豬", "金尊", "海香", "魚絲", "雞脆", "豬脆", "香酥"],
    "override_reason": ["試產指定批", "舊批破損不可用", "規格臨時變更", "急單", "其他"],
}


def main() -> int:
    init_db()
    with transaction() as conn:
        existing = conn.execute("SELECT COUNT(*) AS n FROM inventory_lot").fetchone()["n"]
        if existing:
            print(f"already seeded ({existing} lots) — nothing to do")
            return 0
        for code, name, spec, shelf, safety, meters, long_code, supplier in ITEMS:
            conn.execute(
                "INSERT OR IGNORE INTO inventory_item (item_code, name, spec, unit, shelf_life_days,"
                " safety_stock, meters_per_box, supplier_code, supplier)"
                " VALUES (?,?,?,'箱',?,?,?,?,?)",
                (code, name, spec, shelf, safety, meters, long_code, supplier))
        for code, receipt, manufacture, expiry, roll, qty, entered, unit, verdict, remark in LOTS:
            conn.execute(
                "INSERT INTO inventory_lot (item_code, receipt_date, manufacture_date, expiry_date,"
                " supplier_lot_code, supplier, entered_meters, entered_unit, inspection, verdict,"
                " recorded_by, confirmed_by, remark, qty_on_hand, created_at, created_by)"
                " VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,'seed')",
                (code, receipt.isoformat(), manufacture, expiry, roll, "臺灣希悅爾", entered, unit,
                 INSPECTION_PASS if verdict == "合格" else INSPECTION_FAIL, verdict,
                 "郭", "黃揚文", remark, qty, now()))
        for category, values in DICTIONARY.items():
            for order, value in enumerate(values):
                conn.execute(
                    "INSERT OR IGNORE INTO dictionary (category, value, sort_order, created_at)"
                    " VALUES (?,?,?,?)", (category, value, order, now()))
        log(conn, "seed", "seed.run", {"items": len(ITEMS), "lots": len(LOTS),
                                       "dictionary": sum(len(v) for v in DICTIONARY.values())})
    print(f"seeded {len(ITEMS)} items, {len(LOTS)} lots")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
