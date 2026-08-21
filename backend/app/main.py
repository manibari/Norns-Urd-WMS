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

from fastapi import Depends, FastAPI, File, Form, Header, HTTPException, UploadFile
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "core"))

from urdwms_core.matching import (  # noqa: E402
    Candidate, fifo_expected, match_candidates, match_item_code,
)
from urdwms_core.normalize import normalize_item_code, to_date_key  # noqa: E402
from urdwms_core.recognition import GeminiProvider, Recognition  # noqa: E402
from urdwms_core.units import boxes_from_meters, meters_from_boxes  # noqa: E402

from .auth import (  # noqa: E402
    DEFAULT_ROLE_LABELS, PERMISSIONS, check_password_policy, create_session, current_user,
    hash_password, requires, role_label, role_labels, verify_password,
)
from .db import init_db, log, now, transaction  # noqa: E402

UPLOADS = Path(__file__).resolve().parents[1] / "uploads"

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
    # A rejected lot is not a candidate. It stays on the books (the record is the
    # deliverable) but FIFO must never point anyone at it, and nobody may draw it.
    rows = conn.execute(
        "SELECT id, receipt_date FROM inventory_lot"
        " WHERE item_code = ? AND qty_on_hand > 0 AND COALESCE(verdict, '合格') <> '不合格'"
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
    """One line off the acceptance form (包材驗收單).

    Field order on paper: 進貨日期 / 廠商名稱 / 原物料名稱 / 型號 / 數量(米).
    `item_code` is the 型號 — the short code the warehouse actually writes and
    the key of the metres table. The long code printed on the box lives in
    `supplier_code` and exists only so recognition can map back to a 型號.
    """

    item_code: str                      # 型號, e.g. T6050BSW
    receipt_date: str
    supplier: str | None = None         # 廠商名稱
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
    meters_per_box: int | None = None
    supplier_code: str | None = None    # 箱上完整料號, 辨識對映用
    expiry_date: str | None = None      # 標示(有效日期). 多數包材沒有
    entered_unit: str = "米"
    # 檢驗項目: 規格尺寸 / 標示製造日期 / 標示有效日期 / 外觀 / 顏色
    inspection: dict = {}
    verdict: str | None = None          # 合格 | 不合格
    confirmed_by: str | None = None
    remark: str | None = None
    # The acceptance form records quantity in metres, so this is the normal path
    # rather than an option. Converted to whole boxes; boxes remain the ledger
    # unit (units.py).
    qty_meters: int | None = None


class ItemIn(BaseModel):
    item_code: str
    name: str
    spec: str | None = None
    unit: str = "箱"
    shelf_life_days: int | None = None
    safety_stock: int = 0
    meters_per_box: int | None = None
    supplier_code: str | None = None
    supplier: str | None = None


class ItemPatch(BaseModel):
    name: str | None = None
    spec: str | None = None
    shelf_life_days: int | None = None
    safety_stock: int | None = None
    meters_per_box: int | None = None
    supplier_code: str | None = None
    supplier: str | None = None


class LotPatch(BaseModel):
    """Admin correction of a receiving line. Every field here is something a
    person can mistype off the paper form."""

    receipt_date: str | None = None
    manufacture_date: str | None = None
    expiry_date: str | None = None
    supplier: str | None = None
    supplier_lot_code: str | None = None
    qty_on_hand: int | None = None
    verdict: str | None = None
    remark: str | None = None


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


# --------------------------------------------------------------------------- auth

class LoginIn(BaseModel):
    username: str
    password: str


class UserIn(BaseModel):
    username: str
    name: str
    role: str                    # 權限層級 user | manager | admin
    title: str | None = None     # 職位, 顯示用
    password: str


class UserPatch(BaseModel):
    name: str | None = None
    role: str | None = None
    title: str | None = None
    password: str | None = None
    active: bool | None = None


class PasswordChangeIn(BaseModel):
    current_password: str
    new_password: str


@app.get("/api/auth/signers")
def signers(user: dict = Depends(current_user)) -> list[dict]:
    """Names that can appear as 確認人 — requires a session.

    Not public: an unauthenticated list of valid accounts is a list of things to
    guess passwords against.
    """
    with transaction() as conn:
        rows = conn.execute(
            "SELECT name, title, role FROM app_user WHERE active = 1 ORDER BY role, name").fetchall()
    # Shows the job title, which is what people recognise each other by; the
    # role is a permission tier and means nothing to whoever is signing.
    return [{"name": r["name"], "title": r["title"], "role_label": role_label(r["role"])}
            for r in rows]


@app.post("/api/auth/login")
def login(payload: LoginIn) -> dict:
    with transaction() as conn:
        row = conn.execute("SELECT * FROM app_user WHERE username = ? AND active = 1",
                           (payload.username.strip().lower(),)).fetchone()
    # One message for both failures. Distinguishing "no such account" from "wrong
    # password" tells an attacker which usernames are worth attacking.
    if row is None or not verify_password(payload.password, row["password_hash"]):
        raise HTTPException(401, "帳號或密碼不正確")
    token, expires = create_session(row["id"])
    with transaction() as conn:
        log(conn, row["name"], "auth.login", {"role": row["role"], "username": row["username"]})
    return {
        "token": token, "expires_at": expires,
        "user": {"id": row["id"], "username": row["username"], "name": row["name"],
                 "title": row["title"], "role": row["role"], "role_label": role_label(row["role"]),
                 "must_change": bool(row["must_change"]),
                 "permissions": sorted(PERMISSIONS.get(row["role"], set()))},
    }


@app.post("/api/auth/password")
def change_password(payload: PasswordChangeIn, user: dict = Depends(current_user)) -> dict:
    """Change your own password. Requires the current one — otherwise a borrowed
    session becomes a permanent takeover."""
    check_password_policy(payload.new_password)
    with transaction() as conn:
        row = conn.execute("SELECT password_hash FROM app_user WHERE id = ?", (user["id"],)).fetchone()
        if not verify_password(payload.current_password, row["password_hash"]):
            raise HTTPException(401, "目前密碼不正確")
        conn.execute("UPDATE app_user SET password_hash = ?, must_change = 0 WHERE id = ?",
                     (hash_password(payload.new_password), user["id"]))
        # Every other session for this account dies: changing a password is what
        # someone does when they think it is known, so leaving old sessions alive
        # defeats the point.
        conn.execute("DELETE FROM app_session WHERE user_id = ?", (user["id"],))
        log(conn, user["name"], "auth.password_change", {"user_id": user["id"]})
    return {"ok": True, "reauth_required": True}


@app.post("/api/auth/logout")
def logout(authorization: str | None = Header(default=None)) -> dict:
    token = (authorization or "").removeprefix("Bearer ").strip()
    with transaction() as conn:
        conn.execute("DELETE FROM app_session WHERE token = ?", (token,))
    return {"ok": True}


@app.get("/api/auth/me")
def me(user: dict = Depends(current_user)) -> dict:
    return {**user, "role_label": role_label(user["role"])}


class RolePatch(BaseModel):
    label: str


@app.get("/api/roles")
def roles(user: dict = Depends(current_user)) -> list[dict]:
    labels = role_labels()
    order = list(DEFAULT_ROLE_LABELS)
    return [
        {"code": code, "label": labels[code], "default_label": DEFAULT_ROLE_LABELS[code],
         "permissions": sorted(PERMISSIONS.get(code, set()))}
        for code in order
    ]


@app.patch("/api/roles/{code}")
def update_role(code: str, payload: RolePatch,
                user: dict = Depends(requires("user.manage"))) -> dict:
    """Rename a role. The permissions behind it are not editable here — what a
    role may do is system design; what it is called is the factory's words."""
    if code not in PERMISSIONS:
        raise HTTPException(404, f"未知的角色 {code}")
    label = payload.label.strip()
    if not label:
        raise HTTPException(400, "名稱不可空白")
    with transaction() as conn:
        conn.execute(
            "INSERT INTO app_role (code, label) VALUES (?, ?)"
            " ON CONFLICT(code) DO UPDATE SET label = excluded.label", (code, label))
        log(conn, user["name"], "role.rename", {"code": code, "label": label})
    return {"code": code, "label": label}


@app.get("/api/users")
def users(user: dict = Depends(requires("user.manage"))) -> list[dict]:
    with transaction() as conn:
        rows = conn.execute("SELECT id, username, name, title, role, active, must_change, created_at"
                            " FROM app_user ORDER BY role, name").fetchall()
    return [{**dict(r), "role_label": role_label(r["role"])} for r in rows]


@app.post("/api/users")
def create_user(payload: UserIn, user: dict = Depends(requires("user.manage"))) -> dict:
    if payload.role not in PERMISSIONS:
        raise HTTPException(400, f"未知的角色 {payload.role}")
    check_password_policy(payload.password)
    username = payload.username.strip().lower()
    if not username:
        raise HTTPException(400, "帳號不可空白")
    with transaction() as conn:
        if conn.execute("SELECT 1 FROM app_user WHERE username = ?", (username,)).fetchone():
            raise HTTPException(409, f"帳號「{username}」已存在")
        cursor = conn.execute(
            "INSERT INTO app_user (username, name, title, role, password_hash, must_change, created_at)"
            " VALUES (?,?,?,?,?,1,?)",
            (username, payload.name.strip(), (payload.title or "").strip() or None,
             payload.role, hash_password(payload.password), now()))
        # Created with a password someone else chose, so it must be changed on
        # first login — otherwise the admin knows everyone's password forever.
        log(conn, user["name"], "user.create", {"username": username, "role": payload.role})
    return {"id": cursor.lastrowid, "username": username, "name": payload.name, "role": payload.role}


@app.patch("/api/users/{user_id}")
def update_user(user_id: int, payload: UserPatch,
                user: dict = Depends(requires("user.manage"))) -> dict:
    changes: dict = {}
    if payload.role is not None:
        if payload.role not in PERMISSIONS:
            raise HTTPException(400, f"未知的角色 {payload.role}")
        changes["role"] = payload.role
    if payload.name is not None and payload.name.strip():
        changes["name"] = payload.name.strip()
    if payload.title is not None:
        changes["title"] = payload.title.strip() or None
    if payload.password is not None:
        check_password_policy(payload.password)
        changes["password_hash"] = hash_password(payload.password)
        changes["must_change"] = 1
    if payload.active is not None:
        changes["active"] = int(payload.active)
    if not changes:
        raise HTTPException(400, "沒有要更新的欄位")
    with transaction() as conn:
        row = conn.execute("SELECT * FROM app_user WHERE id = ?", (user_id,)).fetchone()
        if row is None:
            raise HTTPException(404, "使用者不存在")
        # Locking out the last admin leaves nobody able to unlock anyone.
        if row["role"] == "admin" and (changes.get("role", "admin") != "admin" or changes.get("active") == 0):
            remaining = conn.execute(
                "SELECT COUNT(*) AS n FROM app_user WHERE role = 'admin' AND active = 1 AND id <> ?",
                (user_id,)).fetchone()["n"]
            if remaining == 0:
                raise HTTPException(409, "這是最後一個啟用中的管理者，不能停用或降級")
        conn.execute(f"UPDATE app_user SET {', '.join(f'{k} = ?' for k in changes)} WHERE id = ?",
                     (*changes.values(), user_id))
        # Changing a role or disabling an account must take effect now, not in
        # twelve hours when the session happens to expire.
        if "role" in changes or changes.get("active") == 0 or "password_hash" in changes:
            conn.execute("DELETE FROM app_session WHERE user_id = ?", (user_id,))
        log(conn, user["name"], "user.update",
            {"user_id": user_id,
             "changed": {k: ("***" if k == "password_hash" else v) for k, v in changes.items()}})
    return {"id": user_id, "changed": list(changes)}


# --------------------------------------------------------------------------- dictionary

# Dropdowns that are NOT attributes of something else.
#
# 廠商 / 原物料名稱 / 規格 deliberately are not here: a 型號 already carries all
# three, so a separate table would hold the same facts twice and let them drift
# apart. Those dropdowns read distinct values off the item master instead.
#
# 人員 is likewise gone — signers are accounts now, not free-standing names.
#
# Retiring a value deactivates it rather than deleting it, because historical
# records reference it, and a traceability report rendering a blank where a
# machine used to be is worse than one naming a machine that no longer exists.
DICTIONARY_CATEGORIES: dict[str, str] = {
    "job_title": "職位名稱",
    "machine": "包裝機台",
    "packed_product": "產品名稱",
    "override_reason": "非 FIFO 覆核原因",
}


class DictionaryIn(BaseModel):
    category: str
    value: str
    sort_order: int = 0


class DictionaryPatch(BaseModel):
    value: str | None = None
    sort_order: int | None = None
    active: bool | None = None


@app.get("/api/dictionary")
def dictionary(category: str | None = None, include_inactive: bool = False,
               user: dict = Depends(current_user)) -> dict:
    sql = "SELECT * FROM dictionary WHERE 1=1"
    params: list = []
    if category:
        sql += " AND category = ?"
        params.append(category)
    if not include_inactive:
        sql += " AND active = 1"
    sql += " ORDER BY category, sort_order, value"
    with transaction() as conn:
        rows = [dict(r) for r in conn.execute(sql, params).fetchall()]
    grouped: dict[str, list] = {key: [] for key in DICTIONARY_CATEGORIES}
    for row in rows:
        grouped.setdefault(row["category"], []).append(row)
    return {"categories": DICTIONARY_CATEGORIES, "entries": grouped}


@app.post("/api/dictionary")
def create_dictionary_entry(payload: DictionaryIn,
                            user: dict = Depends(requires("dictionary.manage"))) -> dict:
    if payload.category not in DICTIONARY_CATEGORIES:
        raise HTTPException(400, f"未知的字典類別 {payload.category}")
    value = payload.value.strip()
    if not value:
        raise HTTPException(400, "值不可空白")
    with transaction() as conn:
        existing = conn.execute(
            "SELECT id, active FROM dictionary WHERE category = ? AND value = ?",
            (payload.category, value)).fetchone()
        if existing:
            if existing["active"]:
                raise HTTPException(409, f"「{value}」已存在")
            # Re-adding a retired value revives the original row so historical
            # records keep pointing at the same entry.
            conn.execute("UPDATE dictionary SET active = 1 WHERE id = ?", (existing["id"],))
            log(conn, user["name"], "dictionary.revive", {"id": existing["id"], "value": value})
            return {"id": existing["id"], "value": value, "revived": True}
        cursor = conn.execute(
            "INSERT INTO dictionary (category, value, sort_order, created_at) VALUES (?,?,?,?)",
            (payload.category, value, payload.sort_order, now()))
        log(conn, user["name"], "dictionary.create", {"category": payload.category, "value": value})
    return {"id": cursor.lastrowid, "value": value, "revived": False}


@app.patch("/api/dictionary/{entry_id}")
def update_dictionary_entry(entry_id: int, payload: DictionaryPatch,
                            user: dict = Depends(requires("dictionary.manage"))) -> dict:
    changes = {k: v for k, v in payload.model_dump().items() if v is not None}
    if not changes:
        raise HTTPException(400, "沒有要更新的欄位")
    if "value" in changes:
        changes["value"] = str(changes["value"]).strip()
        if not changes["value"]:
            raise HTTPException(400, "值不可空白")
    if "active" in changes:
        changes["active"] = int(bool(changes["active"]))
    with transaction() as conn:
        if not conn.execute("SELECT 1 FROM dictionary WHERE id = ?", (entry_id,)).fetchone():
            raise HTTPException(404, "字典項目不存在")
        conn.execute(f"UPDATE dictionary SET {', '.join(f'{k} = ?' for k in changes)} WHERE id = ?",
                     (*changes.values(), entry_id))
        log(conn, user["name"], "dictionary.update", {"id": entry_id, **changes})
    return {"id": entry_id, **changes}


# --------------------------------------------------------------------------- master data

@app.get("/api/health")
def health() -> dict:
    return {"ok": True, "at": now()}


@app.get("/api/items")
def items(user: dict = Depends(current_user)) -> list[dict]:
    with transaction() as conn:
        rows = conn.execute(
            # "在庫" means drawable. A rejected lot is physically present but must
            # never be counted as available, or FIFO and the low-stock alert both
            # end up reasoning about stock nobody is allowed to touch.
            "SELECT i.*,"
            " COALESCE(SUM(CASE WHEN COALESCE(l.verdict,'合格') <> '不合格' THEN l.qty_on_hand ELSE 0 END), 0)"
            "   AS on_hand,"
            " COUNT(CASE WHEN l.qty_on_hand > 0 AND COALESCE(l.verdict,'合格') <> '不合格' THEN 1 END)"
            "   AS open_lots,"
            " COALESCE(SUM(CASE WHEN l.verdict = '不合格' THEN l.qty_on_hand ELSE 0 END), 0) AS rejected_qty"
            " FROM inventory_item i LEFT JOIN inventory_lot l ON l.item_code = i.item_code"
            " GROUP BY i.item_code ORDER BY i.item_code",
        ).fetchall()
    return [{**dict(r), "on_hand_m": meters_from_boxes(r["on_hand"], r["meters_per_box"])} for r in rows]


@app.get("/api/item-options")
def item_options(user: dict = Depends(current_user)) -> dict:
    """Distinct 廠商 / 原物料名稱 / 規格 already in use, for the new-型號 form.

    Derived rather than stored: these are attributes of a 型號, so the item
    master is the only place they live. Keeping a parallel list would be two
    copies of the same fact, free to disagree.
    """
    with transaction() as conn:
        rows = conn.execute(
            "SELECT DISTINCT supplier, name, spec FROM inventory_item").fetchall()
    def distinct(key: str) -> list[str]:
        return sorted({r[key] for r in rows if r[key]})
    return {
        "supplier": distinct("supplier"),
        "material_name": distinct("name"),
        "spec": distinct("spec"),
    }


@app.patch("/api/items/{item_code:path}")
def update_item(item_code: str, payload: ItemPatch,
                user: dict = Depends(requires("item.manage"))) -> dict:
    """Edit the master record, including the metres-per-box rate."""
    changes = {k: v for k, v in payload.model_dump().items() if v is not None}
    if not changes:
        raise HTTPException(400, "沒有要更新的欄位")
    if "meters_per_box" in changes and changes["meters_per_box"] <= 0:
        raise HTTPException(400, "每箱米數必須大於 0（不用米數換算請留空）")
    with transaction() as conn:
        if not conn.execute("SELECT 1 FROM inventory_item WHERE item_code = ?", (item_code,)).fetchone():
            raise HTTPException(404, f"料號 {item_code} 不存在")
        conn.execute(f"UPDATE inventory_item SET {', '.join(f'{k} = ?' for k in changes)}"
                     " WHERE item_code = ?", (*changes.values(), item_code))
        log(conn, user["name"], "item.update", {"item_code": item_code, **changes})
    return {"item_code": item_code, **changes}


@app.post("/api/items")
def create_item(payload: ItemIn, user: dict = Depends(requires("item.manage"))) -> dict:
    with transaction() as conn:
        if conn.execute("SELECT 1 FROM inventory_item WHERE item_code = ?", (payload.item_code,)).fetchone():
            raise HTTPException(409, f"料號 {payload.item_code} 已存在")
        conn.execute(
            "INSERT INTO inventory_item (item_code, name, spec, unit, shelf_life_days, safety_stock,"
            " meters_per_box, supplier_code, supplier) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (payload.item_code.strip(), payload.name.strip(), payload.spec, payload.unit,
             payload.shelf_life_days, payload.safety_stock, payload.meters_per_box,
             payload.supplier_code, payload.supplier),
        )
        log(conn, user["name"], "item.create", {"item_code": payload.item_code})
    return {"item_code": payload.item_code}


@app.delete("/api/items/{item_code:path}")
def delete_item(item_code: str, user: dict = Depends(requires("item.manage"))) -> dict:
    """Delete a 型號 — only while no lot references it.

    Same boundary as deleting a lot: the master record is what a usage record's
    item_code resolves to, so removing one with history behind it turns every
    traceability answer about it into a dangling code.
    """
    with transaction() as conn:
        row = conn.execute("SELECT * FROM inventory_item WHERE item_code = ?", (item_code,)).fetchone()
        if row is None:
            raise HTTPException(404, f"型號 {item_code} 不存在")
        lots = conn.execute("SELECT COUNT(*) AS n FROM inventory_lot WHERE item_code = ?",
                            (item_code,)).fetchone()["n"]
        scans = conn.execute("SELECT COUNT(*) AS n FROM material_usage_scan WHERE item_code = ?",
                             (item_code,)).fetchone()["n"]
        if lots or scans:
            raise HTTPException(
                409,
                f"型號 {item_code} 已有 {lots} 批進貨、{scans} 筆領用紀錄，刪掉會讓那些紀錄失去對應。"
                " 不再使用請把安全水位設 0 並停止收貨。",
            )
        conn.execute("DELETE FROM inventory_item WHERE item_code = ?", (item_code,))
        log(conn, user["name"], "item.delete", {"item_code": item_code, "name": row["name"]})
    return {"item_code": item_code, "deleted": True}


@app.get("/api/lots")
def lots(item_code: str | None = None, user: dict = Depends(current_user)) -> list[dict]:
    with transaction() as conn:
        sql = ("SELECT l.*, i.name AS item_name FROM inventory_lot l"
               " JOIN inventory_item i ON i.item_code = l.item_code")
        params: tuple = ()
        if item_code:
            sql += " WHERE l.item_code = ?"
            params = (item_code,)
        sql += " ORDER BY l.item_code, l.receipt_date"
        rows = [dict(r) for r in conn.execute(sql, params).fetchall()]
        drawn = {r["lot_id"]: r["n"] for r in conn.execute(
            "SELECT lot_id, COUNT(*) AS n FROM material_usage_scan"
            " WHERE lot_id IS NOT NULL GROUP BY lot_id").fetchall()}

        by_item: dict[str, list[Candidate]] = {}
        for row in rows:
            by_item.setdefault(row["item_code"], [])
        for code in by_item:
            by_item[code] = _candidates(conn, code)

    for row in rows:
        expected = fifo_expected(by_item.get(row["item_code"], []))
        row["is_fifo_next"] = str(row["id"]) in expected
        row["inspection"] = json.loads(row.get("inspection") or "{}")
        row["draw_count"] = drawn.get(row["id"], 0)
    return rows


@app.patch("/api/lots/{lot_id}")
def update_lot(lot_id: int, payload: LotPatch,
               user: dict = Depends(requires("lot.edit"))) -> dict:
    """Correct a receiving line.

    Required by US-1 ("事後發現數量登錯可修正，但寫入 audit_log，不可靜默改").
    The audit entry carries before and after values, because "someone changed
    the receipt date" is useless without knowing what it was — and the receipt
    date is the FIFO sort key, so changing it reorders what everyone should be
    drawing next.
    """
    changes: dict = {}
    for field in ("supplier", "supplier_lot_code", "remark", "verdict"):
        value = getattr(payload, field)
        if value is not None:
            changes[field] = value
    for field in ("receipt_date", "manufacture_date", "expiry_date"):
        raw = getattr(payload, field)
        if raw is None:
            continue
        parsed = to_date_key(raw)
        if parsed is None or parsed.iso is None:
            raise HTTPException(400, f"{field} 日期格式無法辨識：{raw}")
        changes[field] = parsed.iso
    if payload.qty_on_hand is not None:
        if payload.qty_on_hand < 0:
            raise HTTPException(400, "在庫數量不可為負")
        changes["qty_on_hand"] = payload.qty_on_hand
    if payload.verdict is not None and payload.verdict not in ("合格", "不合格"):
        raise HTTPException(400, "判定只能是合格或不合格")
    if not changes:
        raise HTTPException(400, "沒有要更新的欄位")
    if changes.get("receipt_date", "") > date.today().isoformat():
        raise HTTPException(400, "進貨日不可在未來")

    with transaction() as conn:
        before = conn.execute("SELECT * FROM inventory_lot WHERE id = ?", (lot_id,)).fetchone()
        if before is None:
            raise HTTPException(404, "批次不存在")
        drawn = conn.execute(
            "SELECT COUNT(*) AS n FROM material_usage_scan WHERE lot_id = ?"
            " AND status IN ('posted','overridden')", (lot_id,)).fetchone()["n"]
        conn.execute(f"UPDATE inventory_lot SET {', '.join(f'{k} = ?' for k in changes)}"
                     " WHERE id = ?", (*changes.values(), lot_id))
        log(conn, user["name"], "lot.update", {
            "lot_id": lot_id,
            "before": {k: before[k] for k in changes},
            "after": changes,
            "posted_draws": drawn,
        })
    return {"id": lot_id, "changed": changes, "posted_draws": drawn}


@app.delete("/api/lots/{lot_id}")
def delete_lot(lot_id: int, user: dict = Depends(requires("lot.delete"))) -> dict:
    """Delete a receiving line — only while nothing has been drawn from it.

    A lot that has been issued from is referenced by usage records. Deleting it
    would leave those records pointing at a lot that no longer exists, and the
    traceability answer ("which film went into this product") is the entire
    reason the system exists. So a used lot is refused, with the count, and the
    caller is pointed at setting the quantity to zero instead — which keeps the
    chain intact and still takes it out of circulation.
    """
    with transaction() as conn:
        row = conn.execute("SELECT * FROM inventory_lot WHERE id = ?", (lot_id,)).fetchone()
        if row is None:
            raise HTTPException(404, "批次不存在")
        drawn = conn.execute(
            "SELECT COUNT(*) AS n FROM material_usage_scan WHERE lot_id = ?", (lot_id,)).fetchone()["n"]
        if drawn:
            raise HTTPException(
                409,
                f"這批已有 {drawn} 筆領用紀錄，刪掉會讓那些紀錄指向不存在的批次，追溯就斷了。"
                " 要讓它退出流通請把在庫數量改成 0。",
            )
        conn.execute("DELETE FROM inventory_lot WHERE id = ?", (lot_id,))
        log(conn, user["name"], "lot.delete", {
            "lot_id": lot_id,
            "deleted": {k: row[k] for k in ("item_code", "receipt_date", "manufacture_date",
                                            "qty_on_hand", "supplier", "verdict")},
        })
    return {"id": lot_id, "deleted": True}


@app.post("/api/lots")
def create_lot(payload: LotIn, user: dict = Depends(requires("lot.create"))) -> dict:
    """US-1 receiving. A future receipt date is almost always a typo, so it is refused."""
    parsed = to_date_key(payload.receipt_date)
    if parsed is None or parsed.iso is None:
        raise HTTPException(400, "進貨日格式無法辨識")
    if parsed.iso > date.today().isoformat():
        raise HTTPException(400, f"進貨日 {parsed.iso} 在未來，請確認是否打錯")
    if payload.qty_meters is None and payload.qty < 1:
        raise HTTPException(400, "數量至少 1 箱")

    manufacture = to_date_key(payload.manufacture_date) if payload.manufacture_date else None
    expiry = to_date_key(payload.expiry_date) if payload.expiry_date else None
    if payload.verdict not in (None, "合格", "不合格"):
        raise HTTPException(400, "判定只能是合格或不合格")
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
                "INSERT INTO inventory_item (item_code, name, spec, unit, shelf_life_days, safety_stock,"
                " meters_per_box, supplier_code, supplier) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (item_code, payload.item_name.strip(), payload.spec, payload.unit,
                 payload.shelf_life_days, payload.safety_stock, payload.meters_per_box,
                 payload.supplier_code, payload.supplier),
            )
            created_item = True
            log(conn, user["name"], "item.create", {"item_code": item_code, "name": payload.item_name})
        elif payload.item_name or payload.shelf_life_days is not None or payload.safety_stock:
            # Receiving is also when someone notices the master data is wrong.
            # Let them fix it in place, but never silently — it hits audit_log.
            updates, params = [], []
            for column, value in (("name", (payload.item_name or "").strip() or None),
                                  ("spec", payload.spec),
                                  ("shelf_life_days", payload.shelf_life_days),
                                  ("meters_per_box", payload.meters_per_box),
                                  ("supplier_code", payload.supplier_code),
                                  ("supplier", payload.supplier),
                                  ("safety_stock", payload.safety_stock or None)):
                if value is not None:
                    updates.append(f"{column} = ?")
                    params.append(value)
            if updates:
                conn.execute(f"UPDATE inventory_item SET {', '.join(updates)} WHERE item_code = ?",
                             (*params, item_code))
                log(conn, user["name"], "item.update", {"item_code": item_code, "changed": updates})
        # One delivery can arrive as several manufacture-date batches, and the form
        # records them as separate lines — so same 型號 + same receipt date is
        # NORMAL, not a duplicate. Only an identical manufacture date makes it
        # look like the same lot entered twice. Warning on the normal case would
        # train people to dismiss the warning, and then the real one goes unread.
        existing = conn.execute(
            "SELECT id, qty_on_hand FROM inventory_lot"
            " WHERE item_code = ? AND receipt_date = ?"
            "   AND COALESCE(manufacture_date, '') = COALESCE(?, '')",
            (item_code, parsed.iso, manufacture.iso if manufacture else None),
        ).fetchone()
        qty = payload.qty
        conversion = None
        if payload.qty_meters is not None:
            # The rate may have been supplied on this very request (new item), so
            # read it back rather than trusting the payload alone.
            rate = conn.execute("SELECT meters_per_box FROM inventory_item WHERE item_code = ?",
                                (item_code,)).fetchone()["meters_per_box"]
            conversion = boxes_from_meters(payload.qty_meters, rate)
            if conversion is None:
                raise HTTPException(400, f"料號 {item_code} 尚未設定每箱米數，無法用米數收貨")
            if conversion.boxes < 1:
                raise HTTPException(
                    400,
                    f"{payload.qty_meters:,} 米不足一箱（每箱 {rate:,} 米）。"
                    " 庫存以箱為單位，不做部分入庫。",
                )
            qty = conversion.boxes

        cursor = conn.execute(
            "INSERT INTO inventory_lot (item_code, receipt_date, manufacture_date, expiry_date,"
            " supplier_lot_code, supplier, entered_meters, entered_unit, inspection, verdict,"
            " recorded_by, confirmed_by, remark, qty_on_hand, created_at, created_by)"
            " VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (item_code, parsed.iso, manufacture.iso if manufacture else None,
             expiry.iso if expiry else None, payload.supplier_lot_code, payload.supplier,
             payload.qty_meters, payload.entered_unit,
             json.dumps(payload.inspection, ensure_ascii=False), payload.verdict,
             # 記錄人 is who is signed in, not a name picked from a list. Dual
             # sign-off is the form's control point; a dropdown of names lets
             # anyone sign as anyone and the control means nothing.
             user["name"], payload.confirmed_by, payload.remark, qty, now(), user["name"]),
        )
        lot_id = cursor.lastrowid
        log(conn, user["name"], "lot.create",
            {"lot_id": lot_id, "item_code": item_code, "receipt_date": parsed.iso, "qty": qty,
             "entered": payload.qty_meters, "verdict": payload.verdict,
             "recorded_by": user["name"], "confirmed_by": payload.confirmed_by})
    return {
        "id": lot_id,
        "receipt_date": parsed.iso,
        "created_item": created_item,
        "qty": qty,
        "verdict": payload.verdict,
        # Dual sign-off exists so one person cannot both book and approve. Same
        # name in both boxes is reported, not blocked — blocking would just get
        # a second name borrowed, and then the record lies (cf. risk R2).
        "recorded_by": user["name"],
        "same_signer": bool(
            payload.confirmed_by and payload.confirmed_by.strip() == user["name"]
        ),
        # Surfaced, never rounded away: metres that do not divide into whole
        # boxes are a real discrepancy the warehouse should see (units.py).
        "conversion_note": conversion.note if conversion else None,
        # Surfaced rather than auto-merged: whether two identical lines are one
        # lot or two is the warehouse's call, not the system's (US-1).
        "duplicate_lot_exists": bool(existing),
    }


# --------------------------------------------------------------------------- recognition

@app.post("/api/recognize")
async def recognize(image: UploadFile = File(...), item_code: str | None = Form(default=None),
                    user: dict = Depends(requires("issue.create"))) -> dict:
    """Recognise a box: which 型號 it is, and which lot.

    Both identifications come from the photo. The operator picks a 型號 only
    when recognition cannot (`item_match.decision == "defer"`), which is the
    same fallback shape as the lot matcher — one rule, one failure mode, one
    thing to explain on the floor.

    Passing `item_code` overrides the label reading; that is the path used when
    the operator has already resolved a deferral.

    Nothing is written and no stock moves. The confirmation step stays because
    "this is not the box I picked up" has no other line of defence
    (M7 architecture, constraint 4).
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
        master = [(r["item_code"], r["supplier_code"]) for r in conn.execute(
            "SELECT item_code, supplier_code FROM inventory_item").fetchall()]
        catalogue = [dict(r) for r in conn.execute(
            "SELECT i.item_code, i.name, i.spec, i.meters_per_box,"
            "       COALESCE(SUM(CASE WHEN COALESCE(l.verdict, '合格') <> '不合格'"
            "                          THEN l.qty_on_hand ELSE 0 END), 0) AS on_hand"
            " FROM inventory_item i LEFT JOIN inventory_lot l ON l.item_code = i.item_code"
            " GROUP BY i.item_code ORDER BY i.item_code").fetchall()]

    item_match = match_item_code(reading.item_code, master)
    resolved = item_code or (item_match.item_code if item_match.locked else None)

    payload: dict = {
        "image_path": f"/uploads/{path.name}",
        "recognition": {
            "receipt_date": reading.receipt_date,
            "manufacture_date": reading.manufacture_date,
            "item_code": reading.item_code,
            "confidence": reading.receipt_date_confidence,
            "item_code_confidence": reading.item_code_confidence,
            "stamp_visible": reading.stamp_visible,
            "notes": reading.notes,
            "error": reading.error,
        },
        "item_match": {
            "decision": "lock" if resolved else "defer",
            "item_code": resolved,
            # Distinguishes "the label said so" from "a human overrode it",
            # which matters when reading the record back months later.
            "matched_on": item_match.matched_on if item_match.locked and not item_code else
                          ("manual" if item_code else None),
            "reason": item_match.reason.value if item_match.reason and not resolved else None,
            "contenders": list(item_match.contenders),
        },
        # Sent along so a deferral can be resolved without a second round trip.
        "catalogue": catalogue,
    }

    if resolved is None:
        payload.update({"item_code": None, "item_name": None, "candidates": [],
                        "decision": "defer", "defer_reason": "item_unresolved",
                        "match_distance": None, "locked_lot": None,
                        "fifo_ok": None, "fifo_expected_date": None})
        return payload

    payload.update(_lot_lookup(resolved, reading.receipt_date))
    return payload


def _lot_lookup(item_code: str, read_receipt_date: str | None) -> dict:
    """Match a read receipt date against the item's drawable lots, and judge FIFO."""
    with transaction() as conn:
        candidates = _candidates(conn, item_code)
        item = conn.execute("SELECT * FROM inventory_item WHERE item_code = ?", (item_code,)).fetchone()
        lot_rows = {str(r["id"]): dict(r) for r in conn.execute(
            "SELECT * FROM inventory_lot WHERE item_code = ? AND qty_on_hand > 0"
            " AND COALESCE(verdict, '合格') <> '不合格'", (item_code,)).fetchall()}

    result = match_candidates(to_date_key(read_receipt_date), candidates)
    out: dict = {
        "item_code": item_code,
        "item_name": item["name"] if item else None,
        "expected_supplier_code": item["supplier_code"] if item else None,
        "candidates": [
            {"lot_id": int(c.lot_id), "receipt_date": c.receipt_date,
             "manufacture_date": lot_rows.get(c.lot_id, {}).get("manufacture_date"),
             "expiry_date": lot_rows.get(c.lot_id, {}).get("expiry_date"),
             "qty_on_hand": lot_rows.get(c.lot_id, {}).get("qty_on_hand")}
            for c in candidates
        ],
        "decision": result.decision.value,
        "defer_reason": result.reason.value if result.reason else None,
        "match_distance": result.best_distance,
    }
    if not result.locked:
        return {**out, "locked_lot": None, "fifo_ok": None, "fifo_expected_date": None}

    ok, expected_date = _fifo_verdict(candidates, result.lot_id or "")
    locked = lot_rows.get(result.lot_id or "", {})
    return {
        **out,
        "locked_lot": {
            "lot_id": int(result.lot_id), "receipt_date": locked.get("receipt_date"),
            "manufacture_date": locked.get("manufacture_date"),
            "qty_on_hand": locked.get("qty_on_hand"),
        },
        "fifo_ok": ok,
        "fifo_expected_date": expected_date,
    }


class ResolveIn(BaseModel):
    item_code: str
    ocr_receipt_date: str | None = None


@app.post("/api/resolve-item")
def resolve_item(payload: ResolveIn, user: dict = Depends(requires("issue.create"))) -> dict:
    """Re-run the lot match after a human picks the 型號 recognition could not.

    Deliberately does not touch the image: recognition already ran and is
    billed. Re-uploading to change one field would cost a second call and could
    return a *different* reading, which would be confusing on screen.
    """
    return {
        **_lot_lookup(payload.item_code, payload.ocr_receipt_date),
        "item_match": {"decision": "lock", "item_code": payload.item_code,
                       "matched_on": "manual", "reason": None, "contenders": []},
    }


# --------------------------------------------------------------------------- issuing

@app.post("/api/scans")
def create_scan(payload: ScanIn, user: dict = Depends(requires("issue.create"))) -> dict:
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
                                   fifo_expected=(None, None), user=user)
            log(conn, user["name"], "scan.blocked_unreadable", {"scan_id": scan_id})
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
                                   fifo_expected=(expected_id, expected_date), user=user)
            log(conn, user["name"], "scan.blocked_fifo",
                {"scan_id": scan_id, "took_lot": payload.lot_id, "expected_lot": expected_id})
            return {"id": scan_id, "status": "blocked_fifo",
                    "fifo_expected_lot_id": expected_id, "fifo_expected_date": expected_date}

        # Stock movement and record, one transaction (US-2).
        scan_id = _insert_scan(conn, payload, status="posted", lot_id=payload.lot_id,
                               fifo_expected=(expected_id, expected_date), user=user)
        conn.execute("UPDATE inventory_lot SET qty_on_hand = qty_on_hand - 1 WHERE id = ? AND qty_on_hand > 0",
                     (payload.lot_id,))
        log(conn, user["name"], "scan.posted", {"scan_id": scan_id, "lot_id": payload.lot_id})
        return {"id": scan_id, "status": "posted"}


def _insert_scan(conn, payload: ScanIn, *, status: str, lot_id: int | None,
                 fifo_expected: tuple[int | None, str | None], user: dict) -> int:
    cursor = conn.execute(
        "INSERT INTO material_usage_scan (item_code, lot_id, status, captured_at, captured_by,"
        " image_path, ocr_receipt_date, ocr_confidence, ocr_notes, match_distance,"
        " fifo_expected_lot_id, fifo_expected_date, field_values, detail_pending, created_at)"
        " VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        (payload.item_code, lot_id, status, now(), user["name"], payload.image_path,
         payload.ocr_receipt_date, payload.ocr_confidence, payload.ocr_notes, payload.match_distance,
         fifo_expected[0], fifo_expected[1], json.dumps(payload.fields, ensure_ascii=False),
         int(payload.detail_pending), now()),
    )
    return int(cursor.lastrowid)


@app.get("/api/scans")
def scans(status: str | None = None, limit: int = 100,
          user: dict = Depends(current_user)) -> list[dict]:
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
def override(scan_id: int, payload: OverrideIn,
             user: dict = Depends(requires("scan.override"))) -> dict:
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
            " WHERE id = ?", (user["name"], payload.reason, scan_id))
        conn.execute("UPDATE inventory_lot SET qty_on_hand = qty_on_hand - 1"
                     " WHERE id = ? AND qty_on_hand > 0", (row["lot_id"],))
        log(conn, payload.actor, "scan.overridden", {"scan_id": scan_id, "reason": payload.reason})
    return {"id": scan_id, "status": "overridden"}


@app.get("/api/audit")
def audit(limit: int = 200, action: str | None = None,
          user: dict = Depends(requires("audit.read"))) -> list[dict]:
    """The change trail.

    US-1 requires corrections to be recorded rather than silent. A trail nobody
    can read is silent in every way that matters, so it gets a screen.
    """
    sql = "SELECT * FROM audit_log"
    params: list = []
    if action:
        sql += " WHERE action LIKE ?"
        params.append(f"{action}%")
    sql += " ORDER BY id DESC LIMIT ?"
    params.append(limit)
    with transaction() as conn:
        rows = conn.execute(sql, params).fetchall()
    return [{**dict(r), "detail": json.loads(r["detail"] or "{}")} for r in rows]


# --------------------------------------------------------------------------- traceability & alerts

@app.get("/api/trace")
def trace(lot_id: int | None = None, packed_product: str | None = None,
          user: dict = Depends(current_user)) -> dict:
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
def alerts(user: dict = Depends(current_user)) -> dict:
    """US-8 plus the pending-detail reminder from US-6.

    An empty result still reports a timestamp: a blank panel cannot distinguish
    "nothing is wrong" from "the check stopped running" (M7 architecture,
    constraint 2).
    """
    today = date.today()
    stale_days, expiry_days, pending_hours = 120, 60, 24
    out: dict[str, list] = {"expiring": [], "stale": [], "low_stock": [], "pending_detail": [],
                            "rejected": []}

    with transaction() as conn:
        for row in conn.execute(
            "SELECT l.*, i.name, i.shelf_life_days FROM inventory_lot l"
            " JOIN inventory_item i ON i.item_code = l.item_code"
            " WHERE l.qty_on_hand > 0 AND COALESCE(l.verdict, '合格') <> '不合格'"
        ).fetchall():
            lot = dict(row)
            # The acceptance form has a 標示(有效日期) column. When the supplier
            # printed a date, use it — an inferred date is a fallback for the
            # (common) case where the label carries no expiry at all.
            expires = None
            source = None
            if lot["expiry_date"]:
                expires, source = date.fromisoformat(lot["expiry_date"]), "標示"
            elif lot["shelf_life_days"] and lot["manufacture_date"]:
                expires = date.fromisoformat(lot["manufacture_date"]) + timedelta(days=lot["shelf_life_days"])
                source = "推算"
            if expires:
                remaining = (expires - today).days
                if remaining <= expiry_days:
                    out["expiring"].append({**lot, "expires_on": expires.isoformat(),
                                            "days_left": remaining, "expiry_source": source})
            age = (today - date.fromisoformat(lot["receipt_date"])).days
            if age >= stale_days:
                out["stale"].append({**lot, "age_days": age})

        for row in conn.execute(
            "SELECT i.item_code, i.name, i.safety_stock, COALESCE(SUM(l.qty_on_hand), 0) AS on_hand"
            " FROM inventory_item i LEFT JOIN inventory_lot l ON l.item_code = i.item_code"
            "   AND COALESCE(l.verdict, '合格') <> '不合格'"
            " WHERE i.safety_stock > 0 GROUP BY i.item_code HAVING on_hand < i.safety_stock"
        ).fetchall():
            out["low_stock"].append(dict(row))

        for row in conn.execute(
            "SELECT l.*, i.name FROM inventory_lot l JOIN inventory_item i ON i.item_code = l.item_code"
            " WHERE l.verdict = '不合格' AND l.qty_on_hand > 0"
        ).fetchall():
            out["rejected"].append(dict(row))

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
