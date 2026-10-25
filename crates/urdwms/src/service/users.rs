//! Accounts, sign-in and role labels.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use super::Store;
use crate::auth::{self, ROLES, Session, check_password_policy, hash_password, role_label, verify_password};
use crate::bail;
use crate::db::{self, log, now};
use crate::error::Res;

pub(crate) fn session_for(conn: &Connection, username: &str) -> Res<Session> {
    let row = conn
        .query_row(
            "SELECT id, username, name, title, role, must_change FROM app_user WHERE username = ? AND active = 1",
            [username],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)? != 0,
                ))
            },
        )
        .optional()?;
    let Some((id, username, name, title, role, must_change)) = row else { bail!("帳號或密碼不正確") };
    let role_label = role_label(conn, &role);
    Ok(Session { id, username, name, title, role, role_label, must_change })
}

/// One message for both failures. Distinguishing "no such account" from
/// "wrong password" tells an attacker which usernames are worth attacking.
pub fn login(store: &Store, username: &str, password: &str) -> Res<Session> {
    let conn = &store.conn;
    let username = username.trim().to_lowercase();
    let hash: Option<String> = conn
        .query_row("SELECT password_hash FROM app_user WHERE username = ? AND active = 1", [&username], |r| r.get(0))
        .optional()?;
    match hash {
        Some(h) if verify_password(password, &h) => {}
        _ => bail!("帳號或密碼不正確"),
    }
    let session = session_for(conn, &username)?;
    log(conn, &session.name, "auth.login", json!({"role": session.role, "username": session.username}))?;
    Ok(session)
}

pub fn logout(store: &Store, user: &Session) {
    let _ = log(&store.conn, &user.name, "auth.logout", json!({"username": user.username}));
}

/// Re-read the signed-in account: a role change or deactivation by an admin
/// takes effect on the next screen, not at the next login.
pub fn refresh_session(store: &Store, user: &Session) -> Option<Session> {
    session_for(&store.conn, &user.username).ok()
}

/// Change your own password. Requires the current one — otherwise a borrowed
/// tablet becomes a permanent takeover.
pub fn change_password(store: &Store, user: &Session, current: &str, new: &str) -> Res<()> {
    check_password_policy(new)?;
    let conn = &store.conn;
    let hash: String = conn.query_row("SELECT password_hash FROM app_user WHERE id = ?", [user.id], |r| r.get(0))?;
    if !verify_password(current, &hash) {
        bail!("目前密碼不正確");
    }
    if current == new {
        bail!("新密碼不能跟目前的一樣");
    }
    db::transaction(conn, |tx| {
        tx.execute(
            "UPDATE app_user SET password_hash = ?, must_change = 0 WHERE id = ?",
            params![hash_password(new), user.id],
        )?;
        log(tx, &user.name, "auth.password_change", json!({"user_id": user.id}))?;
        Ok(())
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub name: String,
    pub title: Option<String>,
    pub role: String,
    pub role_label: String,
    pub active: bool,
    pub must_change: bool,
}

pub fn users(store: &Store, user: &Session) -> Res<Vec<UserRow>> {
    let conn = &store.conn;
    user.require(conn, "user.manage")?;
    let mut stmt =
        conn.prepare("SELECT id, username, name, title, role, active, must_change FROM app_user ORDER BY role, name")?;
    let rows = stmt
        .query_map([], |r| {
            let role: String = r.get(4)?;
            Ok(UserRow {
                id: r.get(0)?,
                username: r.get(1)?,
                name: r.get(2)?,
                title: r.get(3)?,
                role_label: role_label(conn, &role),
                role,
                active: r.get::<_, i64>(5)? != 0,
                must_change: r.get::<_, i64>(6)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Names that can appear as 確認人. Shows the job title, which is what people
/// recognise each other by; the role means nothing to whoever is signing.
pub fn signers(conn: &Connection) -> Res<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT name, title, role FROM app_user WHERE active = 1 ORDER BY role, name")?;
    let rows = stmt
        .query_map([], |r| {
            let title: Option<String> = r.get(1)?;
            let role: String = r.get(2)?;
            Ok((r.get::<_, String>(0)?, title.unwrap_or_else(|| role_label(conn, &role))))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub struct NewUser {
    pub username: String,
    pub name: String,
    pub title: Option<String>,
    pub role: String,
    pub password: String,
}

/// Created with a password someone else chose, so it must be changed on first
/// login — otherwise the admin knows everyone's password forever.
pub fn create_user(store: &Store, user: &Session, new: &NewUser) -> Res<i64> {
    let conn = &store.conn;
    user.require(conn, "user.manage")?;
    if !ROLES.contains(&new.role.as_str()) {
        bail!("未知的角色 {}", new.role);
    }
    check_password_policy(&new.password)?;
    let username = new.username.trim().to_lowercase();
    if username.is_empty() {
        bail!("帳號不可空白");
    }
    if new.name.trim().is_empty() {
        bail!("顯示名不可空白（它會寫進紀錄人與稽核軌跡）");
    }
    db::transaction(conn, |tx| {
        if tx.query_row("SELECT 1 FROM app_user WHERE username = ?", [&username], |_| Ok(())).optional()?.is_some() {
            bail!("帳號「{username}」已存在");
        }
        tx.execute(
            "INSERT INTO app_user (username, name, title, role, password_hash, must_change, created_at) VALUES (?,?,?,?,?,1,?)",
            params![
                username,
                new.name.trim(),
                new.title.as_deref().map(str::trim).filter(|t| !t.is_empty()),
                new.role,
                hash_password(&new.password),
                now()
            ],
        )?;
        let id = tx.last_insert_rowid();
        log(tx, &user.name, "user.create", json!({"username": username, "role": new.role}))?;
        Ok(id)
    })
}

#[derive(Debug, Clone, Default)]
pub struct UserPatch {
    pub name: Option<String>,
    pub title: Option<String>,
    pub role: Option<String>,
    pub password: Option<String>,
    pub active: Option<bool>,
}

pub fn update_user(store: &Store, user: &Session, id: i64, patch: &UserPatch) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "user.manage")?;
    use rusqlite::types::Value as V;
    let mut changes: Vec<(&str, V)> = Vec::new();
    if let Some(role) = &patch.role {
        if !ROLES.contains(&role.as_str()) {
            bail!("未知的角色 {role}");
        }
        changes.push(("role", V::Text(role.clone())));
    }
    if let Some(name) = patch.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        changes.push(("name", V::Text(name.into())));
    }
    if let Some(title) = &patch.title {
        let t = title.trim();
        changes.push(("title", if t.is_empty() { V::Null } else { V::Text(t.into()) }));
    }
    if let Some(pw) = &patch.password {
        check_password_policy(pw)?;
        changes.push(("password_hash", V::Text(hash_password(pw))));
        changes.push(("must_change", V::Integer(1)));
    }
    if let Some(active) = patch.active {
        changes.push(("active", V::Integer(active as i64)));
    }
    if changes.is_empty() {
        bail!("沒有要更新的欄位");
    }
    db::transaction(conn, |tx| {
        let role: Option<String> =
            tx.query_row("SELECT role FROM app_user WHERE id = ?", [id], |r| r.get(0)).optional()?;
        let Some(role) = role else { bail!("使用者不存在") };
        // Locking out the last admin leaves nobody able to unlock anyone.
        let demoted = patch.role.as_deref().is_some_and(|r| r != "admin");
        if role == "admin" && (demoted || patch.active == Some(false)) {
            let remaining: i64 = tx.query_row(
                "SELECT COUNT(*) FROM app_user WHERE role = 'admin' AND active = 1 AND id <> ?",
                [id],
                |r| r.get(0),
            )?;
            if remaining == 0 {
                bail!("這是最後一個啟用中的系統管理者，不能停用或降級");
            }
        }
        let set: Vec<String> = changes.iter().map(|(k, _)| format!("{k} = ?")).collect();
        let mut values: Vec<V> = changes.iter().map(|(_, v)| v.clone()).collect();
        values.push(V::Integer(id));
        tx.execute(
            &format!("UPDATE app_user SET {} WHERE id = ?", set.join(", ")),
            rusqlite::params_from_iter(values),
        )?;
        let shown: serde_json::Map<String, serde_json::Value> = changes
            .iter()
            .map(|(k, v)| {
                (k.to_string(), if *k == "password_hash" { json!("***") } else { super::items::sql_to_json(v) })
            })
            .collect();
        log(tx, &user.name, "user.update", json!({"user_id": id, "changed": shown}))?;
        Ok(())
    })
}

/// Rename a role. What a role may do is system design; what it is called is
/// the factory's words.
pub fn rename_role(store: &Store, user: &Session, code: &str, label: &str) -> Res<()> {
    let conn = &store.conn;
    user.require(conn, "user.manage")?;
    if !ROLES.contains(&code) {
        bail!("未知的角色 {code}");
    }
    let label = label.trim();
    if label.is_empty() {
        bail!("名稱不可空白");
    }
    conn.execute(
        "INSERT INTO app_role (code, label) VALUES (?, ?) ON CONFLICT(code) DO UPDATE SET label = excluded.label",
        params![code, label],
    )?;
    log(conn, &user.name, "role.rename", json!({"code": code, "label": label}))?;
    Ok(())
}

pub fn roles(conn: &Connection) -> Vec<(String, String)> {
    ROLES.iter().map(|r| (r.to_string(), role_label(conn, r))).collect()
}

pub fn permission_label(p: &str) -> &'static str {
    match p {
        "issue.create" => "領用登錄",
        "issue.detail" => "補明細",
        "lot.create" => "收貨建批",
        "item.manage" => "維護型號",
        "scan.override" => "覆核放行",
        "audit.read" => "看系統日誌",
        "lot.edit" => "批次修正",
        "lot.delete" => "批次刪除",
        "dictionary.manage" => "選項與系統設定",
        "user.manage" => "人員與角色",
        _ => "",
    }
}

pub fn permission_summary(role: &str) -> String {
    auth::permissions(role).iter().map(|p| permission_label(p)).collect::<Vec<_>>().join("、")
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    #[test]
    fn login_uses_one_message_for_both_failures() {
        let s = store();
        assert_eq!(login(&s, "nobody", "x").unwrap_err(), login(&s, "kuo", "wrong").unwrap_err());
        assert_eq!(login(&s, " KUO ", "urdwms2026").unwrap().name, "郭");
    }

    #[test]
    fn new_accounts_must_change_their_password() {
        let s = store();
        let admin = session(&s, "peter");
        let new = NewUser {
            username: "qc".into(),
            name: "品管".into(),
            title: None,
            role: "user".into(),
            password: "temporary1".into(),
        };
        create_user(&s, &admin, &new).unwrap();
        let first = login(&s, "qc", "temporary1").unwrap();
        assert!(first.must_change);
        change_password(&s, &first, "temporary1", "mine-only-now").unwrap();
        assert!(!login(&s, "qc", "mine-only-now").unwrap().must_change);
    }

    #[test]
    fn the_last_admin_cannot_be_locked_out() {
        let s = store();
        let admin = session(&s, "peter");
        let err =
            update_user(&s, &admin, admin.id, &UserPatch { active: Some(false), ..Default::default() }).unwrap_err();
        assert!(err.0.contains("最後一個"));
    }
}
