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

# 原物料名稱 identifies an item; 型號 is optional. The 脫氧劑 line on the real
# acceptance form has no model number at all, which is why 型號 cannot be the key
# — and why that item can never be identified from a photo.
ITEMS = [
    # 原物料名稱, 型號, 規格, 保存天數, 安全水位, 每箱米數, 箱上完整料號, 廠商
    ("高阻氧食品包裝拉伸膜", "T7320BC", "340mm x 900M", 540, 3, 900,
     "2003.T7320BC-340X900-P1", "臺灣希悅爾"),
    ("食品包裝拉伸膜", "T6240BA", "334mm x 600M", 540, 2, 600,
     "2003.T6240BA-334X600", "臺灣希悅爾"),
    ("食品包裝拉伸膜", "T6050BSW", "300mm x 600M", 540, 2, 600, None, "臺灣希悅爾"),
    # 無型號: 只能人工選, 不走影像辨識
    ("脫氧劑", None, "100cc", 730, 5, None, None, "弘東京"),
]

LOTS = [
    # 品項索引(對應 ITEMS), 進貨日, 製造日, 有效日, 原廠 ROLL#, 箱數, 單上數量, 單位, 判定, 備註
    (0, TODAY - timedelta(days=164), "2025-09-26", None, "20250915-3081*61", 2, 1800, "米", "合格", None),
    (0, TODAY - timedelta(days=101), "2026-01-18", None, "20260105-2214*08", 3, 2700, "米", "合格", None),
    (0, TODAY - timedelta(days=9),   "2026-06-02", None, "20260528-4417*22", 4, 3600, "米", "合格", None),
    (1, TODAY - timedelta(days=41),  "2025-06-01", None, "20250520-1180*13", 1,  600, "米", "合格", None),
    # 同一次到貨、兩個製造日 → 兩筆 (單上寫在同一格, 系統分批)
    (2, TODAY - timedelta(days=2),   "2025-08-19", None, "20250815-7781*30", 8, 7200, "米", "合格", None),
    (2, TODAY - timedelta(days=2),   "2025-09-26", None, "20250920-7802*11", 12, 10800, "米", "合格", None),
    # 檢驗不合格: 留紀錄, 不計在庫, FIFO 不指, 領不出來
    (1, TODAY - timedelta(days=5),   "2026-02-10", None, "20260205-3310*07", 10, 6000, "米", "不合格", "外觀破損"),
    # 無型號的品項也要能收貨與領用, 只是走人工
    (3, TODAY - timedelta(days=20),  "2026-01-05", "2028-01-05", None, 6, 6, "包", "合格", None),
]

INSPECTION_PASS = '{"規格尺寸": true, "標示製造日期": true, "標示有效日期": null, "外觀": true, "顏色": true}'
INSPECTION_FAIL = '{"規格尺寸": true, "標示製造日期": true, "標示有效日期": null, "外觀": false, "顏色": true}'


# Seeds the dropdowns that are not attributes of a 型號. Machines and products
# come off the paper form's own notes (「包裝機台編號：1號 D-003…」,
# 「肉乾類產品須填寫包裝產品（例如：經典、豬切…）」), so they are this factory's
# configuration, not the product's.
# Accounts covering the roles on the form: the person who writes the line, the
# person who confirms it, the line operator, and someone to manage the rest.
# Demo passwords — a real deployment sets these per person on first login.
# Role is the permission tier; title is the job. 倉管 and 廠長 are different
# jobs needing the same permissions, which is exactly why they are separate.
USERS = [
    # 帳號, 顯示名(簽核用), 職位, 角色, 密碼
    ("kuo", "郭", "倉管", "manager", "urdwms2026"),
    ("huang", "黃揚文", "廠長", "manager", "urdwms2026"),
    ("operator", "線上作業員", "包裝線作業員", "user", "urdwms2026"),
    ("peter", "Peter", "資訊", "admin", "peter0821"),
]

DICTIONARY = {
    "job_title": ["倉管", "廠長", "品管", "線長", "包裝線作業員", "資訊"],
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
        item_ids: list[int] = []
        for name, model, spec, shelf, safety, meters, long_code, supplier in ITEMS:
            cursor = conn.execute(
                "INSERT INTO inventory_item (name, model, spec, unit, shelf_life_days,"
                " safety_stock, meters_per_box, supplier_code, supplier)"
                " VALUES (?,?,?,'箱',?,?,?,?,?)",
                (name, model, spec, shelf, safety, meters, long_code, supplier))
            item_ids.append(cursor.lastrowid)
        for idx, receipt, manufacture, expiry, roll, qty, entered, unit, verdict, remark in LOTS:
            conn.execute(
                "INSERT INTO inventory_lot (item_id, receipt_date, manufacture_date, expiry_date,"
                " supplier_lot_code, supplier, entered_meters, entered_unit, inspection, verdict,"
                " recorded_by, confirmed_by, remark, qty_on_hand, created_at, created_by)"
                " VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,'seed')",
                (item_ids[idx], receipt.isoformat(), manufacture, expiry, roll,
                 ITEMS[idx][7], entered, unit,
                 INSPECTION_PASS if verdict == "合格" else INSPECTION_FAIL, verdict,
                 "郭", "黃揚文", remark, qty, now()))
        from app.auth import DEFAULT_ROLE_LABELS, hash_password
        for order, (code, label) in enumerate(DEFAULT_ROLE_LABELS.items()):
            conn.execute("INSERT OR IGNORE INTO app_role (code, label, sort_order) VALUES (?,?,?)",
                         (code, label, order))
        for username, name, title, role, password in USERS:
            conn.execute(
                "INSERT OR IGNORE INTO app_user (username, name, title, role, password_hash, created_at)"
                " VALUES (?,?,?,?,?,?)",
                (username, name, title, role, hash_password(password), now()))

        for category, values in DICTIONARY.items():
            for order, value in enumerate(values):
                conn.execute(
                    "INSERT OR IGNORE INTO dictionary (category, value, sort_order, created_at)"
                    " VALUES (?,?,?,?)", (category, value, order, now()))
        log(conn, "seed", "seed.run", {"items": len(ITEMS), "lots": len(LOTS), "users": len(USERS),
                                       "dictionary": sum(len(v) for v in DICTIONARY.values())})
    print(f"seeded {len(ITEMS)} items, {len(LOTS)} lots, {len(USERS)} users")
    print("  登入帳號：" + "、".join(f"{u}（{n}·{t}·{r}）" for u, n, t, r, _ in USERS))
    print(f"  demo 密碼：{USERS[0][4]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
