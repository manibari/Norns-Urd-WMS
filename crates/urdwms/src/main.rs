//! Urd-WMS — packaging lot management for one plant, as one Windows program.
//!
//! Receiving, image-recognised issuing with FIFO, traceability and alerts, on
//! a local SQLite file. No server, no browser, no runtime to install.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod applog;
mod auth;
mod db;
mod error;
mod seed;
mod service;
mod ui;

use std::path::PathBuf;

const HELP: &str = "\
Urd-WMS 包材批次管理

用法: urdwms [選項]

  --data-dir <路徑>     資料位置（預設 %LOCALAPPDATA%\\UrdWMS）
  --demo                空資料庫時載入示範資料（品項、批次、四個示範帳號）
  --screenshot <資料夾> 依序擷取每個畫面存成 PNG 後結束（搭配 --as <帳號>）
  --as <帳號>           以此帳號直接登入（只限 --screenshot）
  --reset-admin [帳號]  忘記管理者密碼時使用：改成一組臨時密碼，登入後要改掉
  --version             顯示版本
";

struct Args {
    data_dir: PathBuf,
    demo: bool,
    screenshot: Option<PathBuf>,
    as_user: Option<String>,
    reset_admin: Option<Option<String>>,
}

fn default_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("URDWMS_DATA") {
        return dir.into();
    }
    #[cfg(windows)]
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("UrdWMS");
    }
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".local/share/urdwms"))
        .unwrap_or_else(|_| PathBuf::from("urdwms-data"))
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

fn optional_username(it: &mut std::iter::Peekable<impl Iterator<Item = String>>) -> Option<String> {
    if it.peek().is_some_and(|next| !next.starts_with("--")) { it.next() } else { None }
}

fn parse_args_from(input: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args =
        Args { data_dir: default_data_dir(), demo: false, screenshot: None, as_user: None, reset_admin: None };
    let mut it = input.into_iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data-dir" => args.data_dir = it.next().ok_or("--data-dir 需要路徑")?.into(),
            "--demo" => args.demo = true,
            "--screenshot" => args.screenshot = Some(it.next().ok_or("--screenshot 需要資料夾")?.into()),
            "--as" => args.as_user = Some(it.next().ok_or("--as 需要帳號")?),
            "--reset-admin" => args.reset_admin = Some(optional_username(&mut it)),
            "--version" => {
                println!("urdwms {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => return Err(format!("不認得的參數：{other}\n\n{HELP}")),
        }
    }
    if args.reset_admin.is_some() && (args.demo || args.screenshot.is_some() || args.as_user.is_some()) {
        return Err("--reset-admin 不能和 --demo、--screenshot、--as 一起用".into());
    }
    Ok(args)
}

fn check_reset_database(data_dir: &std::path::Path) -> Result<(), String> {
    let path = data_dir.join("urdwms.db");
    if path.exists() {
        Ok(())
    } else {
        Err(format!("找不到資料庫：{}。資料若放在別的位置，請加 --data-dir <路徑>。", path.display()))
    }
}

/// A crash on the floor should leave something behind to read.
fn install_panic_log(dir: PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let line = format!("[{}] {info}\n{}\n", db::now(), std::backtrace::Backtrace::force_capture());
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("crash.log"))
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        applog::error(format!("程式異常：{info}"));
        default(info);
    }));
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    #[cfg(windows)]
    {
        rfd::MessageDialog::new()
            .set_title("Urd-WMS")
            .set_description(message)
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    std::process::exit(1);
}

fn window_icon() -> egui::IconData {
    // A blue rounded square with a white box outline — drawn rather than
    // shipped, so the exe carries no asset files.
    let size = 64usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32, y as f32);
            let corner = 12.0;
            let dx = (corner - fx).max(fx - (size as f32 - 1.0 - corner)).max(0.0);
            let dy = (corner - fy).max(fy - (size as f32 - 1.0 - corner)).max(0.0);
            let inside = dx * dx + dy * dy <= corner * corner;
            let box_edge = (18..=46).contains(&x)
                && (20..=48).contains(&y)
                && (!(22..=42).contains(&x) || !(24..=44).contains(&y) || y == 32 || y == 33);
            let i = (y * size + x) * 4;
            if inside {
                let (r, g, b) = if box_edge { (255, 255, 255) } else { (0x16, 0x77, 0xff) };
                rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    }
    egui::IconData { rgba, width: size as u32, height: size as u32 }
}

fn main() {
    let args = parse_args().unwrap_or_else(|e| fail(&e));
    if let Some(username) = &args.reset_admin {
        check_reset_database(&args.data_dir).unwrap_or_else(|e| fail(&e));
        applog::init(&args.data_dir);
        let store = service::Store::open(&args.data_dir).unwrap_or_else(|e| fail(&format!("無法開啟資料庫：{e}")));
        let reset = service::reset_admin_password(&store, username.as_deref()).unwrap_or_else(|e| fail(&e.0));
        let message = format!(
            "已重設「{}」（帳號 {}）的密碼。\n\n臨時密碼：{}\n\n請用這組密碼登入，系統會要求立刻改成只有你知道的密碼。",
            reset.name, reset.username, reset.password
        );
        println!("{message}");
        #[cfg(windows)]
        rfd::MessageDialog::new()
            .set_title("Urd-WMS")
            .set_description(&message)
            .set_level(rfd::MessageLevel::Info)
            .show();
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&args.data_dir) {
        fail(&format!("無法建立資料夾 {}：{e}", args.data_dir.display()));
    }
    install_panic_log(args.data_dir.clone());
    applog::init(&args.data_dir);
    applog::info(format!(
        "啟動 urdwms {}（{} {}），資料位置 {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        args.data_dir.display()
    ));
    let store = service::Store::open(&args.data_dir).unwrap_or_else(|e| fail(&format!("無法開啟資料庫：{e}")));
    if args.demo {
        match seed::demo(&store.conn) {
            Ok(true) => eprintln!(
                "已載入示範資料。帳號：peter/peter0821（系統管理者）、kuo/urdwms2026（倉管）、operator/urdwms2026（作業員）"
            ),
            Ok(false) => eprintln!("資料庫已有批次，略過示範資料"),
            Err(e) => fail(&format!("示範資料載入失敗：{e}")),
        }
    }
    let first_run = seed::first_run(&store.conn).unwrap_or_else(|e| fail(&format!("初始化失敗：{e}")));
    let auto_login = match (&args.screenshot, &args.as_user) {
        (Some(_), Some(name)) => Some(service::session_for(&store.conn, name).unwrap_or_else(|e| fail(&e.0))),
        _ => None,
    };
    let screenshots = args.screenshot.clone().map(|dir| {
        let _ = std::fs::create_dir_all(&dir);
        ui::ScreenshotPlan::new(dir)
    });

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Urd-WMS 包材批次管理")
        .with_inner_size([1366.0, 900.0])
        .with_min_inner_size([960.0, 640.0])
        .with_icon(window_icon());
    if screenshots.is_none() {
        viewport = viewport.with_maximized(true);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let data_dir = args.data_dir.clone();
    let result = eframe::run_native(
        "Urd-WMS",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, store, data_dir, first_run, auto_login, screenshots)))),
    );
    if let Err(e) = result {
        applog::error(format!("無法啟動畫面：{e}"));
        fail(&format!("無法啟動畫面（顯示卡驅動不支援 OpenGL 2.0？）：{e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_admin_argument_order_and_conflict() {
        for (input, username, data_dir) in [
            (vec!["--reset-admin"], None, None),
            (vec!["--reset-admin", "peter"], Some("peter"), None),
            (vec!["--reset-admin", "--data-dir", "X"], None, Some("X")),
            (vec!["--data-dir", "X", "--reset-admin", "peter"], Some("peter"), Some("X")),
        ] {
            let args = parse_args_from(input.into_iter().map(str::to_string)).unwrap();
            assert_eq!(args.reset_admin.as_ref().and_then(Option::as_deref), username);
            assert!(args.reset_admin.is_some());
            if let Some(dir) = data_dir {
                assert_eq!(args.data_dir, PathBuf::from(dir));
            }
        }
        assert_eq!(
            parse_args_from(["--reset-admin", "--demo"].into_iter().map(str::to_string)).err().unwrap(),
            "--reset-admin 不能和 --demo、--screenshot、--as 一起用"
        );
    }

    #[test]
    fn missing_reset_database_does_not_create_directory() {
        let dir = std::env::temp_dir().join(format!("urdwms-reset-missing-{}", std::process::id()));
        let nested = dir.join("absent");
        let error = check_reset_database(&nested).unwrap_err();
        assert_eq!(
            error,
            format!(
                "找不到資料庫：{}。資料若放在別的位置，請加 --data-dir <路徑>。",
                nested.join("urdwms.db").display()
            )
        );
        assert!(!nested.exists());
    }
}
