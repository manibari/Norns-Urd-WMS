"""Urd-WMS vertical slice — receiving, image-recognised issuing, FIFO, traceability, alerts.

Scope is one working path end to end, not the full v1 (requirement US-1/2/3/4/5
plus the US-8 alerts). Everything here uses the same core the PoC measured, so
what you see on screen is the behaviour that was measured, not a mock of it.
"""

from __future__ import annotations

import json
import os
import sys
from datetime import date, datetime, timedelta
from pathlib import Path

from fastapi import FastAPI, File, Form, HTTPException, UploadFile
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "core"))

from urdwms_core.matching import Candidate, fifo_expected, match_candidates  # noqa: E402
from urdwms_core.normalize import to_date_key  # noqa: E402
from urdwms_core.recognition import GeminiProvider, Recognition  # noqa: E402

from .db import init_db, log, now, transaction  # noqa: E402

UPLOADS = Path(__file__).resolve().parents[1] / "uploads"
DEMO_USER = "demo.operator"

app = FastAPI(title="Urd-WMS", version="0.1.0")
app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://localhost:3071", "http://127.0.0.1:3071"],
    allow_methods=["*"],
    allow_headers=["*"],
)


@app.on_event("startup")
def startup() -> None:
    init_db()
    UPLOADS.mkdir(parents=True, exist_ok=True)
    app.mount("/uploads", StaticFiles(directory=UPLOADS), name="uploads")


# --------------------------------------------------------------------------- helpers

def _candidates(conn, item_code: str) -> list[Candidate]:
    rows = conn.execute(
        "SELECT id, receipt_date FROM inventory_lot WHERE item_code = ? AND qty_on_hand > 0"
        " ORDER BY receipt_date",
        (item_code,),
    ).fetchall()
    return [Candidate(str(r["id"]), r["receipt_date"]) for r in rows]


def _fifo_verdict(candidates: list[Candidate], lot_id: str) -> tuple[bool, str | None]:
    """FIFO compares receipt dates, so lots received the same day are equally legal."""
    expected = fifo_expected(candidates)
    if not expected:
        return False, None
    expected_date = next(c.receipt_date for c in candidates if c.lot_id == expected[0])
    return lot_id in expected, expected_date


def _provider() -> GeminiProvider:
    return GeminiProvider(
        model=os.environ.get("URDWMS_MODEL", "gemini-pro-latest"),
        media_resolution="high",
    )


# --------------------------------------------------------------------------- schemas

class LotIn(BaseModel):
    item_code: str
    receipt_date: str
    manufacture_date: str | None = None
    supplier_lot_code: str | None = None
    qty: int = 1
    # Receiving is where an item first appears in the system, so an unknown code
    # must be creatable here rather than sending the warehouse to a separate
    # master-data screen first. Supply a name and the item is created with it.
    item_name: str | None = None
    spec: str | None = None
    unit: str = "箱"
    shelf_life_days: int | None = None
    safety_stock: int = 0


class ItemIn(BaseModel):
    item_code: str
    name: str
    spec: str | None = None
    unit: str = "箱"
    shelf_life_days: int | None = None
    safety_stock: int = 0


class ScanIn(BaseModel):
    item_code: str
    lot_id: int | None = None
    image_path: str | None = None
    ocr_receipt_date: str | None = None
    ocr_confidence: float | None = None
    ocr_notes: str | None = None
    match_distance: float | None = None
    fields: dict = {}
    detail_pending: bool = False


class OverrideIn(BaseModel):
    reason: str
    actor: str = "demo.supervisor"


# --------------------------------------------------------------------------- master data

@app.get("/api/health")
def health() -> dict:
    return {"ok": True, "at": now()}


@app.get("/api/items")
def items() -> list[dict]:
    with transaction() as conn:
        rows = conn.execute(
            "SELECT i.*, COALESCE(SUM(l.qty_on_hand), 0) AS on_hand,"
            "       COUNT(CASE WHEN l.qty_on_hand > 0 THEN 1 END) AS open_lots"
            " FROM inventory_item i LEFT JOIN inventory_lot l ON l.item_code = i.item_code"
            " GROUP BY i.item_code ORDER BY i.item_code",
        ).fetchall()
    return [dict(r) for r in rows]


@app.post("/api/items")
def create_item(payload: ItemIn) -> dict:
    with transaction() as conn:
        if conn.execute("SELECT 1 FROM inventory_item WHERE item_code = ?", (payload.item_code,)).fetchone():
            raise HTTPException(409, f"料號 {payload.item_code} 已存在")
        conn.execute(
            "INSERT INTO inventory_item (item_code, name, spec, unit, shelf_life_days, safety_stock)"
            " VALUES (?, ?, ?, ?, ?, ?)",
            (payload.item_code.strip(), payload.name.strip(), payload.spec, payload.unit,
             payload.shelf_life_days, payload.safety_stock),
        )
        log(conn, DEMO_USER, "item.create", {"item_code": payload.item_code})
    return {"item_code": payload.item_code}


@app.get("/api/lots")
def lots(item_code: str | None = None) -> list[dict]:
    with transaction() as conn:
        sql = ("SELECT l.*, i.name AS item_name FROM inventory_lot l"
               " JOIN inventory_item i ON i.item_code = l.item_code")
        params: tuple = ()
        if item_code:
            sql += " WHERE l.item_code = ?"
            params = (item_code,)
        sql += " ORDER BY l.item_code, l.receipt_date"
        rows = [dict(r) for r in conn.execute(sql, params).fetchall()]

        by_item: dict[str, list[Candidate]] = {}
        for row in rows:
            by_item.setdefault(row["item_code"], [])
        for code in by_item:
            by_item[code] = _candidates(conn, code)

    for row in rows:
        expected = fifo_expected(by_item.get(row["item_code"], []))
        row["is_fifo_next"] = str(row["id"]) in expected
    return rows


@app.post("/api/lots")
def create_lot(payload: LotIn) -> dict:
    """US-1 receiving. A future receipt date is almost always a typo, so it is refused."""
    parsed = to_date_key(payload.receipt_date)
    if parsed is None or parsed.iso is None:
        raise HTTPException(400, "進貨日格式無法辨識")
    if parsed.iso > date.today().isoformat():
        raise HTTPException(400, f"進貨日 {parsed.iso} 在未來，請確認是否打錯")
    if payload.qty < 1:
        raise HTTPException(400, "數量至少 1 箱")

    manufacture = to_date_key(payload.manufacture_date) if payload.manufacture_date else None
    item_code = payload.item_code.strip()
    if not item_code:
        raise HTTPException(400, "料號不可空白")

    created_item = False
    with transaction() as conn:
        known = conn.execute("SELECT 1 FROM inventory_item WHERE item_code = ?", (item_code,)).fetchone()
        if not known:
            if not (payload.item_name or "").strip():
                raise HTTPException(400, f"料號 {item_code} 不在主檔，請一併填品名以新增品項")
            conn.execute(
                "INSERT INTO inventory_item (item_code, name, spec, unit, shelf_life_days, safety_stock)"
                " VALUES (?, ?, ?, ?, ?, ?)",
                (item_code, payload.item_name.strip(), payload.spec, payload.unit,
                 payload.shelf_life_days, payload.safety_stock),
            )
            created_item = True
            log(conn, DEMO_USER, "item.create", {"item_code": item_code, "name": payload.item_name})
        elif payload.item_name or payload.shelf_life_days is not None or payload.safety_stock:
            # Receiving is also when someone notices the master data is wrong.
            # Let them fix it in place, but never silently — it hits audit_log.
            updates, params = [], []
            for column, value in (("name", (payload.item_name or "").strip() or None),
                                  ("spec", payload.spec),
                                  ("shelf_life_days", payload.shelf_life_days),
                                  ("safety_stock", payload.safety_stock or None)):
                if value is not None:
                    updates.append(f"{column} = ?")
                    params.append(value)
            if updates:
                conn.execute(f"UPDATE inventory_item SET {', '.join(updates)} WHERE item_code = ?",
                             (*params, item_code))
                log(conn, DEMO_USER, "item.update", {"item_code": item_code, "changed": updates})
        existing = conn.execute(
            "SELECT id, qty_on_hand FROM inventory_lot WHERE item_code = ? AND receipt_date = ?",
            (item_code, parsed.iso),
        ).fetchone()
        cursor = conn.execute(
            "INSERT INTO inventory_lot (item_code, receipt_date, manufacture_date, supplier_lot_code,"
            " qty_on_hand, created_at, created_by) VALUES (?, ?, ?, ?, ?, ?, ?)",
            (item_code, parsed.iso, manufacture.iso if manufacture else None,
             payload.supplier_lot_code, payload.qty, now(), DEMO_USER),
        )
        lot_id = cursor.lastrowid
        log(conn, DEMO_USER, "lot.create",
            {"lot_id": lot_id, "item_code": item_code, "receipt_date": parsed.iso, "qty": payload.qty})
    return {
        "id": lot_id,
        "receipt_date": parsed.iso,
        "created_item": created_item,
        # Surfaced rather than auto-merged: whether two deliveries on one day are
        # one lot or two is the warehouse's call, not the system's (US-1).
        "same_day_lot_exists": bool(existing),
    }


# --------------------------------------------------------------------------- recognition

@app.post("/api/recognize")
async def recognize(item_code: str = Form(...), image: UploadFile = File(...)) -> dict:
    """Recognise a box, then match it against what is actually in stock.

    Returns a proposal only — nothing is written and no stock moves. The operator
    confirms on screen, because "this is not the box I picked up" has no other
    line of defence (M7 architecture, constraint 4).
    """
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    suffix = Path(image.filename or "capture.jpg").suffix or ".jpg"
    path = UPLOADS / f"{stamp}{suffix}"
    path.write_bytes(await image.read())

    try:
        reading: Recognition = _provider().recognize(path)
    except RuntimeError as exc:
        reading = Recognition(error=str(exc))

    with transaction() as conn:
        candidates = _candidates(conn, item_code)
        item = conn.execute("SELECT * FROM inventory_item WHERE item_code = ?", (item_code,)).fetchone()
        lot_rows = {str(r["id"]): dict(r) for r in conn.execute(
            "SELECT * FROM inventory_lot WHERE item_code = ? AND qty_on_hand > 0", (item_code,)).fetchall()}

    result = match_candidates(to_date_key(reading.receipt_date), candidates)
    payload: dict = {
        "image_path": f"/uploads/{path.name}",
        "item_code": item_code,
        "item_name": item["name"] if item else None,
        "recognition": {
            "receipt_date": reading.receipt_date,
            "manufacture_date": reading.manufacture_date,
            "item_code": reading.item_code,
            "confidence": reading.receipt_date_confidence,
            "stamp_visible": reading.stamp_visible,
            "notes": reading.notes,
            "error": reading.error,
        },
        "candidates": [
            {"lot_id": int(c.lot_id), "receipt_date": c.receipt_date,
             "manufacture_date": lot_rows.get(c.lot_id, {}).get("manufacture_date"),
             "qty_on_hand": lot_rows.get(c.lot_id, {}).get("qty_on_hand")}
            for c in candidates
        ],
        "decision": result.decision.value,
        "defer_reason": result.reason.value if result.reason else None,
        "match_distance": result.best_distance,
    }

    if not result.locked:
        # Not a failure — the常駐 fallback (US-4). The operator picks from the
        # candidate list and the draw still goes through the FIFO check.
        payload["locked_lot"] = None
        payload["fifo_ok"] = None
        payload["fifo_expected_date"] = None
        return payload

    ok, expected_date = _fifo_verdict(candidates, result.lot_id or "")
    locked = lot_rows.get(result.lot_id or "", {})
    payload["locked_lot"] = {
        "lot_id": int(result.lot_id), "receipt_date": locked.get("receipt_date"),
        "manufacture_date": locked.get("manufacture_date"), "qty_on_hand": locked.get("qty_on_hand"),
    }
    payload["fifo_ok"] = ok
    payload["fifo_expected_date"] = expected_date
    return payload


# --------------------------------------------------------------------------- issuing

@app.post("/api/scans")
def create_scan(payload: ScanIn) -> dict:
    """Record the draw. Always.

    A FIFO violation writes a complete record and moves no stock. It is not an
    error path — it is the deliverable (requirement section 2.1). Which is why
    this endpoint returns 200 with status=blocked_fifo rather than a 4xx: the
    caller did not do anything wrong, and the record exists either way.
    """
    with transaction() as conn:
        candidates = _candidates(conn, payload.item_code)

        if payload.lot_id is None:
            scan_id = _insert_scan(conn, payload, status="blocked_unreadable", lot_id=None,
                                   fifo_expected=(None, None))
            log(conn, DEMO_USER, "scan.blocked_unreadable", {"scan_id": scan_id})
            return {"id": scan_id, "status": "blocked_unreadable"}

        lot = conn.execute("SELECT * FROM inventory_lot WHERE id = ?", (payload.lot_id,)).fetchone()
        if lot is None:
            raise HTTPException(404, "批次不存在")
        if lot["qty_on_hand"] < 1:
            raise HTTPException(409, "該批次已無庫存")

        ok, expected_date = _fifo_verdict(candidates, str(payload.lot_id))
        expected_ids = fifo_expected(candidates)
        expected_id = int(expected_ids[0]) if expected_ids else None

        if not ok:
            scan_id = _insert_scan(conn, payload, status="blocked_fifo", lot_id=payload.lot_id,
                                   fifo_expected=(expected_id, expected_date))
            log(conn, DEMO_USER, "scan.blocked_fifo",
                {"scan_id": scan_id, "took_lot": payload.lot_id, "expected_lot": expected_id})
            return {"id": scan_id, "status": "blocked_fifo",
                    "fifo_expected_lot_id": expected_id, "fifo_expected_date": expected_date}

        # Stock movement and record, one transaction (US-2).
        scan_id = _insert_scan(conn, payload, status="posted", lot_id=payload.lot_id,
                               fifo_expected=(expected_id, expected_date))
        conn.execute("UPDATE inventory_lot SET qty_on_hand = qty_on_hand - 1 WHERE id = ? AND qty_on_hand > 0",
                     (payload.lot_id,))
        log(conn, DEMO_USER, "scan.posted", {"scan_id": scan_id, "lot_id": payload.lot_id})
        return {"id": scan_id, "status": "posted"}


def _insert_scan(conn, payload: ScanIn, *, status: str, lot_id: int | None,
                 fifo_expected: tuple[int | None, str | None]) -> int:
    cursor = conn.execute(
        "INSERT INTO material_usage_scan (item_code, lot_id, status, captured_at, captured_by,"
        " image_path, ocr_receipt_date, ocr_confidence, ocr_notes, match_distance,"
        " fifo_expected_lot_id, fifo_expected_date, field_values, detail_pending, created_at)"
        " VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        (payload.item_code, lot_id, status, now(), DEMO_USER, payload.image_path,
         payload.ocr_receipt_date, payload.ocr_confidence, payload.ocr_notes, payload.match_distance,
         fifo_expected[0], fifo_expected[1], json.dumps(payload.fields, ensure_ascii=False),
         int(payload.detail_pending), now()),
    )
    return int(cursor.lastrowid)


@app.get("/api/scans")
def scans(status: str | None = None, limit: int = 100) -> list[dict]:
    with transaction() as conn:
        sql = ("SELECT s.*, l.receipt_date, l.manufacture_date, i.name AS item_name"
               " FROM material_usage_scan s"
               " LEFT JOIN inventory_lot l ON l.id = s.lot_id"
               " LEFT JOIN inventory_item i ON i.item_code = s.item_code")
        params: tuple = ()
        if status:
            sql += " WHERE s.status = ?"
            params = (status,)
        sql += " ORDER BY s.id DESC LIMIT ?"
        rows = conn.execute(sql, (*params, limit)).fetchall()
    return [{**dict(r), "field_values": json.loads(r["field_values"] or "{}")} for r in rows]


@app.post("/api/scans/{scan_id}/override")
def override(scan_id: int, payload: OverrideIn) -> dict:
    """US-5. Without this the floor learns to stop photographing boxes entirely."""
    if not payload.reason.strip():
        raise HTTPException(400, "覆核必須填原因")
    with transaction() as conn:
        row = conn.execute("SELECT * FROM material_usage_scan WHERE id = ?", (scan_id,)).fetchone()
        if row is None:
            raise HTTPException(404, "紀錄不存在")
        if row["status"] != "blocked_fifo":
            raise HTTPException(409, f"只有 blocked_fifo 可覆核，目前為 {row['status']}")
        conn.execute(
            "UPDATE material_usage_scan SET status = 'overridden', override_by = ?, override_reason = ?"
            " WHERE id = ?", (payload.actor, payload.reason, scan_id))
        conn.execute("UPDATE inventory_lot SET qty_on_hand = qty_on_hand - 1"
                     " WHERE id = ? AND qty_on_hand > 0", (row["lot_id"],))
        log(conn, payload.actor, "scan.overridden", {"scan_id": scan_id, "reason": payload.reason})
    return {"id": scan_id, "status": "overridden"}


# --------------------------------------------------------------------------- traceability & alerts

@app.get("/api/trace")
def trace(lot_id: int | None = None, packed_product: str | None = None) -> dict:
    """US-7. Blocked records appear here too — the film may physically have been used."""
    with transaction() as conn:
        sql = ("SELECT s.*, l.receipt_date, l.manufacture_date FROM material_usage_scan s"
               " LEFT JOIN inventory_lot l ON l.id = s.lot_id WHERE 1=1")
        params: list = []
        if lot_id:
            sql += " AND s.lot_id = ?"
            params.append(lot_id)
        rows = [dict(r) for r in conn.execute(sql + " ORDER BY s.id DESC", params).fetchall()]

    for row in rows:
        row["field_values"] = json.loads(row["field_values"] or "{}")
    if packed_product:
        rows = [r for r in rows if r["field_values"].get("包裝產品") == packed_product]
    return {"count": len(rows), "records": rows}


@app.get("/api/alerts")
def alerts() -> dict:
    """US-8 plus the pending-detail reminder from US-6.

    An empty result still reports a timestamp: a blank panel cannot distinguish
    "nothing is wrong" from "the check stopped running" (M7 architecture,
    constraint 2).
    """
    today = date.today()
    stale_days, expiry_days, pending_hours = 120, 60, 24
    out: dict[str, list] = {"expiring": [], "stale": [], "low_stock": [], "pending_detail": []}

    with transaction() as conn:
        for row in conn.execute(
            "SELECT l.*, i.name, i.shelf_life_days FROM inventory_lot l"
            " JOIN inventory_item i ON i.item_code = l.item_code WHERE l.qty_on_hand > 0"
        ).fetchall():
            lot = dict(row)
            if lot["shelf_life_days"] and lot["manufacture_date"]:
                expires = date.fromisoformat(lot["manufacture_date"]) + timedelta(days=lot["shelf_life_days"])
                remaining = (expires - today).days
                if remaining <= expiry_days:
                    out["expiring"].append({**lot, "expires_on": expires.isoformat(), "days_left": remaining})
            age = (today - date.fromisoformat(lot["receipt_date"])).days
            if age >= stale_days:
                out["stale"].append({**lot, "age_days": age})

        for row in conn.execute(
            "SELECT i.item_code, i.name, i.safety_stock, COALESCE(SUM(l.qty_on_hand), 0) AS on_hand"
            " FROM inventory_item i LEFT JOIN inventory_lot l ON l.item_code = i.item_code"
            " WHERE i.safety_stock > 0 GROUP BY i.item_code HAVING on_hand < i.safety_stock"
        ).fetchall():
            out["low_stock"].append(dict(row))

        cutoff = (datetime.now() - timedelta(hours=pending_hours)).isoformat(timespec="seconds")
        for row in conn.execute(
            "SELECT id, item_code, captured_at FROM material_usage_scan"
            " WHERE detail_pending = 1 AND status IN ('posted','overridden') AND captured_at < ?",
            (cutoff,),
        ).fetchall():
            out["pending_detail"].append(dict(row))

    total = sum(len(v) for v in out.values())
    return {
        "checked_at": now(),
        "total": total,
        "thresholds": {"expiry_days": expiry_days, "stale_days": stale_days, "pending_hours": pending_hours},
        **out,
    }
