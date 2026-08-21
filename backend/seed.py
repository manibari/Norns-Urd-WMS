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

ITEMS = [
    # code, name, spec, shelf_life_days, safety_stock
    ("2003.T7320BC-340X900-P1", "高阻氧食品包裝拉伸膜", "340mm x 900M", 540, 3),
    ("2003.T6240BA-334X600", "食品包裝拉伸膜", "334mm x 600M", 540, 2),
]

LOTS = [
    # item_code, receipt_date, manufacture_date, supplier_lot, qty
    ("2003.T7320BC-340X900-P1", TODAY - timedelta(days=164), "2025-09-26", "20250915-3081*61", 2),
    ("2003.T7320BC-340X900-P1", TODAY - timedelta(days=101), "2026-01-18", "20260105-2214*08", 3),
    ("2003.T7320BC-340X900-P1", TODAY - timedelta(days=9),   "2026-06-02", "20260528-4417*22", 4),
    ("2003.T6240BA-334X600",    TODAY - timedelta(days=41),  "2025-06-01", "20250520-1180*13", 1),
]


def main() -> int:
    init_db()
    with transaction() as conn:
        existing = conn.execute("SELECT COUNT(*) AS n FROM inventory_lot").fetchone()["n"]
        if existing:
            print(f"already seeded ({existing} lots) — nothing to do")
            return 0
        for code, name, spec, shelf, safety in ITEMS:
            conn.execute(
                "INSERT OR IGNORE INTO inventory_item (item_code, name, spec, unit, shelf_life_days, safety_stock)"
                " VALUES (?,?,?,'箱',?,?)", (code, name, spec, shelf, safety))
        for code, receipt, manufacture, supplier, qty in LOTS:
            conn.execute(
                "INSERT INTO inventory_lot (item_code, receipt_date, manufacture_date, supplier_lot_code,"
                " qty_on_hand, created_at, created_by) VALUES (?,?,?,?,?,?,'seed')",
                (code, receipt.isoformat(), manufacture, supplier, qty, now()))
        log(conn, "seed", "seed.run", {"items": len(ITEMS), "lots": len(LOTS)})
    print(f"seeded {len(ITEMS)} items, {len(LOTS)} lots")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
