"""Urd-WMS vertical slice — receiving, image-recognised issuing, FIFO, traceability, alerts.

Scope is one working path end to end, not the full v1 (requirement US-1/2/3/4/5
plus the US-8 alerts). Everything here uses the same core the PoC measured, so
what you see on screen is the behaviour that was measured, not a mock of it.
"""

from __future__ import annotations

import json
import os
import sys
import time
from datetime import date, datetime, timedelta
from pathlib import Path

from fastapi import Depends, FastAPI, File, Form, Header, HTTPException, UploadFile
from fastapi.concurrency import run_in_threadpool
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "core"))

from urdwms_core.camera import CameraConfig, CameraError, capture as camera_capture  # noqa: E402
from urdwms_core.matching import (  # noqa: E402
    Candidate, fifo_basis, fifo_expected, fifo_target, match_candidates, match_item_code,
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

def _candidates(conn, item_id: int) -> list[Candidate]:
    # A rejected lot is not a candidate. It stays on the books (the record is the
    # deliverable) but FIFO must never point anyone at it, and nobody may draw it.
    rows = conn.execute(
        "SELECT id, receipt_date, manufacture_date FROM inventory_lot"
        " WHERE item_id = ? AND qty_on_hand > 0 AND COALESCE(verdict, '合格') <> '不合格'"
        " ORDER BY receipt_date",
        (item_id,),
    ).fetchall()
    return [Candidate(str(r["id"]), r["receipt_date"], r["manufacture_date"]) for r in rows]


def _fifo_verdict(candidates: list[Candidate], lot_id: str) -> tuple[bool, str | None]:
    """FIFO compares 製造日期, so lots made the same day are equally legal.

    The date returned is what the operator is told to look for, so it must be
    the same field the judgement used — telling someone to fetch a receipt date
    while grading them on manufacture date is how a rule stops being followed.
    """
    expected = fifo_expected(candidates)
    if not expected:
        return False, None
    target = next(c for c in candidates if c.lot_id == expected[0])
    return lot_id in expected, target.manufacture_date or target.receipt_date


# Built once and reused. Constructing a client per request meant a fresh TLS
# handshake and auth exchange on every photo — most of the wait was that, not
# the model: the same recognition takes ~7s against a warm client and ~23s
# against a cold one.
_PROVIDER: GeminiProvider | None = None


def _assert_supplier_code_free(conn, supplier_code: str | None, exclude_id: int | None) -> None:
    """A box code must identify exactly one item.

    Two items sharing one printed code makes every photo of that box ambiguous
    for ever — the matcher correctly refuses to guess, so recognition simply
    never works for either item. Better to refuse the duplicate at entry than to
    let it quietly disable the feature.
    """
    code = (supplier_code or "").strip()
    if not code:
        return
    sql = "SELECT id, name, model FROM inventory_item WHERE supplier_code = ?"
    params: list = [code]
    if exclude_id is not None:
        sql += " AND id <> ?"
        params.append(exclude_id)
    clash = conn.execute(sql, params).fetchone()
    if clash:
        label = clash["model"] or clash["name"]
        raise HTTPException(
            409,
            f"箱上料號 {code} 已經是「{label}」的。同一個料號指到兩個品項的話，"
            " 拍照永遠分不出是哪一個 —— 請確認哪一個才對。",
        )


def _provider() -> GeminiProvider:
    global _PROVIDER
    if _PROVIDER is None:
        # Flash, measured rather than assumed: over repeated runs on the same
        # box photo it reads the 型號 and both dates exactly as the pro model
        # does, at a median 8.4s against 19.1s. On a packing line that gap is
        # the difference between waiting and not bothering.
        chosen = _setting(RECOGNITION_KEY,
                          {"model": os.environ.get("URDWMS_MODEL", "gemini-3.7-flash")})["model"]
        _PROVIDER = GeminiProvider(model=chosen, media_resolution="high")
    return _PROVIDER


# --------------------------------------------------------------------------- schemas

class LotIn(BaseModel):
    """One line off the acceptance form (包材驗收單).

    Field order on paper: 進貨日期 / 廠商名稱 / 原物料名稱 / 型號 / 數量(米).
    原物料名稱 is required and 型號 is optional — the form's 脫氧劑 line has no
    model number at all, so 型號 cannot be the identifier. The long code printed
    on the box lives in `supplier_code` and exists only so recognition can map a
    label back to an item.
    """

    # 既有品項填 item_id；新品項留空並填 item_name（必填）與 model（選填）。
    item_id: int | None = None
    receipt_date: str
    supplier: str | None = None         # 廠商名稱
    manufacture_date: str | None = None
    supplier_lot_code: str | None = None
    qty: int = 1
    # Receiving is where an item first appears in the system, so a new one must
    # be creatable here rather than sending the warehouse to a separate
    # master-data screen first. 原物料名稱 is what identifies it; 型號 is optional
    # (the form's 脫氧劑 line has none).
    item_name: str | None = None
    model: str | None = None
    spec: str | None = None
    unit: str = "箱"
    shelf_life_days: int | None = None
    safety_stock: int = 0
    meters_per_box: int | None = None
    pack_unit: str | None = None
    has_expiry: bool | None = None
    use_recognition: bool | None = None
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
    name: str                    # 原物料名稱. 必填
    model: str | None = None     # 型號. 選填
    spec: str | None = None
    unit: str = "箱"
    shelf_life_days: int | None = None
    safety_stock: int = 0
    meters_per_box: int | None = None
    pack_unit: str | None = None
    has_expiry: bool = False
    use_recognition: bool = True
    supplier_code: str | None = None
    supplier: str | None = None


class ItemPatch(BaseModel):
    name: str | None = None
    model: str | None = None
    spec: str | None = None
    # shelf_life_days is retained on the row for history but is no longer used:
    # expiry is the date recorded at receiving, not one counted from a duration.
    shelf_life_days: int | None = None
    safety_stock: int | None = None
    meters_per_box: int | None = None
    pack_unit: str | None = None
    has_expiry: bool | None = None
    use_recognition: bool | None = None
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
    item_id: int
    lot_id: int | None = None
    # Usually one box, occasionally more. Defaulting to 1 keeps the common case
    # a single tap without pretending the other case does not happen.
    qty: int = 1
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


@app.delete("/api/dictionary/{entry_id}")
def delete_dictionary_entry(entry_id: int,
                            user: dict = Depends(requires("dictionary.manage"))) -> dict:
    """Remove an option outright.

    Safe because records store the chosen value as text, not a reference —
    deleting 「弘東京」 from the list does not blank it out of last year's
    receiving lines. Deactivating is still the gentler option when the value is
    merely retired; deletion is for entries that were mistakes.
    """
    with transaction() as conn:
        row = conn.execute("SELECT * FROM dictionary WHERE id = ?", (entry_id,)).fetchone()
        if row is None:
            raise HTTPException(404, "字典項目不存在")
        conn.execute("DELETE FROM dictionary WHERE id = ?", (entry_id,))
        log(conn, user["name"], "dictionary.delete",
            {"category": row["category"], "value": row["value"]})
    return {"id": entry_id, "deleted": True, "value": row["value"]}


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


# --------------------------------------------------------------------------- camera

CAMERA_KEY = "camera"
RECOGNITION_KEY = "recognition"
ALERT_KEY = "alerts"

# Measured on the field photos (docs/poc/recognition-poc-spec.md). Offered as a
# choice rather than hard-coded because the trade-off is a factory's to make:
# on a slow line the pro model's extra seconds may be fine.
RECOGNITION_MODELS = [
    {"value": "gemini-3.7-flash", "label": "Flash（預設）", "note": "中位 8.4 秒，實測讀值與 Pro 相同"},
    {"value": "gemini-pro-latest", "label": "Pro", "note": "中位 19.1 秒，同樣讀對"},
    {"value": "gemini-3.5-flash", "label": "Flash 3.5", "note": "較舊，實測把進貨日讀錯過"},
]

DEFAULT_ALERTS = {"expiry_days": 60, "stale_days": 120, "pending_hours": 24}


def _setting(key: str, fallback: dict) -> dict:
    with transaction() as conn:
        row = conn.execute("SELECT value FROM app_setting WHERE key = ?", (key,)).fetchone()
    return {**fallback, **(json.loads(row["value"]) if row else {})}


def _save_setting(key: str, value: dict, actor: str) -> None:
    with transaction() as conn:
        conn.execute(
            "INSERT INTO app_setting (key, value, updated_at, updated_by) VALUES (?,?,?,?)"
            " ON CONFLICT(key) DO UPDATE SET value = excluded.value,"
            " updated_at = excluded.updated_at, updated_by = excluded.updated_by",
            (key, json.dumps(value, ensure_ascii=False), now(), actor),
        )
        log(conn, actor, f"setting.{key}", value)


class RecognitionIn(BaseModel):
    model: str


class AlertThresholdsIn(BaseModel):
    expiry_days: int
    stale_days: int
    pending_hours: int


@app.get("/api/settings/recognition")
def get_recognition(user: dict = Depends(current_user)) -> dict:
    return {
        **_setting(RECOGNITION_KEY, {"model": os.environ.get("URDWMS_MODEL", "gemini-3.7-flash")}),
        "models": RECOGNITION_MODELS,
    }


@app.put("/api/settings/recognition")
def set_recognition(payload: RecognitionIn,
                    user: dict = Depends(requires("dictionary.manage"))) -> dict:
    if payload.model not in {m["value"] for m in RECOGNITION_MODELS}:
        raise HTTPException(400, f"未知的模型 {payload.model}")
    _save_setting(RECOGNITION_KEY, {"model": payload.model}, user["name"])
    global _PROVIDER
    _PROVIDER = None   # rebuilt on next use so the change takes effect now
    return {"ok": True, "model": payload.model}


@app.get("/api/settings/alerts")
def get_alert_thresholds(user: dict = Depends(current_user)) -> dict:
    return _setting(ALERT_KEY, DEFAULT_ALERTS)


@app.put("/api/settings/alerts")
def set_alert_thresholds(payload: AlertThresholdsIn,
                         user: dict = Depends(requires("dictionary.manage"))) -> dict:
    values = payload.model_dump()
    for key, value in values.items():
        if value < 1:
            raise HTTPException(400, f"{key} 必須大於 0")
    _save_setting(ALERT_KEY, values, user["name"])
    return {"ok": True, **values}


class CameraIn(BaseModel):
    enabled: bool = False
    transport: str = "http"
    host: str = ""
    port: int = 80
    path: str = "/snapshot.jpg"
    username: str = ""
    password: str = ""
    trigger: str = ""
    timeout: float = 8.0


def _camera_config() -> CameraConfig:
    with transaction() as conn:
        row = conn.execute("SELECT value FROM app_setting WHERE key = ?", (CAMERA_KEY,)).fetchone()
    if not row:
        return CameraConfig()
    return CameraConfig(**json.loads(row["value"]))


@app.get("/api/camera")
def get_camera(user: dict = Depends(current_user)) -> dict:
    config = _camera_config()
    return {
        # The password is never returned — only whether one is set. A settings
        # screen that round-trips a secret leaks it to every browser that opens
        # the page.
        **{k: v for k, v in config.__dict__.items() if k != "password"},
        "has_password": bool(config.password),
        "endpoint": config.endpoint if config.host else None,
    }


@app.put("/api/camera")
def set_camera(payload: CameraIn, user: dict = Depends(requires("dictionary.manage"))) -> dict:
    if payload.transport not in ("http", "raw"):
        raise HTTPException(400, "連線方式只能是 http 或 raw")
    if payload.enabled and not payload.host.strip():
        raise HTTPException(400, "啟用網路相機必須填位址")
    if not 1 <= payload.port <= 65535:
        raise HTTPException(400, "連接埠必須在 1–65535")

    existing = _camera_config()
    data = payload.model_dump()
    # Blank password means "leave it alone", so a settings save does not wipe a
    # credential the form never received.
    if not data["password"]:
        data["password"] = existing.password

    with transaction() as conn:
        conn.execute(
            "INSERT INTO app_setting (key, value, updated_at, updated_by) VALUES (?,?,?,?)"
            " ON CONFLICT(key) DO UPDATE SET value = excluded.value,"
            " updated_at = excluded.updated_at, updated_by = excluded.updated_by",
            (CAMERA_KEY, json.dumps(data, ensure_ascii=False), now(), user["name"]),
        )
        log(conn, user["name"], "camera.configure",
            {k: v for k, v in data.items() if k != "password"})
    return {"ok": True, "endpoint": CameraConfig(**data).endpoint}


@app.post("/api/camera/test")
async def test_camera(user: dict = Depends(requires("dictionary.manage"))) -> dict:
    """Fetch one frame and report what happened, without keeping it."""
    config = _camera_config()
    started = time.monotonic()
    try:
        data = await run_in_threadpool(camera_capture, config)
    except CameraError as exc:
        return {"ok": False, "endpoint": config.endpoint, "error": str(exc)}
    return {
        "ok": True,
        "endpoint": config.endpoint,
        "bytes": len(data),
        "elapsed_ms": round((time.monotonic() - started) * 1000),
    }


@app.post("/api/camera/capture")
async def capture_from_camera(user: dict = Depends(requires("issue.create"))) -> dict:
    """Grab a frame and run it through recognition, exactly as an upload would.

    Same downstream path as a phone photo — the issuing flow takes bytes and
    does not care where they came from, which is the whole point of keeping the
    capture source behind an interface.
    """
    config = _camera_config()
    if not config.enabled:
        raise HTTPException(400, "網路相機未啟用，請在基本資料設定")
    try:
        data = await run_in_threadpool(camera_capture, config)
    except CameraError as exc:
        raise HTTPException(502, str(exc)) from exc

    stamp = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    path = UPLOADS / f"{stamp}-cam.jpg"
    path.write_bytes(data)
    return await _recognise_path(path, data, None)


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
    # 廠商 IS a free-standing thing (one supplier serves many items), unlike
    # 原物料名稱/規格 which only ever describe one item — hence a list of its
    # own that can be curated.
    "supplier": "廠商",
    "job_title": "職位名稱",
    "pack_unit": "計量單位",
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
def items(order: str = "sort", user: dict = Depends(current_user)) -> list[dict]:
    """Item master.

    Two screens want two orders and neither is wrong: the master is arranged by
    hand (`order=sort`), while stock is most useful with the latest delivery on
    top (`order=recent`). Sorting client-side instead would mean each screen
    quietly disagreeing about what "first" means.
    """
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
            " COALESCE(SUM(CASE WHEN l.verdict = '不合格' THEN l.qty_on_hand ELSE 0 END), 0) AS rejected_qty,"
            " MAX(l.receipt_date) AS last_receipt_date,"
            " MAX(l.created_at) AS last_lot_at"
            " FROM inventory_item i LEFT JOIN inventory_lot l ON l.item_id = i.id"
            # Most recently received first: what just arrived is what people are
            # looking for. Items with no lots yet sort last rather than first,
            # since an empty row at the top is noise.
            " GROUP BY i.id"
            + (" ORDER BY last_lot_at IS NULL, last_lot_at DESC, last_receipt_date DESC, i.name"
               if order == "recent"
               # Hand-arranged order; never-arranged rows fall back to id so the
               # list is always stable rather than arbitrary.
               else " ORDER BY COALESCE(i.sort_order, i.id)"),
        ).fetchall()
    return [
        {
            **dict(r),
            "on_hand_m": meters_from_boxes(r["on_hand"], r["meters_per_box"]),
            # Two conditions, and they mean different things: something on the
            # label to match against, AND someone having decided to use it.
            "matchable": bool(r["model"] or r["supplier_code"]),
            "recognisable": bool(r["use_recognition"] and (r["model"] or r["supplier_code"])),
            "label": r["model"] or r["name"],
        }
        for r in rows
    ]


class ReorderIn(BaseModel):
    item_ids: list[int]


@app.post("/api/items/reorder")
def reorder_items(payload: ReorderIn,
                  user: dict = Depends(requires("item.manage"))) -> dict:
    """Persist a hand-arranged order for the item master.

    A master list someone maintains gets grouped the way that person thinks —
    films together, bags together, the thing they touch daily at the top. That
    grouping is not derivable from any column, so it is stored.
    """
    with transaction() as conn:
        for position, item_id in enumerate(payload.item_ids):
            conn.execute("UPDATE inventory_item SET sort_order = ? WHERE id = ?",
                         (position, item_id))
        log(conn, user["name"], "item.reorder", {"count": len(payload.item_ids)})
    return {"ok": True, "count": len(payload.item_ids)}


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
        dict_suppliers = [r["value"] for r in conn.execute(
            "SELECT value FROM dictionary WHERE category = 'supplier' AND active = 1"
            " ORDER BY sort_order, value").fetchall()]
    def distinct(key: str) -> list[str]:
        return sorted({r[key] for r in rows if r[key]})
    return {
        # Curated list first, then any supplier already on an item that has not
        # been added to it — so an existing value never becomes unselectable
        # just because someone tidied the list.
        "supplier": dict_suppliers + [v for v in distinct("supplier") if v not in dict_suppliers],
        "material_name": distinct("name"),
        "spec": distinct("spec"),
    }


@app.patch("/api/items/{item_id}")
def update_item(item_id: int, payload: ItemPatch,
                user: dict = Depends(requires("item.manage"))) -> dict:
    """Edit the master record, including 型號 and the metres-per-box rate."""
    changes = {k: v for k, v in payload.model_dump().items() if v is not None}
    if not changes:
        raise HTTPException(400, "沒有要更新的欄位")
    if "name" in changes and not str(changes["name"]).strip():
        raise HTTPException(400, "原物料名稱不可空白")
    if "meters_per_box" in changes and changes["meters_per_box"] <= 0:
        raise HTTPException(400, "每箱數量必須大於 0（不換算請留空）")
    if "model" in changes:
        changes["model"] = str(changes["model"]).strip() or None
    if "has_expiry" in changes:
        changes["has_expiry"] = int(bool(changes["has_expiry"]))
    if "use_recognition" in changes:
        changes["use_recognition"] = int(bool(changes["use_recognition"]))
    if "pack_unit" in changes:
        changes["pack_unit"] = str(changes["pack_unit"]).strip() or None
    with transaction() as conn:
        if not conn.execute("SELECT 1 FROM inventory_item WHERE id = ?", (item_id,)).fetchone():
            raise HTTPException(404, "品項不存在")
        if changes.get("model") and conn.execute(
                "SELECT 1 FROM inventory_item WHERE model = ? AND id <> ?",
                (changes["model"], item_id)).fetchone():
            raise HTTPException(409, f"型號 {changes['model']} 已被其他品項使用")
        if "supplier_code" in changes:
            _assert_supplier_code_free(conn, changes["supplier_code"], item_id)
        conn.execute(f"UPDATE inventory_item SET {', '.join(f'{k} = ?' for k in changes)}"
                     " WHERE id = ?", (*changes.values(), item_id))
        log(conn, user["name"], "item.update", {"id": item_id, **changes})
    return {"id": item_id, **changes}


@app.post("/api/items")
def create_item(payload: ItemIn, user: dict = Depends(requires("item.manage"))) -> dict:
    name = payload.name.strip()
    model = (payload.model or "").strip() or None
    if not name:
        raise HTTPException(400, "原物料名稱不可空白")
    with transaction() as conn:
        if model and conn.execute("SELECT 1 FROM inventory_item WHERE model = ?", (model,)).fetchone():
            raise HTTPException(409, f"型號 {model} 已存在")
        _assert_supplier_code_free(conn, payload.supplier_code, None)
        cursor = conn.execute(
            "INSERT INTO inventory_item (name, model, spec, unit, shelf_life_days, safety_stock,"
            " meters_per_box, pack_unit, has_expiry, supplier_code, supplier)"
            " VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (name, model, payload.spec, payload.unit, payload.shelf_life_days,
             payload.safety_stock, payload.meters_per_box, payload.pack_unit,
             int(payload.has_expiry), payload.supplier_code, payload.supplier),
        )
        log(conn, user["name"], "item.create", {"id": cursor.lastrowid, "name": name, "model": model})
    return {"id": cursor.lastrowid, "name": name, "model": model}


@app.delete("/api/items/{item_id}")
def delete_item(item_id: int, user: dict = Depends(requires("item.manage"))) -> dict:
    """Delete a 型號 — only while no lot references it.

    Same boundary as deleting a lot: the master record is what a usage record's
    item_id resolves to, so removing one with history behind it turns every
    traceability answer about it into a dangling reference.
    """
    with transaction() as conn:
        row = conn.execute("SELECT * FROM inventory_item WHERE id = ?", (item_id,)).fetchone()
        if row is None:
            raise HTTPException(404, "品項不存在")
        label = row["model"] or row["name"]
        lots = conn.execute("SELECT COUNT(*) AS n FROM inventory_lot WHERE item_id = ?",
                            (item_id,)).fetchone()["n"]
        scans = conn.execute("SELECT COUNT(*) AS n FROM material_usage_scan WHERE item_id = ?",
                             (item_id,)).fetchone()["n"]
        if lots or scans:
            raise HTTPException(
                409,
                f"{label} 已有 {lots} 批進貨、{scans} 筆領用紀錄，刪掉會讓那些紀錄失去對應。"
                " 不再使用請把安全水位設 0 並停止收貨。",
            )
        conn.execute("DELETE FROM inventory_item WHERE id = ?", (item_id,))
        log(conn, user["name"], "item.delete", {"id": item_id, "name": row["name"], "model": row["model"]})
    return {"id": item_id, "deleted": True}


@app.get("/api/lots")
def lots(item_id: int | None = None, user: dict = Depends(current_user)) -> list[dict]:
    with transaction() as conn:
        sql = ("SELECT l.*, i.name AS item_name, i.model AS item_model, i.spec AS item_spec,"
               " i.has_expiry AS item_has_expiry, i.shelf_life_days AS item_shelf_life_days"
               " FROM inventory_lot l JOIN inventory_item i ON i.id = l.item_id")
        params: tuple = ()
        if item_id:
            sql += " WHERE l.item_id = ?"
            params = (item_id,)
        sql += " ORDER BY i.name, i.model, l.receipt_date"
        rows = [dict(r) for r in conn.execute(sql, params).fetchall()]
        drawn = {r["lot_id"]: r["n"] for r in conn.execute(
            "SELECT lot_id, COUNT(*) AS n FROM material_usage_scan"
            " WHERE lot_id IS NOT NULL GROUP BY lot_id").fetchall()}

        by_item: dict[int, list[Candidate]] = {}
        for row in rows:
            by_item.setdefault(row["item_id"], [])
        for key in by_item:
            by_item[key] = _candidates(conn, key)

    for row in rows:
        pool = by_item.get(row["item_id"], [])
        # One lot gets the badge. Judgement still accepts every same-day lot
        # (see fifo_ok on the issuing side) — but a screen marking two lots
        # "應領" has told nobody which box to pick up.
        row["is_fifo_next"] = str(row["id"]) == fifo_target(pool)
        row["fifo_also_ok"] = (
            str(row["id"]) in fifo_expected(pool) and str(row["id"]) != fifo_target(pool)
        )
        # Which field the guidance rests on. Falling back to receipt date
        # because nothing is dated is a weaker claim and should look like one.
        row["fifo_basis"] = fifo_basis(pool)
        row["inspection"] = json.loads(row.get("inspection") or "{}")
        row["draw_count"] = drawn.get(row["id"], 0)
        row["item_label"] = row["item_model"] or row["item_name"]
        received = row.get("qty_received") or row["qty_on_hand"]
        row["qty_received"] = received
        row["qty_drawn"] = max(0, received - row["qty_on_hand"])
        row["lot_state"] = (
            "已領完" if row["qty_on_hand"] <= 0
            else "領貨中" if row["qty_on_hand"] < received
            else "未動用"
        )
        # The expiry is the date recorded at receiving, full stop. Inferring it
        # from a shelf life would produce a date nobody wrote down and nobody
        # can check against the box — and the acceptance form has a 有效日期
        # column precisely because that is where the real answer comes from.
        row["effective_expiry"] = row["expiry_date"]
        row["days_left"] = (
            (date.fromisoformat(row["expiry_date"]) - date.today()).days
            if row["expiry_date"] else None
        )
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
            "deleted": {k: row[k] for k in ("item_id", "receipt_date", "manufacture_date",
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
    created_item = False
    with transaction() as conn:
        item_id = payload.item_id
        known = conn.execute("SELECT * FROM inventory_item WHERE id = ?", (item_id,)).fetchone() \
            if item_id else None
        if known is None:
            # New item. 原物料名稱 is what identifies it; 型號 is optional, because
            # the acceptance form's 脫氧劑 line simply has no model number.
            if not (payload.item_name or "").strip():
                raise HTTPException(400, "新品項請填原物料名稱")
            model = (payload.model or "").strip() or None
            if model and conn.execute("SELECT 1 FROM inventory_item WHERE model = ?", (model,)).fetchone():
                raise HTTPException(409, f"型號 {model} 已存在，請直接選那個品項")
            cursor = conn.execute(
                "INSERT INTO inventory_item (name, model, spec, unit, shelf_life_days, safety_stock,"
                " meters_per_box, pack_unit, has_expiry, use_recognition, supplier_code, supplier)"
                " VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (payload.item_name.strip(), model, payload.spec, payload.unit,
                 payload.shelf_life_days, payload.safety_stock, payload.meters_per_box,
                 payload.pack_unit, int(bool(payload.has_expiry)),
                 int(bool(model or payload.supplier_code)),
                 payload.supplier_code, payload.supplier),
            )
            item_id = cursor.lastrowid
            created_item = True
            log(conn, user["name"], "item.create",
                {"id": item_id, "name": payload.item_name, "model": model})
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
                conn.execute(f"UPDATE inventory_item SET {', '.join(updates)} WHERE id = ?",
                             (*params, item_id))
                log(conn, user["name"], "item.update", {"id": item_id, "changed": updates})
        # One delivery can arrive as several manufacture-date batches, and the form
        # records them as separate lines — so same 型號 + same receipt date is
        # NORMAL, not a duplicate. Only an identical manufacture date makes it
        # look like the same lot entered twice. Warning on the normal case would
        # train people to dismiss the warning, and then the real one goes unread.
        existing = conn.execute(
            "SELECT id, qty_on_hand FROM inventory_lot"
            " WHERE item_id = ? AND receipt_date = ?"
            "   AND COALESCE(manufacture_date, '') = COALESCE(?, '')",
            (item_id, parsed.iso, manufacture.iso if manufacture else None),
        ).fetchone()
        # An item flagged as having a shelf life must arrive with something that
        # pins down when it expires — the printed 有效日期, or a 製造日期 the
        # shelf-life days can be counted from. Without either, the expiry alert
        # has nothing to work with and would quietly never fire for this lot.
        item_row = conn.execute("SELECT name, model, has_expiry FROM inventory_item WHERE id = ?",
                                (item_id,)).fetchone()
        if item_row and item_row["has_expiry"] and not (expiry and expiry.iso):
            label = item_row["model"] or item_row["name"]
            raise HTTPException(400, f"{label} 有效期，請填「有效期限」。")

        qty = payload.qty
        conversion = None
        if payload.qty_meters is not None:
            # The rate may have been supplied on this very request (new item), so
            # read it back rather than trusting the payload alone.
            rate = conn.execute("SELECT meters_per_box FROM inventory_item WHERE id = ?",
                                (item_id,)).fetchone()["meters_per_box"]
            conversion = boxes_from_meters(payload.qty_meters, rate)
            if conversion is None:
                raise HTTPException(400, "此品項尚未設定每箱數量，無法用單上數量收貨")
            if conversion.boxes < 1:
                raise HTTPException(
                    400,
                    f"{payload.qty_meters:,} 不足一箱（每箱 {rate:,}）。"
                    " 庫存以箱為單位，不做部分入庫。",
                )
            qty = conversion.boxes

        cursor = conn.execute(
            "INSERT INTO inventory_lot (item_id, receipt_date, manufacture_date, expiry_date,"
            " supplier_lot_code, supplier, entered_meters, entered_unit, inspection, verdict,"
            " recorded_by, confirmed_by, remark, qty_received, qty_on_hand, created_at, created_by)"
            " VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (item_id, parsed.iso, manufacture.iso if manufacture else None,
             expiry.iso if expiry else None, payload.supplier_lot_code, payload.supplier,
             payload.qty_meters, payload.entered_unit,
             json.dumps(payload.inspection, ensure_ascii=False), payload.verdict,
             # 記錄人 is who is signed in, not a name picked from a list. Dual
             # sign-off is the form's control point; a dropdown of names lets
             # anyone sign as anyone and the control means nothing.
             user["name"], payload.confirmed_by, payload.remark, qty, qty, now(), user["name"]),
        )
        lot_id = cursor.lastrowid
        log(conn, user["name"], "lot.create",
            {"lot_id": lot_id, "item_id": item_id, "receipt_date": parsed.iso, "qty": qty,
             "entered": payload.qty_meters, "verdict": payload.verdict,
             "recorded_by": user["name"], "confirmed_by": payload.confirmed_by})
    return {
        "id": lot_id,
        "item_id": item_id,
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
async def recognize(image: UploadFile = File(...), item_id: int | None = Form(default=None),
                    user: dict = Depends(requires("issue.create"))) -> dict:
    """Recognise an uploaded box photo: which item it is, and which lot.

    Only items with a 型號 or a registered supplier code can be identified this
    way — there is nothing on the label to map back from otherwise (the form's
    脫氧劑 line has no model number). Those are picked by hand, which is a normal
    path, not a failure.

    Passing `item_id` overrides the label reading; that is the path used when
    the operator has already resolved a deferral.

    Nothing is written and no stock moves. The confirmation step stays because
    "this is not the box I picked up" has no other line of defence
    (M7 architecture, constraint 4).
    """
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    suffix = Path(image.filename or "capture.jpg").suffix or ".jpg"
    path = UPLOADS / f"{stamp}{suffix}"
    data = await image.read()
    path.write_bytes(data)
    return await _recognise_path(path, data, item_id)


async def _recognise_path(path: Path, data: bytes, item_id: int | None) -> dict:
    """Recognise a saved image and match it against stock.

    Shared by the upload path and the network camera: whatever produced the
    bytes, everything downstream is identical — which is what makes a fixed
    camera an addition rather than a second implementation of issuing.
    """
    t_start = time.monotonic()
    try:
        # Off the event loop: the SDK call is blocking, and holding the loop for
        # several seconds would stall every other request on a shared tablet.
        reading: Recognition = await run_in_threadpool(_provider().recognize, path)
    except RuntimeError as exc:
        reading = Recognition(error=str(exc))
    t_model = time.monotonic()

    with transaction() as conn:
        # Items with recognition switched off are excluded from matching
        # entirely — that is what switching it off means.
        master = [(str(r["id"]), r["model"], r["supplier_code"]) for r in conn.execute(
            "SELECT id, model, supplier_code FROM inventory_item WHERE use_recognition = 1").fetchall()]
        catalogue = [
            {**dict(r), "label": r["model"] or r["name"],
             "recognisable": bool(r["use_recognition"] and (r["model"] or r["supplier_code"]))}
            for r in conn.execute(
                "SELECT i.id, i.name, i.model, i.spec, i.meters_per_box, i.supplier_code,"
                " i.use_recognition,"
                "       COALESCE(SUM(CASE WHEN COALESCE(l.verdict, '合格') <> '不合格'"
                "                          THEN l.qty_on_hand ELSE 0 END), 0) AS on_hand"
                " FROM inventory_item i LEFT JOIN inventory_lot l ON l.item_id = i.id"
                " GROUP BY i.id ORDER BY i.name, i.model").fetchall()
        ]

    item_match = match_item_code(reading.item_code, master, model_code=reading.model_code)
    resolved = item_id or (int(item_match.item_id) if item_match.locked else None)

    payload: dict = {
        "image_path": f"/uploads/{path.name}",
        # Surfaced so "recognition is slow" can be pinned to a stage rather than
        # guessed at.
        "timing_ms": {
            "model": round((t_model - t_start) * 1000),
            "image_kb": round(len(data) / 1024),
        },
        "recognition": {
            "receipt_date": reading.receipt_date,
            "manufacture_date": reading.manufacture_date,
            "model_code": reading.model_code,
            "item_code": reading.item_code,
            "confidence": reading.receipt_date_confidence,
            "item_code_confidence": reading.item_code_confidence,
            "stamp_visible": reading.stamp_visible,
            "notes": reading.notes,
            "error": reading.error,
        },
        "item_match": {
            "decision": "lock" if resolved else "defer",
            "item_id": resolved,
            # Distinguishes "the label said so" from "a human overrode it",
            # which matters when reading the record back months later.
            "matched_on": item_match.matched_on if item_match.locked and not item_id else
                          ("manual" if item_id else None),
            "reason": item_match.reason.value if item_match.reason and not resolved else None,
            "contenders": list(item_match.contenders),
        },
        # Sent along so a deferral can be resolved without a second round trip.
        "catalogue": catalogue,
    }

    if resolved is None:
        payload.update({"item_id": None, "item_name": None, "item_label": None, "candidates": [],
                        "decision": "defer", "defer_reason": "item_unresolved",
                        "match_distance": None, "locked_lot": None,
                        "fifo_ok": None, "fifo_expected_date": None})
        return payload

    payload.update(_lot_lookup(resolved, reading.receipt_date, reading.manufacture_date))
    return payload


def _lot_lookup(item_id: int, read_receipt_date: str | None,
                read_manufacture_date: str | None = None) -> dict:
    """Match the dates read off the box against this item's drawable lots.

    Both dates go in. 製造日 identifies the lot when it can (it is the FIFO key
    and it is printed rather than stamped); 進貨日 settles it when two lots were
    made the same day.
    """
    with transaction() as conn:
        candidates = _candidates(conn, item_id)
        item = conn.execute("SELECT * FROM inventory_item WHERE id = ?", (item_id,)).fetchone()
        lot_rows = {str(r["id"]): dict(r) for r in conn.execute(
            "SELECT * FROM inventory_lot WHERE item_id = ? AND qty_on_hand > 0"
            " AND COALESCE(verdict, '合格') <> '不合格'", (item_id,)).fetchall()}

    result = match_candidates(
        to_date_key(read_receipt_date),
        candidates,
        manufacture=to_date_key(read_manufacture_date),
    )
    out: dict = {
        "item_id": item_id,
        "item_name": item["name"] if item else None,
        "item_model": item["model"] if item else None,
        "item_label": (item["model"] or item["name"]) if item else None,
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
        # Which date on the box actually named the lot. A lock off the printed
        # 製造日 is a stronger claim than one off the hand-stamped 進貨日, and
        # the record should be able to say which it was.
        "lot_matched_on": result.matched_on,
        "match_distance": result.best_distance,
        "fifo_target_lot_id": int(fifo_target(candidates)) if fifo_target(candidates) else None,
        "fifo_basis": fifo_basis(candidates),
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
    item_id: int
    ocr_receipt_date: str | None = None
    ocr_manufacture_date: str | None = None


@app.post("/api/resolve-item")
def resolve_item(payload: ResolveIn, user: dict = Depends(requires("issue.create"))) -> dict:
    """Re-run the lot match after a human picks the 型號 recognition could not.

    Deliberately does not touch the image: recognition already ran and is
    billed. Re-uploading to change one field would cost a second call and could
    return a *different* reading, which would be confusing on screen.
    """
    return {
        **_lot_lookup(payload.item_id, payload.ocr_receipt_date, payload.ocr_manufacture_date),
        "item_match": {"decision": "lock", "item_id": payload.item_id,
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
        candidates = _candidates(conn, payload.item_id)

        if payload.lot_id is None:
            scan_id = _insert_scan(conn, payload, status="blocked_unreadable", lot_id=None,
                                   fifo_expected=(None, None), user=user)
            log(conn, user["name"], "scan.blocked_unreadable", {"scan_id": scan_id})
            return {"id": scan_id, "status": "blocked_unreadable"}

        if payload.qty < 1:
            raise HTTPException(400, "領用數量至少 1 箱")

        lot = conn.execute("SELECT * FROM inventory_lot WHERE id = ?", (payload.lot_id,)).fetchone()
        if lot is None:
            raise HTTPException(404, "批次不存在")
        if lot["qty_on_hand"] < 1:
            raise HTTPException(409, "該批次已無庫存")
        if payload.qty > lot["qty_on_hand"]:
            # Refused rather than clamped: a request for more than exists means
            # the operator and the books disagree, and silently issuing fewer
            # would hide that.
            raise HTTPException(
                409,
                f"這批只剩 {lot['qty_on_hand']} 箱，領不了 {payload.qty} 箱。"
                " 若實際數量不符，請找管理者修正批次數量。",
            )

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
        conn.execute("UPDATE inventory_lot SET qty_on_hand = qty_on_hand - ?"
                     " WHERE id = ? AND qty_on_hand >= ?",
                     (payload.qty, payload.lot_id, payload.qty))
        log(conn, user["name"], "scan.posted",
            {"scan_id": scan_id, "lot_id": payload.lot_id, "qty": payload.qty})
        return {"id": scan_id, "status": "posted"}


def _insert_scan(conn, payload: ScanIn, *, status: str, lot_id: int | None,
                 fifo_expected: tuple[int | None, str | None], user: dict) -> int:
    cursor = conn.execute(
        "INSERT INTO material_usage_scan (item_id, lot_id, status, captured_at, captured_by,"
        " image_path, ocr_receipt_date, ocr_confidence, ocr_notes, match_distance,"
        " fifo_expected_lot_id, fifo_expected_date, field_values, detail_pending, qty, created_at)"
        " VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        (payload.item_id, lot_id, status, now(), user["name"], payload.image_path,
         payload.ocr_receipt_date, payload.ocr_confidence, payload.ocr_notes, payload.match_distance,
         fifo_expected[0], fifo_expected[1], json.dumps(payload.fields, ensure_ascii=False),
         int(payload.detail_pending), payload.qty, now()),
    )
    return int(cursor.lastrowid)


@app.get("/api/scans")
def scans(status: str | None = None, limit: int = 100,
          user: dict = Depends(current_user)) -> list[dict]:
    with transaction() as conn:
        sql = ("SELECT s.*, l.receipt_date, l.manufacture_date,"
               " i.name AS item_name, i.model AS item_model"
               " FROM material_usage_scan s"
               " LEFT JOIN inventory_lot l ON l.id = s.lot_id"
               " LEFT JOIN inventory_item i ON i.id = s.item_id")
        params: tuple = ()
        if status:
            sql += " WHERE s.status = ?"
            params = (status,)
        sql += " ORDER BY s.id DESC LIMIT ?"
        rows = conn.execute(sql, (*params, limit)).fetchall()
    return [
        {**dict(r), "field_values": json.loads(r["field_values"] or "{}"),
         "item_label": r["item_model"] or r["item_name"]}
        for r in rows
    ]


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
        conn.execute("UPDATE inventory_lot SET qty_on_hand = qty_on_hand - ?"
                     " WHERE id = ? AND qty_on_hand >= ?",
                     (row["qty"], row["lot_id"], row["qty"]))
        log(conn, user["name"], "scan.overridden", {"scan_id": scan_id, "reason": payload.reason})
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
    thresholds = _setting(ALERT_KEY, DEFAULT_ALERTS)
    expiry_days = thresholds["expiry_days"]
    stale_days = thresholds["stale_days"]
    pending_hours = thresholds["pending_hours"]
    out: dict[str, list] = {"expiring": [], "stale": [], "low_stock": [], "pending_detail": [],
                            "rejected": []}

    with transaction() as conn:
        for row in conn.execute(
            "SELECT l.*, i.name, i.shelf_life_days FROM inventory_lot l"
            " JOIN inventory_item i ON i.id = l.item_id"
            " WHERE l.qty_on_hand > 0 AND COALESCE(l.verdict, '合格') <> '不合格'"
        ).fetchall():
            lot = dict(row)
            # Only the date someone actually recorded. An alert fired off an
            # inferred date would be an alert about a number the system made up.
            if lot["expiry_date"]:
                remaining = (date.fromisoformat(lot["expiry_date"]) - today).days
                if remaining <= expiry_days:
                    out["expiring"].append({**lot, "expires_on": lot["expiry_date"],
                                            "days_left": remaining})
            age = (today - date.fromisoformat(lot["receipt_date"])).days
            if age >= stale_days:
                out["stale"].append({**lot, "age_days": age})

        for row in conn.execute(
            "SELECT i.id, i.name, i.model, i.safety_stock, COALESCE(SUM(l.qty_on_hand), 0) AS on_hand"
            " FROM inventory_item i LEFT JOIN inventory_lot l ON l.item_id = i.id"
            "   AND COALESCE(l.verdict, '合格') <> '不合格'"
            " WHERE i.safety_stock > 0 GROUP BY i.id HAVING on_hand < i.safety_stock"
        ).fetchall():
            out["low_stock"].append(dict(row))

        for row in conn.execute(
            "SELECT l.*, i.name, i.model FROM inventory_lot l"
            " JOIN inventory_item i ON i.id = l.item_id"
            " WHERE l.verdict = '不合格' AND l.qty_on_hand > 0"
        ).fetchall():
            out["rejected"].append(dict(row))

        cutoff = (datetime.now() - timedelta(hours=pending_hours)).isoformat(timespec="seconds")
        for row in conn.execute(
            "SELECT id, item_id, captured_at FROM material_usage_scan"
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
