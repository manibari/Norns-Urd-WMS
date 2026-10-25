//! 領用登錄 — photo first.
//!
//! The operator walks up with a box already in their hands. They know what it
//! is; the system is the one that needs to find out. So the screen opens on a
//! camera, not a dropdown, and the first thing it says back is a verdict:
//! 可以拿 / 先別拿. Asking "which item?" first would make the person do the
//! identification and narrow the candidate set by hand — the one thing §2.2
//! says the machine must not be helped with.
//!
//! The manual path is always one tap away and is entered automatically when
//! recognition cannot resolve the box (§2.3 辨識可斷，系統不可斷).
//!
//! Copy on a blocked draw is "已記錄，請換一箱", never "失敗": a failure message
//! invites re-shooting until it passes; this one says the record already
//! exists (§2.1).
//!
//! Declared override of the house density: 64px touch targets and oversized
//! type, because this is used with gloves on, standing, possibly backlit. The
//! verdict is a full colour block, not a toast.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Color32, CornerRadius, Frame, Margin, RichText, ScrollArea, Ui, Vec2};
use serde_json::{Map, Value, json};
use urdwms_core::camera::{self, CameraConfig, CameraError, SourceInfo};

use super::theme::{BAD, MUTED, OK, PRIMARY, TOUCH, WARN, bold_font, font};
use super::widgets::*;
use super::{Nav, Page, Shared, Task};
use crate::auth::Session;
use crate::error::AppError;
use crate::service::{self, Item, ItemOrder, Lot, Proposal, ScanInput, ScanOutcome, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Stage {
    #[default]
    Shoot,
    /// Folder source: waiting for the camera to save a newer still.
    Waiting,
    Verdict,
    Manual,
}

/// What came back from the worker: the proposal plus the decoded photo, so
/// the verdict screen shows the picture at once.
type CaptureResult = Result<(Proposal, Option<egui::ColorImage>), CaptureFailure>;

enum CaptureFailure {
    /// Not a fault — there is no current photo yet. Keep waiting.
    Stale,
    Failed(String),
}

#[derive(Default)]
pub struct IssuePage {
    loaded: Option<u64>,
    items: Vec<Item>,
    machines: Vec<String>,
    products: Vec<String>,
    camera: CameraConfig,
    stage: Stage,
    item_id: Option<i64>,
    lots: Vec<Lot>,
    lot_id: Option<i64>,
    qty: i64,
    capture: Option<Proposal>,
    task: Task<CaptureResult>,
    trigger: Task<Result<Option<String>, String>>,
    camera_reply: Option<String>,
    /// When the operator declared "the next photo is this box".
    wait_since: Option<std::time::SystemTime>,
    last_seen: Option<SourceInfo>,
    last_poll: Option<Instant>,
    machine: Option<String>,
    product: Option<String>,
    steps: String,
    show_extra: bool,
    outcome: Option<ScanOutcome>,
    error: Option<String>,
}

impl IssuePage {
    fn folder_source(&self) -> bool {
        self.camera.enabled && self.camera.transport == "folder" && self.camera.configured()
    }

    /// Folder source with a TCP command port: the trigger goes over TCP, the
    /// photo lands in the folder.
    fn folder_trigger(&self) -> bool {
        self.folder_source() && !self.camera.host.is_empty() && !self.camera.trigger.is_empty()
    }

    fn fire_trigger(&mut self, ctx: &egui::Context) {
        self.start_waiting();
        let config = self.camera.clone();
        self.trigger = Task::spawn(ctx, move || camera::send_trigger(&config).map_err(|e| e.to_string()));
    }

    fn network_source(&self) -> bool {
        self.camera.enabled && self.camera.transport != "folder" && self.camera.configured()
    }

    fn load(&mut self, shared: &mut Shared) {
        let conn = &shared.store.conn;
        self.items = service::items(conn, ItemOrder::Sort).unwrap_or_default();
        self.machines = service::values(conn, "machine");
        self.products = service::values(conn, "packed_product");
        self.camera = service::camera_config(conn).unwrap_or_default();
        if let Some(id) = self.item_id {
            self.reload_lots(shared, id, self.lot_id);
        }
        self.loaded = Some(shared.generation);
    }

    /// Default to what FIFO wants: the common case is one tap, and choosing
    /// otherwise is a deliberate act.
    fn reload_lots(&mut self, shared: &Shared, item: i64, prefer: Option<i64>) {
        self.lots = service::lots(&shared.store.conn, Some(item))
            .unwrap_or_default()
            .into_iter()
            .filter(|l| l.drawable())
            .collect();
        let keep = prefer.filter(|p| self.lots.iter().any(|l| l.id == *p));
        self.lot_id = keep
            .or_else(|| self.lots.iter().find(|l| l.is_fifo_next).map(|l| l.id))
            .or(self.lots.first().map(|l| l.id));
    }

    /// Arriving from 庫存總覽's 領用 link names the item — honour it rather
    /// than demanding a photo.
    pub fn start_manual(&mut self, shared: &Shared, item: i64) {
        self.reset();
        self.stage = Stage::Manual;
        self.item_id = Some(item);
        self.reload_lots(shared, item, None);
    }

    /// Screenshot runs: show the verdict for the sample box photo with a
    /// scripted reading, exactly as a live recognition would produce it.
    pub fn demo(&mut self, ctx: &egui::Context, shared: &mut Shared, manufacture: &str) {
        self.reset();
        self.loaded = None;
        let bytes = include_bytes!("../../../../docs/requirements/assets/packaging-material-fifo/IMG_2928.jpg");
        let reading = urdwms_core::recognition::Recognition {
            model_code: Some("T7320BC".into()),
            item_code: Some("2003.T7320BC-340X900-P1".into()),
            manufacture_date: Some(manufacture.into()),
            manufacture_date_confidence: 0.95,
            notes: "示範：固定讀值".into(),
            ..Default::default()
        };
        let provider = urdwms_core::recognition::Scripted(reading);
        match service::recognise_and_store(&shared.store, &provider, bytes, "-demo") {
            Ok(p) => {
                if let (Ok(img), Some(path)) = (super::images::decode(bytes), p.image_path.as_deref()) {
                    let abs = shared.store.resolve_image(path);
                    shared.images.insert(ctx, &abs, img);
                }
                self.item_id = p.item_id;
                if let Some(id) = p.item_id {
                    let locked = p.locked_lot.as_ref().map(|l| l.lot_id);
                    self.reload_lots(shared, id, locked);
                }
                self.stage = if p.item_id.is_some() { Stage::Verdict } else { Stage::Manual };
                self.capture = Some(p);
            }
            Err(e) => shared.error(e.0),
        }
    }

    fn reset(&mut self) {
        self.task.cancel();
        self.capture = None;
        self.outcome = None;
        self.qty = 1;
        self.wait_since = None;
        self.last_seen = None;
        self.item_id = None;
        self.lots.clear();
        self.lot_id = None;
        self.machine = None;
        self.product = None;
        self.steps.clear();
        self.show_extra = false;
        self.error = None;
        self.camera_reply = None;
        self.stage = Stage::Shoot;
    }

    fn start_waiting(&mut self) {
        self.capture = None;
        self.last_seen = None;
        self.wait_since = Some(std::time::SystemTime::now());
        self.stage = Stage::Waiting;
    }

    fn run_capture(&mut self, ctx: &egui::Context, shared: &Shared, source: Source) {
        let data_dir = shared.data_dir.clone();
        let config = self.camera.clone();
        self.error = None;
        self.task = Task::spawn(ctx, move || capture_and_recognise(&data_dir, &config, source));
    }

    fn poll(&mut self, ui: &Ui, shared: &mut Shared) {
        if let Some(result) = self.task.poll() {
            match result {
                Ok((proposal, image)) => {
                    if let (Some(img), Some(path)) = (image, proposal.image_path.as_deref()) {
                        let abs = shared.store.resolve_image(path);
                        shared.images.insert(ui.ctx(), &abs, img);
                    }
                    if let Some(e) = &proposal.recognition.error {
                        shared.warn(format!("辨識沒有結果：{e}"));
                    }
                    match proposal.item_id {
                        // Recognition may fail; the system may not (§2.3).
                        None => {
                            self.item_id = None;
                            self.lots.clear();
                            self.stage = Stage::Manual;
                        }
                        Some(id) => {
                            self.item_id = Some(id);
                            let locked = proposal.locked_lot.as_ref().map(|l| l.lot_id);
                            self.reload_lots(shared, id, locked);
                            self.stage = Stage::Verdict;
                        }
                    }
                    self.capture = Some(proposal);
                }
                // A stale photo is not a fault, it is "not taken yet" — go back
                // to waiting rather than kicking the operator into manual.
                Err(CaptureFailure::Stale) => self.stage = Stage::Waiting,
                Err(CaptureFailure::Failed(e)) => {
                    shared.error(format!("{e}　已切到手動登錄"));
                    self.stage = Stage::Manual;
                }
            }
        }

        if let Some(result) = self.trigger.poll() {
            match result {
                Ok(reply) => self.camera_reply = reply,
                Err(e) => shared.error(format!("觸發相機失敗：{e}")),
            }
        }

        // Waiting on a camera folder: is the newest still newer than the moment
        // the operator said "the next photo is this box"? Not "within five
        // minutes" — that could be the previous box's photo.
        if self.stage == Stage::Waiting && !self.task.running() {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
            if self.last_poll.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
                self.last_poll = Some(Instant::now());
                if let Ok((_, info)) = camera::newest_image(&self.camera) {
                    let fresh = self.wait_since.is_some_and(|since| {
                        let age = Duration::from_secs_f64(info.age_seconds);
                        std::time::SystemTime::now().checked_sub(age).is_some_and(|saved| saved > since)
                    });
                    self.last_seen = Some(info);
                    if fresh {
                        self.run_capture(ui.ctx(), shared, Source::Camera);
                    }
                }
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        if self.loaded != Some(shared.generation) {
            self.load(shared);
        }
        if self.qty < 1 {
            self.qty = 1;
        }
        self.poll(ui, shared);
        // A folder source arms itself: the operator walks to the camera with
        // the box and the result is on screen when they turn round.
        if self.folder_source() && self.stage == Stage::Shoot && !self.task.running() {
            self.start_waiting();
        }

        if let Some(outcome) = self.outcome.clone() {
            return self.outcome_block(ui, &outcome);
        }

        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(1100.0);
            page_title(ui, "領用登錄", None);
            if self.task.running() {
                return busy(ui, self.task.started);
            }
            match self.stage {
                Stage::Shoot => self.shoot(ui, shared),
                Stage::Waiting => self.waiting(ui, shared),
                Stage::Verdict => self.verdict(ui, shared),
                Stage::Manual => self.manual(ui, shared),
            }
            if self.stage != Stage::Shoot && self.stage != Stage::Waiting {
                self.lots_and_submit(ui, shared, user);
            }
        });
    }

    fn file_button(&mut self, ui: &mut Ui, shared: &Shared, label: &str, height: f32, size: f32) {
        if touch(ui, label, Color32::WHITE, super::theme::TEXT, height, size, true).clicked()
            && let Some(path) = pick_image()
        {
            self.run_capture(ui.ctx(), shared, Source::File(path));
        }
    }

    fn shoot(&mut self, ui: &mut Ui, shared: &mut Shared) {
        card(ui, |ui| {
            ui.label(RichText::new("把箱子放好，拍側面標籤 —— 要看得到型號和生產日期").font(bold_font(22.0)));
            ui.label(
                RichText::new("不用先選要領什麼 —— 拍完系統會告訴你這箱能不能拿出去。").font(font(16.0)).color(MUTED),
            );
            ui.add_space(16.0);
            if self.network_source() {
                if touch(ui, "📷 拍這一箱", PRIMARY, Color32::WHITE, 96.0, 26.0, true).clicked() {
                    self.run_capture(ui.ctx(), shared, Source::Camera);
                }
                ui.add_space(6.0);
                self.file_button(ui, shared, "🖼 改用照片檔", TOUCH, 18.0);
            } else {
                if self.camera.configured() && !self.camera.enabled {
                    alert_with(
                        ui,
                        Kind::Warning,
                        "影像來源已設定，但沒有啟用",
                        &format!(
                            "目前設定的是 {}。到系統設定把「啟用」打開，這裡就會自動等相機的照片。",
                            self.camera.endpoint()
                        ),
                        |ui| {
                            if link(ui, "去系統設定 →").clicked() {
                                shared.nav = Some(Nav::Go(Page::Settings));
                            }
                        },
                    );
                    ui.add_space(10.0);
                }
                // No fixed camera configured: a photo file is the capture path
                // (the tablet's own camera app saves to a folder, which can
                // also be set as a folder source in 系統設定).
                if touch(ui, "🖼 選一張箱子的照片", PRIMARY, Color32::WHITE, 96.0, 24.0, true).clicked()
                    && let Some(path) = pick_image()
                {
                    self.run_capture(ui.ctx(), shared, Source::File(path));
                }
                if !self.camera.configured() {
                    ui.label(small(
                        "尚未設定固定相機。可到「系統設定 → 影像設定」接一台網路相機，或指定相機存檔的資料夾。",
                    ));
                }
            }
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                if link(ui, "相機壞了 / 這箱沒有標籤 → 手動登錄").clicked() {
                    self.stage = Stage::Manual;
                }
            });
        });
    }

    fn waiting(&mut self, ui: &mut Ui, shared: &mut Shared) {
        card(ui, |ui| {
            // The photo area stays blank — an old photo here is the most
            // dangerous thing on the screen: it looks like the one just taken.
            Frame::new()
                .fill(Color32::from_rgb(0xfa, 0xfa, 0xfa))
                .stroke(egui::Stroke::new(2.0, Color32::from_rgb(0xd9, 0xd9, 0xd9)))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(40))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        ui.add(egui::Spinner::new().size(40.0));
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(if self.folder_trigger() {
                                "拍好了就等照片進來"
                            } else {
                                "去拍這一箱"
                            })
                            .font(bold_font(28.0)),
                        );
                        ui.label(
                            RichText::new("拍完存進資料夾就會自動開始辨識，不用回來按任何東西。")
                                .font(font(16.0))
                                .color(MUTED),
                        );
                    });
                });
            ui.add_space(12.0);
            if self.folder_trigger() {
                let label = if self.trigger.running() { "觸發中…" } else { "📷 拍這一箱" };
                if touch(ui, label, PRIMARY, Color32::WHITE, 96.0, 26.0, !self.trigger.running()).clicked() {
                    self.fire_trigger(ui.ctx());
                }
                if let Some(reply) = &self.camera_reply {
                    ui.label(small(format!("相機回覆：{reply}")));
                }
                ui.add_space(8.0);
            }
            if let Some(info) = self.last_seen.clone().filter(|i| i.age_seconds <= camera::STALE_AFTER)
                && touch(
                    ui,
                    format!("用資料夾最新那張（{}，{:.0} 秒前）", info.name, info.age_seconds),
                    Color32::WHITE,
                    super::theme::TEXT,
                    TOUCH,
                    18.0,
                    true,
                )
                .clicked()
            {
                self.run_capture(ui.ctx(), shared, Source::Camera);
            }
            let seen = match &self.last_seen {
                Some(info) => format!(
                    "資料夾最新的是 {} 那張（{}），在等更新的。",
                    info.time.get(11..19).unwrap_or(&info.time),
                    info.name
                ),
                None => "正在看資料夾…".into(),
            };
            ui.vertical_centered(|ui| ui.label(muted(seen)));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 12.0) / 2.0;
                ui.allocate_ui(Vec2::new(w, TOUCH), |ui| {
                    if touch(ui, "改手動登錄", Color32::WHITE, super::theme::TEXT, TOUCH, 18.0, true).clicked() {
                        self.wait_since = None;
                        self.stage = Stage::Manual;
                    }
                });
                ui.allocate_ui(Vec2::new(w, TOUCH), |ui| {
                    self.file_button(ui, shared, "改用照片檔", TOUCH, 18.0);
                });
            });
        });
    }

    fn photo_card(&self, ui: &mut Ui, shared: &mut Shared, proposal: &Proposal, max_h: f32) {
        let Some(path) = proposal.image_path.as_deref() else { return };
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("這張照片").font(bold_font(18.0)));
                if let Some(t) = &proposal.source_time {
                    tag(ui, format!("拍攝時間 {}", short_time(t)), PRIMARY);
                }
                if let Some(n) = &proposal.source_name {
                    ui.label(muted(n));
                }
                ui.label(small(format!("辨識 {:.1} 秒", proposal.timing_ms as f64 / 1000.0)));
            });
            let abs = shared.store.resolve_image(path);
            shared.images.show(ui, &abs, Vec2::new(ui.available_width(), max_h));
            ui.add_space(6.0);
            egui::Grid::new("reading").num_columns(4).spacing([16.0, 6.0]).show(ui, |ui| {
                let r = &proposal.recognition;
                ui.label(muted("型號"));
                ui.label(strong(r.model_code.clone().or(r.item_code.clone()).unwrap_or_else(|| "讀不出來".into())));
                ui.label(muted("製造日期"));
                ui.label(strong(r.manufacture_date.clone().unwrap_or_else(|| "讀不出來".into())));
                ui.end_row();
                ui.label(muted("箱上料號"));
                ui.label(or_dash(r.item_code.as_deref()));
                ui.label(muted("驗收章"));
                ui.label(or_dash(r.receipt_date.as_deref()));
                ui.end_row();
            });
            if !proposal.recognition.notes.is_empty() {
                ui.label(small(&proposal.recognition.notes));
            }
        });
    }

    /// The answer to the only question the operator asked: can this box go
    /// out? Colour and heading carry it alone, because it gets read from a few
    /// steps away before anyone leans in.
    fn verdict(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let Some(p) = self.capture.clone() else {
            self.stage = Stage::Manual;
            return;
        };
        let fifo_lot = self.lots.iter().find(|l| l.is_fifo_next).cloned();
        // Three states, genuinely different: fine / wrong box / could not tell
        // which box. Collapsing the last two would tell someone to swap a box
        // we never identified.
        let (color, icon, heading) = match (&p.locked_lot, p.fifo_ok) {
            (Some(_), Some(true)) => (OK, "✔", "可以拿"),
            (Some(_), _) => (BAD, "✖", "這箱先別拿"),
            (None, _) => (WARN, "?", "認得出品項，但讀不出是哪一批"),
        };
        Frame::new().fill(color).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(28)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let white = |t: String, size: f32, b: bool| {
                RichText::new(t).font(if b { bold_font(size) } else { font(size) }).color(Color32::WHITE)
            };
            ui.horizontal(|ui| {
                ui.label(white(icon.into(), 52.0, true));
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(white(heading.into(), 44.0, true));
                    ui.label(white(
                        format!(
                            "{}　{}",
                            p.item_name.clone().unwrap_or_default(),
                            p.item_model.clone().unwrap_or_default()
                        ),
                        22.0,
                        false,
                    ));
                });
            });
            ui.add_space(10.0);
            match (&p.locked_lot, p.fifo_ok) {
                (Some(l), Some(true)) => {
                    ui.label(white(
                        format!(
                            "製造 {}｜進貨 {}｜這批還有 {} 箱",
                            l.manufacture_date.as_deref().unwrap_or("未填"),
                            l.receipt_date,
                            l.qty_on_hand
                        ),
                        20.0,
                        false,
                    ));
                    ui.label(white("這就是先進先出該領的那一批。".into(), 17.0, false));
                }
                (Some(l), _) => {
                    ui.label(white(
                        format!(
                            "你這箱：製造 {}（進貨 {}）",
                            l.manufacture_date.as_deref().unwrap_or("未填"),
                            l.receipt_date
                        ),
                        20.0,
                        false,
                    ));
                    let extra = fifo_lot
                        .as_ref()
                        .map(|f| format!("（進貨 {}，還有 {} 箱）", f.receipt_date, f.qty_on_hand))
                        .unwrap_or_default();
                    ui.label(white(
                        format!("該拿的是：製造 {} 那批{}", p.fifo_expected_date.clone().unwrap_or_default(), extra),
                        21.0,
                        true,
                    ));
                }
                (None, _) => {
                    ui.label(white(
                        format!(
                            "生產日期讀到「{}」，跟在庫的批次都對不起來。",
                            p.recognition.manufacture_date.as_deref().unwrap_or("讀不出")
                        ),
                        19.0,
                        false,
                    ));
                    ui.label(white("照片已留存。請在下面挑正確的批次。".into(), 17.0, false));
                }
            }
        });
        ui.add_space(12.0);
        self.photo_card(ui, shared, &p, 360.0);
        ui.add_space(12.0);

        // Every lot's 製造日 laid out, so the operator sees where the box in
        // hand sits — "take that one" means more when you can see them all.
        if !self.lots.is_empty() {
            titled_card(
                ui,
                format!("{} 在庫批次的製造日期", p.item_label().unwrap_or("這個品項")),
                |_| {},
                |ui| {
                    for lot in &self.lots {
                        let this = p.locked_lot.as_ref().is_some_and(|l| l.lot_id == lot.id);
                        row_frame(this).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "製造 {}",
                                        lot.manufacture_date.as_deref().unwrap_or("未填")
                                    ))
                                    .font(bold_font(17.0)),
                                );
                                if this {
                                    tag(ui, "你手上這箱", PRIMARY);
                                }
                                if lot.is_fifo_next {
                                    tag(ui, "FIFO 應領", OK);
                                }
                                if lot.fifo_also_ok {
                                    tag_grey(ui, "同製造日．可領");
                                }
                                ui.label(muted(format!("進貨 {}", lot.receipt_date)));
                                ui.label(muted(format!("在庫 {} 箱", lot.qty_on_hand)));
                            });
                        });
                    }
                },
            );
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            let wrong = p.locked_lot.is_some() && p.fifo_ok == Some(false);
            if secondary(ui, if wrong { "↻ 去換一箱，重拍" } else { "↻ 重拍" }).clicked() {
                self.reset();
            }
            if link(ui, "辨識錯了？改手動選").clicked() {
                self.stage = Stage::Manual;
            }
        });
        if p.locked_lot.is_some() && p.fifo_ok == Some(false) {
            alert(
                ui,
                Kind::Warning,
                "還是要領這箱的話，往下送出",
                "系統會完整記錄下來（誰、何時、拿了哪批、應拿哪批、照片），但不扣帳，等主管覆核。重拍不會讓這筆紀錄消失。",
            );
        }
    }

    fn manual(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let capture = self.capture.clone();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("選品項").font(bold_font(20.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary(ui, "📷 改用拍照").clicked() {
                        self.reset();
                    }
                });
            });
            if let Some(p) = capture.as_ref().filter(|p| p.item_id.is_none()) {
                let reason = match p.item_defer_reason {
                    Some("no_code_read") => "讀不到型號",
                    Some("ambiguous_item") => "有兩個以上型號都可能是它",
                    Some("no_item_match") => "讀到的型號不在主檔裡",
                    _ => "",
                };
                alert_with(
                    ui,
                    Kind::Warning,
                    "這張照片認不出是哪個品項，請自己選",
                    &format!(
                        "型號：{}｜生產日期：{}{}",
                        p.recognition
                            .model_code
                            .clone()
                            .or(p.recognition.item_code.clone())
                            .unwrap_or_else(|| "讀不出來".into()),
                        p.recognition.manufacture_date.clone().unwrap_or_else(|| "讀不出來".into()),
                        if reason.is_empty() { String::new() } else { format!("（{reason}）") }
                    ),
                    |ui| {
                        ui.label(small("照片已留存，會跟著這筆紀錄一起存檔。"));
                    },
                );
                ui.add_space(8.0);
            }
            let options: Vec<(i64, String, bool)> = self
                .items
                .iter()
                .map(|i| {
                    let spec = i.spec.as_deref().map(|s| format!(" {s}")).unwrap_or_default();
                    (i.id, format!("{}｜{}{}（在庫 {} 箱）", i.label(), i.name, spec, i.on_hand), i.on_hand > 0)
                })
                .collect();
            let before = self.item_id;
            ui.scope(|ui| {
                ui.spacing_mut().interact_size.y = 48.0;
                ui.style_mut().text_styles.insert(egui::TextStyle::Button, font(19.0));
                ui.style_mut().text_styles.insert(egui::TextStyle::Body, font(19.0));
                search_select(
                    ui,
                    "issue-item",
                    &mut self.item_id,
                    &options,
                    "選要領用的品項",
                    ui.available_width() - 20.0,
                );
            });
            ui.label(small("在庫 0 的品項不能選。"));
            if self.item_id != before
                && let Some(id) = self.item_id
            {
                if let Some(p) = &mut self.capture
                    && p.image_path.is_some()
                {
                    // Re-run the lot match on the reading already paid for.
                    let _ = service::resolve_item(&shared.store.conn, p, id);
                }
                let prefer = self.capture.as_ref().and_then(|p| p.locked_lot.as_ref().map(|l| l.lot_id));
                self.reload_lots(shared, id, prefer);
            }
        });
        if let Some(p) = capture.as_ref().filter(|p| p.item_id.is_none()) {
            // Seeing the photo is how to tell "blurred" from "not in the master".
            ui.add_space(12.0);
            self.photo_card(ui, shared, p, 360.0);
        }
    }

    fn lots_and_submit(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        let Some(item_id) = self.item_id else { return };
        ui.add_space(14.0);
        if self.lots.is_empty() {
            Frame::new()
                .fill(Color32::WHITE)
                .stroke(egui::Stroke::new(2.0, BAD))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(20))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("還不能領用").font(bold_font(22.0)).color(BAD));
                    let label = self
                        .items
                        .iter()
                        .find(|i| i.id == item_id)
                        .map(|i| i.label().to_string())
                        .unwrap_or_else(|| "此品項".into());
                    ui.label(format!("{label} 沒有可領用的在庫批次。FIFO 要排序，就得先知道還有哪幾批在庫。"));
                    if user.can("lot.create") && primary(ui, "📦 去收貨建批").clicked() {
                        shared.nav = Some(Nav::Go(Page::Receiving));
                    }
                });
            return;
        }

        let p = self.capture.clone();
        // The box in hand is not on the books: following the preselected lot
        // would create a record pointing at the wrong lot, and the trace report
        // would never show what went wrong.
        if let Some(p) = p.as_ref().filter(|p| p.photo_disagrees()) {
            alert_with(
                ui,
                Kind::Error,
                &format!(
                    "照片上的生產日期 {} 不在這個品項的在庫批次裡",
                    p.recognition.manufacture_date.clone().unwrap_or_default()
                ),
                &format!(
                    "在庫的是：{}\n手上這箱跟帳上對不起來 —— 可能是這批還沒收貨建批、型號選錯，或日期讀錯了。照著下面預選的批次送出，會生出一筆指向錯誤批次的紀錄。",
                    self.lots
                        .iter()
                        .map(|l| l.manufacture_date.clone().unwrap_or_else(|| "未填".into()))
                        .collect::<Vec<_>>()
                        .join("、")
                ),
                |ui| {
                    ui.horizontal(|ui| {
                        if secondary(ui, "↻ 重拍").clicked() {
                            self.reset();
                        }
                        if user.can("lot.create") && primary(ui, "📦 去收貨").clicked() {
                            shared.nav = Some(Nav::Go(Page::Receiving));
                        }
                    });
                },
            );
            ui.add_space(10.0);
        }

        let picking = self.stage == Stage::Manual || p.as_ref().is_none_or(|p| p.locked_lot.is_none());
        let fifo_lot = self.lots.iter().find(|l| l.is_fifo_next).cloned();
        if picking {
            Frame::new()
                .fill(Color32::WHITE)
                .stroke(egui::Stroke::new(1.5, OK))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(20))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("可以領用").font(bold_font(20.0)).color(super::theme::OK_DARK));
                        ui.label(muted("選批次（先進先出看製造日）"));
                    });
                    ui.add_space(6.0);
                    for lot in self.lots.clone() {
                        let selected = self.lot_id == Some(lot.id);
                        let r = row_frame(selected)
                            .inner_margin(Margin::symmetric(16, 14))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.set_min_height(TOUCH - 28.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(if selected { "◉" } else { "○" })
                                            .font(font(22.0))
                                            .color(if selected { PRIMARY } else { MUTED }),
                                    );
                                    // Manufacture date leads: it is what FIFO sorts on,
                                    // so it is what someone should read off the box.
                                    ui.label(
                                        RichText::new(format!(
                                            "製造 {}",
                                            lot.manufacture_date.as_deref().unwrap_or("未填")
                                        ))
                                        .font(bold_font(19.0)),
                                    );
                                    if lot.is_fifo_next {
                                        tag(ui, "FIFO 應領", OK);
                                    }
                                    if lot.fifo_also_ok {
                                        tag_grey(ui, "同製造日．可領")
                                            .on_hover_text("跟應領那批同一個製造日，領這批也合法，不會被擋");
                                    }
                                    ui.label(muted(format!("進貨 {}", lot.receipt_date)));
                                    if let (Some(d), Some(left)) = (&lot.expiry_date, lot.days_left) {
                                        let c = if left <= 90 { BAD } else { MUTED };
                                        ui.label(RichText::new(format!("有效 {d}（剩 {left} 天）")).color(c));
                                    }
                                    ui.label(muted(format!("在庫 {} 箱", lot.qty_on_hand)));
                                });
                            })
                            .response
                            .interact(egui::Sense::click());
                        if r.clicked() {
                            self.lot_id = Some(lot.id);
                        }
                        ui.add_space(4.0);
                    }
                    if self.lots.first().is_some_and(|l| l.fifo_basis == "進貨日期") {
                        alert(
                            ui,
                            Kind::Warning,
                            "這個品項的批次都沒填製造日期，暫時改用進貨日排序",
                            "先進先出看的是製造日期。補上製造日期後排序才會準。",
                        );
                    }
                });
            ui.add_space(12.0);
        }

        let chosen = self.lots.iter().find(|l| Some(l.id) == self.lot_id).cloned();
        // Only warn about a genuinely later lot; a same-day lot is accepted.
        let wrong = chosen.as_ref().is_some_and(|l| !l.is_fifo_next && !l.fifo_also_ok);
        if wrong
            && picking
            && let Some(f) = &fifo_lot
        {
            alert(
                ui,
                Kind::Warning,
                "這不是 FIFO 應領的那批",
                &format!(
                    "應領用製造日 {} 那批（進貨 {}）。仍然可以送出 —— 系統會記錄下來並擋住扣帳，等主管覆核。",
                    f.manufacture_date.as_deref().unwrap_or(&f.receipt_date),
                    f.receipt_date
                ),
            );
            ui.add_space(8.0);
        }

        card(ui, |ui| {
            let max = chosen.as_ref().map_or(1, |l| l.qty_on_hand).max(1);
            ui.horizontal(|ui| {
                ui.label(RichText::new("領幾箱").font(font(19.0)));
                let step = |ui: &mut Ui, t: &str, enabled: bool| {
                    ui.add_enabled(
                        enabled,
                        egui::Button::new(RichText::new(t).font(font(28.0)))
                            .min_size(Vec2::splat(TOUCH))
                            .corner_radius(CornerRadius::same(10)),
                    )
                };
                if step(ui, "−", self.qty > 1).clicked() {
                    self.qty -= 1;
                }
                ui.add_sized(
                    Vec2::new(90.0, TOUCH),
                    egui::Label::new(RichText::new(self.qty.to_string()).font(bold_font(30.0))),
                );
                if step(ui, "＋", self.qty < max).clicked() {
                    self.qty += 1;
                }
                ui.label(muted(format!("這批在庫 {max} 箱")));
            });
            self.qty = self.qty.clamp(1, max);
            ui.add_space(8.0);
            let toggle_text = if self.show_extra {
                "▼ 其他資訊（選填，可事後補）"
            } else {
                "▶ 其他資訊（選填，可事後補）"
            };
            if ui.add(egui::Button::new(muted(toggle_text)).frame(false)).clicked() {
                self.show_extra = !self.show_extra;
            }
            if self.show_extra {
                ui.horizontal_wrapped(|ui| {
                    field(ui, "包裝機台", false, None, |ui| {
                        select(ui, "machine", &mut self.machine, &self.machines, "選機台", 200.0, true)
                    });
                    ui.add_space(12.0);
                    field(ui, "產品名稱", false, Some("沒填會列入「明細待補」"), |ui| {
                        select(ui, "product", &mut self.product, &self.products, "包了哪個產品", 200.0, true)
                    });
                    ui.add_space(12.0);
                    field(ui, "使用步徑（碼表）", false, None, |ui| {
                        text_input(ui, &mut self.steps, "例 557", 160.0)
                    });
                });
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let w = ui.available_width();
                let label = if wrong {
                    "還是要領這箱（會記錄、不扣帳）".to_string()
                } else {
                    format!("確認領用 {} 箱", self.qty)
                };
                let (fill, enabled) = (if wrong { BAD } else { PRIMARY }, self.lot_id.is_some());
                ui.allocate_ui(Vec2::new(w * 0.66, TOUCH), |ui| {
                    if touch(ui, label, fill, Color32::WHITE, TOUCH, 20.0, enabled).clicked() {
                        self.submit(shared, user, item_id);
                    }
                });
                // Cancel only drops this recognition — nothing was deducted and
                // no record deleted, because none exists before confirming.
                ui.allocate_ui(Vec2::new(ui.available_width(), TOUCH), |ui| {
                    if touch(ui, "取消", Color32::WHITE, super::theme::TEXT, TOUCH, 20.0, true).clicked() {
                        self.reset();
                    }
                });
            });
            if let (Some(item), Some(l)) = (self.items.iter().find(|i| i.id == item_id), &chosen) {
                ui.vertical_centered(|ui| {
                    ui.label(muted(format!(
                        "{}｜{} 箱｜製造 {}｜進貨 {}",
                        item.label(),
                        self.qty,
                        l.manufacture_date.as_deref().unwrap_or("未填"),
                        l.receipt_date
                    )));
                });
            }
        });
    }

    fn submit(&mut self, shared: &mut Shared, user: &Session, item_id: i64) {
        let mut fields = Map::new();
        fields.insert("包裝機台".into(), self.machine.clone().map_or(Value::Null, Value::String));
        fields.insert("產品名稱".into(), self.product.clone().map_or(Value::Null, Value::String));
        fields.insert(
            "使用步徑".into(),
            if self.steps.trim().is_empty() { Value::Null } else { json!(self.steps.trim()) },
        );
        let p = self.capture.as_ref();
        let input = ScanInput {
            item_id,
            lot_id: self.lot_id,
            qty: self.qty,
            image_path: p.and_then(|p| p.image_path.clone()),
            ocr_receipt_date: p.and_then(|p| p.recognition.receipt_date.clone()),
            ocr_confidence: p.map(|p| p.recognition.receipt_date_confidence),
            ocr_notes: p.map(|p| p.recognition.notes.clone()).filter(|n| !n.is_empty()),
            match_distance: p.and_then(|p| p.match_distance),
            fields,
            detail_pending: self.product.is_none(),
        };
        let result = service::create_scan(&shared.store, user, &input);
        if let Some(outcome) = shared.act(result, None) {
            self.outcome = Some(outcome);
        }
    }

    fn outcome_block(&mut self, ui: &mut Ui, outcome: &ScanOutcome) {
        let (color, heading) = match outcome.status {
            "blocked_fifo" => (BAD, "已記錄，請換一箱"),
            "blocked_unreadable" => (WARN, "已記錄，待人工補批次"),
            _ => (OK, "已登錄"),
        };
        Frame::new().fill(color).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(48)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height() * 0.6);
            let white = |t: String, size: f32, b: bool| {
                RichText::new(t).font(if b { bold_font(size) } else { font(size) }).color(Color32::WHITE)
            };
            ui.label(white(heading.into(), 52.0, true));
            if outcome.status == "blocked_fifo" {
                ui.add_space(10.0);
                ui.label(white(
                    format!("應領用：製造日 {} 那批", outcome.fifo_expected_date.clone().unwrap_or_default()),
                    28.0,
                    false,
                ));
            }
            ui.add_space(20.0);
            let detail = if outcome.status == "blocked_fifo" {
                format!(
                    "這一筆已存成紀錄 #{}，包含時間與登記人。重拍或重送不會讓它消失，請去拿製造日較早的那箱。",
                    outcome.id
                )
            } else {
                format!("紀錄 #{} 已建立。", outcome.id)
            };
            ui.label(white(detail, 19.0, false));
            ui.add_space(36.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("下一箱").font(bold_font(22.0)))
                        .fill(Color32::WHITE)
                        .min_size(Vec2::new(240.0, TOUCH))
                        .corner_radius(CornerRadius::same(10)),
                )
                .clicked()
            {
                self.reset();
            }
        });
    }
}

fn busy(ui: &mut Ui, since: Option<Instant>) {
    card(ui, |ui| {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.add(egui::Spinner::new().size(48.0));
            ui.add_space(12.0);
            ui.label(RichText::new("辨識中，看是哪一箱…").font(font(22.0)));
            if let Some(t) = since {
                ui.label(muted(format!("{:.0} 秒", t.elapsed().as_secs_f64())));
                ui.ctx().request_repaint_after(Duration::from_millis(500));
            }
        });
        ui.add_space(40.0);
    });
}

enum Source {
    Camera,
    File(PathBuf),
}

/// Runs on a worker thread with its own connection: capture, save, recognise,
/// decode for display. Whatever produced the bytes, the rest is identical.
fn capture_and_recognise(data_dir: &std::path::Path, config: &CameraConfig, source: Source) -> CaptureResult {
    let failed = |e: AppError| CaptureFailure::Failed(e.0);
    let (bytes, info, suffix) = match source {
        Source::Camera => match camera::capture(config) {
            Ok(frame) => (frame.data, frame.source, "-cam"),
            Err(CameraError::Stale { .. }) => return Err(CaptureFailure::Stale),
            Err(e) => return Err(CaptureFailure::Failed(e.to_string())),
        },
        Source::File(path) => match std::fs::read(&path) {
            Ok(b) => (b, None, ""),
            Err(e) => return Err(CaptureFailure::Failed(format!("讀不到 {}：{e}", path.display()))),
        },
    };
    let store = Store::open(data_dir).map_err(failed)?;
    let provider = service::provider(&store.conn);
    let mut proposal = service::recognise_and_store(&store, provider.as_ref(), &bytes, suffix).map_err(failed)?;
    if let Some(info) = info {
        proposal.source_name = Some(info.name);
        proposal.source_time = Some(info.time);
    }
    let image = super::images::decode(&bytes).ok();
    Ok((proposal, image))
}

#[cfg(windows)]
fn pick_image() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("選一張箱子的照片")
        .add_filter("影像", &["jpg", "jpeg", "png", "bmp", "webp"])
        .pick_file()
}

/// Development builds on Linux: take the newest image in $URDWMS_PICK.
#[cfg(not(windows))]
fn pick_image() -> Option<PathBuf> {
    let dir = std::env::var("URDWMS_PICK").ok()?;
    let config = CameraConfig { transport: "folder".into(), folder: dir, ..Default::default() };
    camera::newest_image(&config).ok().map(|(p, _)| p)
}
