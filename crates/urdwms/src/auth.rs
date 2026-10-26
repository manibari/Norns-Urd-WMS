//! Identity and permissions.
//!
//! Username and password, hashed with PBKDF2-SHA256 and a per-user salt, in
//! the same `pbkdf2_sha256$iterations$salt$hash` format the Python version
//! wrote — existing accounts log in unchanged.
//!
//! Why this matters beyond gating buttons: the acceptance form's control is
//! dual sign-off. Recording the signature as the authenticated user is what
//! makes the second signature mean anything.

use rusqlite::{Connection, OptionalExtension};

use crate::bail;
use crate::error::Res;

/// A role is NOT a job title. 倉管 and 廠長 are different jobs that both need
/// the same thing from the system, so they are both `manager`. The job title
/// lives on the user and is what screens show people; the role is what the
/// permission check reads. Mixing them means every new job title needs a new
/// permission set, which is how an access model turns into a mess.
pub const ROLES: &[&str] = &["user", "manager", "admin"];

pub fn permissions(role: &str) -> &'static [&'static str] {
    match role {
        // 作業: 領用登錄與補明細
        "user" => &["issue.create", "issue.detail"],
        // 管理: 加上收貨建批、維護型號、覆核放行、看日誌
        "manager" => &["issue.create", "issue.detail", "lot.create", "item.manage", "scan.override", "audit.read"],
        // 系統: 加上批次修正與刪除、選項、人員與角色
        "admin" => &[
            "issue.create",
            "issue.detail",
            "lot.create",
            "item.manage",
            "scan.override",
            "audit.read",
            "lot.edit",
            "lot.delete",
            "dictionary.manage",
            "user.manage",
        ],
        _ => &[],
    }
}

/// Shipped defaults. Live labels come from `app_role` so each factory can use
/// its own words — 倉管 vs 資材 vs 物管 is a naming difference, not a
/// different set of permissions.
pub fn default_role_label(code: &str) -> &'static str {
    match code {
        "user" => "一般使用者",
        "manager" => "管理者",
        "admin" => "系統管理者",
        _ => "",
    }
}

pub fn role_label(conn: &Connection, code: &str) -> String {
    conn.query_row("SELECT label FROM app_role WHERE code = ?", [code], |r| r.get::<_, String>(0))
        .optional()
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            let d = default_role_label(code);
            if d.is_empty() { code.to_string() } else { d.to_string() }
        })
}

pub const MIN_PASSWORD_LENGTH: usize = 8;
const ITERATIONS: u32 = 240_000;

pub fn hash_password(password: &str) -> String {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("system random source");
    let mut digest = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, ITERATIONS, &mut digest);
    format!("pbkdf2_sha256${ITERATIONS}${}${}", hex::encode(salt), hex::encode(digest))
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    let [algorithm, iterations, salt, digest] = parts[..] else { return false };
    if algorithm != "pbkdf2_sha256" {
        return false;
    }
    let (Ok(iterations), Ok(salt), Ok(expected)) = (iterations.parse::<u32>(), hex::decode(salt), hex::decode(digest))
    else {
        return false;
    };
    let mut candidate = vec![0u8; expected.len()];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, iterations, &mut candidate);
    // Constant time: how much of the hash matched must not leak through timing.
    candidate.len() == expected.len() && candidate.iter().zip(&expected).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Length only. Composition rules push people to `Password1!` and a sticky
/// note; length is the requirement that actually correlates with strength.
pub fn check_password_policy(password: &str) -> Res<()> {
    if password.chars().count() < MIN_PASSWORD_LENGTH {
        bail!("密碼至少 {MIN_PASSWORD_LENGTH} 個字元");
    }
    Ok(())
}

/// Who is signed in on this tablet.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub id: i64,
    pub username: String,
    /// 顯示名/簽核名 — what goes into 記錄人 and the audit trail.
    pub name: String,
    pub title: Option<String>,
    pub role: String,
    pub role_label: String,
    pub must_change: bool,
}

impl Session {
    pub fn can(&self, permission: &str) -> bool {
        permissions(&self.role).contains(&permission)
    }

    /// Enforce one permission. Hiding a button is a convenience; this is the
    /// control, and every write path calls it.
    pub fn require(&self, conn: &Connection, permission: &str) -> Res<()> {
        if self.can(permission) {
            return Ok(());
        }
        let allowed: Vec<String> =
            ROLES.iter().filter(|r| permissions(r).contains(&permission)).map(|r| role_label(conn, r)).collect();
        bail!("你的身分（{}）不能做這件事。需要：{}", self.role_label, allowed.join("、"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_round_trip() {
        let stored = hash_password("urdwms2026");
        assert!(stored.starts_with("pbkdf2_sha256$240000$"));
        assert!(verify_password("urdwms2026", &stored));
        assert!(!verify_password("urdwms2027", &stored));
    }

    #[test]
    fn reads_hashes_written_by_the_python_version() {
        // hashlib.pbkdf2_hmac("sha256", b"peter0821", bytes.fromhex(salt), 1000).hex()
        let stored = "pbkdf2_sha256$1000$000102030405060708090a0b0c0d0e0f$\
                      d6725c8f7573bd7689cde4ce3bfed00a81ea29ac333f8f84653f0f015cac9edf";
        assert!(verify_password("peter0821", stored));
        assert!(!verify_password("peter0822", stored));
        assert!(!verify_password("peter0821", "md5$x$y$z"));
    }

    #[test]
    fn policy_is_length_only() {
        assert!(check_password_policy("short").is_err());
        assert!(check_password_policy("longenough").is_ok());
        assert!(check_password_policy("八個中文字也可以").is_ok());
    }

    #[test]
    fn tiers_are_supersets() {
        for p in permissions("user") {
            assert!(permissions("manager").contains(p));
        }
        for p in permissions("manager") {
            assert!(permissions("admin").contains(p));
        }
    }
}
