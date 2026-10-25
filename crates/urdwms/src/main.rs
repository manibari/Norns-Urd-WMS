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
  --version             顯示版本
";

struct Args {
    data_dir: PathBuf,
    demo: bool,
    screenshot: Option<PathBuf>,
    as_user: Option<String>,
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
    let mut args = Args { data_dir: default_data_dir(), demo: false, screenshot: None, as_user: None };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data-dir" => args.data_dir = it.next().ok_or("--data-dir 需要路徑")?.into(),
            "--demo" => args.demo = true,
            "--screenshot" => args.screenshot = Some(it.next().ok_or("--screenshot 需要資料夾")?.into()),
            "--as" => args.as_user = Some(it.next().ok_or("--as 需要帳號")?),
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
    Ok(args)
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
