"""Identity and permissions.

Username and password, hashed with PBKDF2 and a per-user salt.

Why this matters beyond gating buttons: the acceptance form's control is dual
sign-off, and 記錄人/確認人 used to be dropdowns, so anyone could sign anyone's
name. Recording the signature as the authenticated user is what makes the second
signature mean anything.
"""

from __future__ import annotations

import hashlib
import hmac
import secrets
from datetime import datetime, timedelta, timezone

from fastapi import Depends, Header, HTTPException

from .db import now, transaction

SESSION_HOURS = 12

# Three permission tiers, each a superset of the one below.
#
# A role is NOT a job title. 倉管 and 廠長 are different jobs that both need the
# same thing from the system — record deliveries, approve a non-FIFO draw — so
# they are both `manager`. The job title lives on the user (`app_user.title`)
# and is what screens show people; the role is what the server checks. Mixing
# them means every new job title needs a new permission set, which is how an
# access model turns into a mess.
PERMISSIONS: dict[str, set[str]] = {
    # 作業: 領用登錄與補明細
    "user": {"issue.create", "issue.detail"},
    # 管理: 加上收貨建批、維護型號、覆核放行、看日誌
    "manager": {"issue.create", "issue.detail", "lot.create", "item.manage",
                "scan.override", "audit.read"},
    # 系統: 加上批次修正與刪除、選項、人員與角色
    "admin": {"issue.create", "issue.detail", "lot.create", "item.manage",
              "scan.override", "audit.read", "lot.edit", "lot.delete",
              "dictionary.manage", "user.manage"},
}

# Shipped defaults. The live labels come from app_role so each factory can use
# its own words — 倉管 vs 資材 vs 物管 is a naming difference, not a different
# set of permissions, and forcing our vocabulary on them makes the screen read
# like someone else's system.
DEFAULT_ROLE_LABELS = {
    "user": "一般使用者",
    "manager": "管理者",
    "admin": "系統管理者",
}


def role_labels() -> dict[str, str]:
    """Current labels, falling back to the defaults for anything unset."""
    labels = dict(DEFAULT_ROLE_LABELS)
    try:
        with transaction() as conn:
            for row in conn.execute("SELECT code, label FROM app_role").fetchall():
                if row["code"] in labels:
                    labels[row["code"]] = row["label"]
    except Exception:  # noqa: BLE001 — a missing table must not break login
        pass
    return labels


def role_label(code: str) -> str:
    return role_labels().get(code, code)

MIN_PASSWORD_LENGTH = 8
_ITERATIONS = 240_000


def hash_password(password: str) -> str:
    """PBKDF2-SHA256 with a per-user random salt.

    Stored as `pbkdf2_sha256$iterations$salt$hash` so the work factor can be
    raised later without invalidating existing rows — a bare digest would pin
    the cost forever.
    """
    salt = secrets.token_hex(16)
    digest = hashlib.pbkdf2_hmac("sha256", password.encode(), bytes.fromhex(salt), _ITERATIONS).hex()
    return f"pbkdf2_sha256${_ITERATIONS}${salt}${digest}"


def verify_password(password: str, stored: str) -> bool:
    try:
        algorithm, iterations, salt, digest = stored.split("$")
    except ValueError:
        return False
    if algorithm != "pbkdf2_sha256":
        return False
    candidate = hashlib.pbkdf2_hmac(
        "sha256", password.encode(), bytes.fromhex(salt), int(iterations)).hex()
    return hmac.compare_digest(candidate, digest)


def check_password_policy(password: str) -> None:
    """Length only.

    Composition rules (one upper, one digit, one symbol) push people to
    `Password1!` and a sticky note. Length is the requirement that actually
    correlates with strength.
    """
    if len(password) < MIN_PASSWORD_LENGTH:
        raise HTTPException(400, f"密碼至少 {MIN_PASSWORD_LENGTH} 個字元")


def create_session(user_id: int) -> tuple[str, str]:
    token = secrets.token_urlsafe(32)
    expires = (datetime.now(timezone.utc).astimezone() + timedelta(hours=SESSION_HOURS))
    with transaction() as conn:
        conn.execute(
            "INSERT INTO app_session (token, user_id, created_at, expires_at) VALUES (?,?,?,?)",
            (token, user_id, now(), expires.isoformat(timespec="seconds")),
        )
    return token, expires.isoformat(timespec="seconds")


def current_user(authorization: str | None = Header(default=None)) -> dict:
    """Resolve the bearer token to a user, or 401."""
    token = (authorization or "").removeprefix("Bearer ").strip()
    if not token:
        raise HTTPException(401, "請先登入")
    with transaction() as conn:
        row = conn.execute(
            "SELECT s.expires_at, u.id, u.username, u.name, u.role, u.title, u.active, u.must_change"
            " FROM app_session s JOIN app_user u ON u.id = s.user_id WHERE s.token = ?",
            (token,),
        ).fetchone()
        if row is None:
            raise HTTPException(401, "登入已失效，請重新登入")
        if row["expires_at"] < now():
            conn.execute("DELETE FROM app_session WHERE token = ?", (token,))
            raise HTTPException(401, "登入逾時，請重新登入")
        if not row["active"]:
            raise HTTPException(403, "此帳號已停用")
    return {"id": row["id"], "username": row["username"], "name": row["name"],
            "role": row["role"], "title": row["title"], "must_change": bool(row["must_change"]),
            "permissions": sorted(PERMISSIONS.get(row["role"], set()))}


def requires(permission: str):
    """Endpoint dependency enforcing one permission.

    The message names the permission and the role, because "沒有權限" alone
    produces a support call every time. Telling someone which role can do this
    lets them go find that person instead.
    """
    def dependency(user: dict = Depends(current_user)) -> dict:
        if permission not in user["permissions"]:
            labels = role_labels()
            allowed = [labels[r] for r, perms in PERMISSIONS.items() if permission in perms]
            raise HTTPException(
                403,
                f"你的身分（{labels.get(user['role'], user['role'])}）不能做這件事。"
                f" 需要：{'、'.join(allowed)}",
            )
        return user
    return dependency
