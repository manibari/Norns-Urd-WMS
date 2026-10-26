//! One error type for everything a screen can be told.
//!
//! The message is written for whoever is standing at the tablet: what went
//! wrong and what to do about it. "沒有權限" alone produces a phone call every
//! time; naming the role that can do it lets them go find that person.

#[derive(Debug, Clone, PartialEq)]
pub struct AppError(pub String);

pub type Res<T> = Result<T, AppError>;

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AppError {}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError(format!("資料庫錯誤：{e}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError(format!("檔案錯誤：{e}"))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError(format!("資料格式錯誤：{e}"))
    }
}

/// `bail!("…")` — return a user-facing error.
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => { return Err($crate::error::AppError(format!($($arg)*))) };
}
