//! Local recovery for an administrator who cannot sign in.

use rusqlite::{OptionalExtension, params};
use serde_json::json;

use super::Store;
use crate::auth;
use crate::bail;
use crate::db;
use crate::error::{AppError, Res};

pub struct AdminReset {
    pub username: String,
    pub name: String,
    pub password: String,
}

struct Account {
    id: i64,
    username: String,
    name: String,
    role: String,
    active: bool,
}

const ACCOUNT_COLUMNS: &str = "id, username, name, role, active";

fn account_row(r: &rusqlite::Row) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get(0)?,
        username: r.get(1)?,
        name: r.get(2)?,
        role: r.get(3)?,
        active: r.get::<_, i64>(4)? != 0,
    })
}

fn temporary_password() -> Res<String> {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let limit = (256 / ALPHABET.len()) * ALPHABET.len();
    let mut password = String::with_capacity(14);
    let mut bytes = [0u8; 32];
    while password.len() < 14 {
        getrandom::fill(&mut bytes).map_err(|e| AppError(format!("無法產生臨時密碼：{e}")))?;
        for byte in bytes {
            if usize::from(byte) >= limit {
                continue;
            }
            if password.len() == 4 || password.len() == 9 {
                password.push('-');
            }
            password.push(char::from(ALPHABET[usize::from(byte) % ALPHABET.len()]));
            if password.len() == 14 {
                break;
            }
        }
    }
    Ok(password)
}

pub fn reset_admin_password(store: &Store, username: Option<&str>) -> Res<AdminReset> {
    let conn = &store.conn;
    let account = if let Some(username) = username {
        let username = username.trim();
        let row = conn
            .query_row(&format!("SELECT {ACCOUNT_COLUMNS} FROM app_user WHERE username = ?"), [username], account_row)
            .optional()?;
        let Some(account) = row else { bail!("找不到帳號「{username}」") };
        if account.role != "admin" {
            bail!("「{username}」不是系統管理者。一般人員的密碼請由管理者在「系統設定 → 人員與權限」重設。");
        }
        account
    } else {
        let mut stmt =
            conn.prepare(&format!("SELECT {ACCOUNT_COLUMNS} FROM app_user WHERE role = 'admin' ORDER BY username"))?;
        let accounts = stmt.query_map([], account_row)?.collect::<Result<Vec<_>, _>>()?;
        match accounts.as_slice() {
            [] => bail!("資料庫裡沒有系統管理者帳號"),
            [_] => accounts.into_iter().next().expect("one admin"),
            [first, ..] => {
                let names = accounts.iter().map(|a| a.username.as_str()).collect::<Vec<_>>().join("、");
                bail!("有多個系統管理者：{names}。請指定帳號，例如 Urd-WMS.exe --reset-admin {}", first.username);
            }
        }
    };
    let password = temporary_password()?;
    db::transaction(conn, |tx| {
        tx.execute(
            "UPDATE app_user SET password_hash = ?, must_change = 1, active = 1 WHERE id = ?",
            params![auth::hash_password(&password), account.id],
        )?;
        db::log(
            tx,
            "本機救援指令",
            "auth.password_reset_cli",
            json!({"username": account.username, "reactivated": !account.active}),
        )?;
        Ok(())
    })?;
    crate::applog::info(format!("以本機救援指令重設管理者密碼：{}", account.username));
    Ok(AdminReset { username: account.username, name: account.name, password })
}

#[cfg(test)]
mod tests {
    use super::super::testing::store;
    use super::*;
    use crate::service::login;

    #[test]
    fn reset_only_admin_and_force_change() {
        let store = store();
        let old_hash: String = store
            .conn
            .query_row("SELECT password_hash FROM app_user WHERE username = 'kuo'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            reset_admin_password(&store, Some("kuo")).err().unwrap().0,
            "「kuo」不是系統管理者。一般人員的密碼請由管理者在「系統設定 → 人員與權限」重設。"
        );
        assert_eq!(reset_admin_password(&store, Some("nobody")).err().unwrap().0, "找不到帳號「nobody」");
        let new_hash: String = store
            .conn
            .query_row("SELECT password_hash FROM app_user WHERE username = 'kuo'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(old_hash, new_hash);

        let first = reset_admin_password(&store, None).unwrap();
        assert_eq!(first.username, "peter");
        assert_eq!(first.password.len(), 14);
        assert!(first.password.bytes().enumerate().all(|(i, b)| if i == 4 || i == 9 {
            b == b'-'
        } else {
            b"abcdefghjkmnpqrstuvwxyz23456789".contains(&b)
        }));
        assert!(login(&store, "peter", "peter0821").is_err());
        assert!(login(&store, "peter", &first.password).unwrap().must_change);
        let detail: String = store
            .conn
            .query_row("SELECT detail FROM audit_log WHERE action = 'auth.password_reset_cli'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&detail).unwrap(),
            json!({"username": "peter", "reactivated": false})
        );
        assert!(!detail.contains(&first.password));
        assert!(!detail.contains("pbkdf2"));

        let second = reset_admin_password(&store, None).unwrap();
        assert_ne!(first.password, second.password);
    }

    #[test]
    fn multiple_admins_require_a_username() {
        let store = store();
        store.conn.execute("UPDATE app_user SET role = 'admin' WHERE username = 'kuo'", []).unwrap();
        assert_eq!(
            reset_admin_password(&store, None).err().unwrap().0,
            "有多個系統管理者：kuo、peter。請指定帳號，例如 Urd-WMS.exe --reset-admin kuo"
        );
        assert_eq!(reset_admin_password(&store, Some(" peter ")).unwrap().username, "peter");
    }

    #[test]
    fn inactive_admin_is_reactivated() {
        let store = store();
        store.conn.execute("UPDATE app_user SET active = 0 WHERE username = 'peter'", []).unwrap();
        let reset = reset_admin_password(&store, None).unwrap();
        assert!(login(&store, "peter", &reset.password).unwrap().must_change);
        let detail: String = store
            .conn
            .query_row("SELECT detail FROM audit_log WHERE action = 'auth.password_reset_cli'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&detail).unwrap()["reactivated"], true);
    }
}
