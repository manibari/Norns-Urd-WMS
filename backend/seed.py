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

LOTS = [
    # 型號, 進貨日, 製造日, 原廠 ROLL#, 箱數
    ("T7320BC",  TODAY - timedelta(days=164), "2025-09-26", "20250915-3081*61", 2),
    ("T7320BC",  TODAY - timedelta(days=101), "2026-01-18", "20260105-2214*08", 3),
    ("T7320BC",  TODAY - timedelta(days=9),   "2026-06-02", "20260528-4417*22", 4),
    ("T6240BA",  TODAY - timedelta(days=41),  "2025-06-01", "20250520-1180*13", 1),
]


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
        for code, receipt, manufacture, supplier, qty in LOTS:
            conn.execute(
                "INSERT INTO inventory_lot (item_code, receipt_date, manufacture_date, supplier_lot_code,"
                " supplier, entered_meters, qty_on_hand, created_at, created_by)"
                " VALUES (?,?,?,?,?,?,?,?,'seed')",
                (code, receipt.isoformat(), manufacture, supplier, "臺灣希悅爾", None, qty, now()))
        log(conn, "seed", "seed.run", {"items": len(ITEMS), "lots": len(LOTS)})
    print(f"seeded {len(ITEMS)} items, {len(LOTS)} lots")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
