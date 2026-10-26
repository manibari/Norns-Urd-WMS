//! Records and traceability.
//!
//! Blocked draws are listed alongside posted ones, not filed away as errors.
//! That is the product: a blocked record says who, when, which machine, which
//! lot they took, which they should have taken, and the photo — permanently
//! (requirement 2.1). It belongs in traceability too, because the system can
//! block the books but not the physical box (risk R3).

use std::collections::HashSet;

use egui::{RichText, ScrollArea, Ui, Vec2};
use serde_json::{Map, Value};

use super::Shared;
use super::theme::{BAD, MUTED, OK, ORANGE, bold_font, font};
use super::widgets::*;
use crate::auth::Session;
use crate::service::{self, Scan, status_label};

#[derive(Default)]
pub struct RecordsPage {
    loaded: Option<u64>,
    rows: Vec<Scan>,
    reasons: Vec<String>,
    products: Vec<String>,
    machines: Vec<String>,
    expanded: HashSet<i64>,
    status_filter: Option<String>,
    product_filter: Option<String>,
    search: String,
    overriding: Option<Override>,
    detail: Option<Detail>,
}

struct Override {
    scan: Scan,
    reason: Option<String>,
    other: String,
}

struct Detail {
    scan: Scan,
    machine: Option<String>,
    product: Option<String>,
    steps: String,
}

const STATUSES: [&str; 4] = ["已登錄", "非 FIFO", "已覆核放行", "待人工補批次"];

fn status_color(status: &str) -> egui::Color32 {
    match status {
        "posted" => OK,
        "blocked_fifo" => BAD,
        "overridden" | "blocked_unreadable" => ORANGE,
        _ => MUTED,
    }
}

impl RecordsPage {
    fn load(&mut self, shared: &mut Shared) {
        let conn = &shared.store.conn;
        match service::scans(conn, None, 2000) {
            Ok(r) => self.rows = r,
            Err(e) => shared.error(format!("領用紀錄載入失敗：{e}")),
        }
        let conn = &shared.store.conn;
        self.reasons = service::values(conn, "override_reason");
        self.products = service::values(conn, "packed_product");
        self.machines = service::values(conn, "machine");
        self.loaded = Some(shared.generation);
    }

    fn visible(&self, scan: &Scan) -> bool {
        if let Some(s) = &self.status_filter
            && status_label(&scan.status) != s
        {
            return false;
        }
        if let Some(p) = &self.product_filter
            && scan.field("產品名稱").as_deref() != Some(p.as_str())
        {
            return false;
        }
        let needle = self.search.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        let hay = format!(
            "#{} {} {} {} {} {}",
            scan.id,
            scan.item_label.clone().unwrap_or_default(),
            scan.captured_by,
            scan.manufacture_date.clone().unwrap_or_default(),
            scan.receipt_date.clone().unwrap_or_default(),
            Value::Object(scan.field_values.clone())
        )
        .to_lowercase();
        hay.contains(&needle)
    }

    pub fn show(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        if self.loaded != Some(shared.generation) {
            self.load(shared);
        }
        let blocked = self.rows.iter().filter(|r| r.status == "blocked_fifo").count();
        let pending = self
            .rows
            .iter()
            .filter(|r| r.detail_pending && matches!(r.status.as_str(), "posted" | "overridden"))
            .count();

        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            page_title(
                ui,
                "紀錄與追溯",
                Some("每一筆領用都在這裡，包括被擋下的 —— 誰、何時、拿了哪批、應拿哪批、照片。"),
            );
            card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(strong(format!("共 {} 筆", self.rows.len())));
                    if blocked > 0 {
                        ui.colored_label(BAD, format!("{blocked} 筆非 FIFO 待覆核"));
                    }
                    if pending > 0 {
                        ui.colored_label(super::theme::WARN_DARK, format!("{pending} 筆產品名稱待補"));
                    }
                    ui.add_space(20.0);
                    let statuses: Vec<String> = STATUSES.iter().map(|s| s.to_string()).collect();
                    select(ui, "rec-status", &mut self.status_filter, &statuses, "全部狀態", 150.0, true);
                    select(
                        ui,
                        "rec-product",
                        &mut self.product_filter,
                        &self.products,
                        "全部產品（追溯）",
                        170.0,
                        true,
                    );
                    text_input(ui, &mut self.search, "🔍 搜尋品項、批次日期、登記人", 260.0);
                });
                ui.add_space(6.0);
                let shown: Vec<Scan> = self.rows.iter().filter(|r| self.visible(r)).cloned().collect();
                if shown.is_empty() {
                    return empty(ui, if self.rows.is_empty() { "尚無領用紀錄" } else { "沒有符合條件的紀錄" });
                }
                for scan in shown.iter().take(300) {
                    self.scan_row(ui, shared, user, scan);
                    ui.add_space(4.0);
                }
                if shown.len() > 300 {
                    ui.label(muted(format!("只列出最新 300 筆（共 {} 筆符合），請用篩選縮小範圍。", shown.len())));
                }
            });
        });
        self.override_modal(ui, shared, user);
        self.detail_modal(ui, shared, user);
    }

    fn scan_row(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session, scan: &Scan) {
        let open = self.expanded.contains(&scan.id);
        let r = row_frame(open)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add_sized([60.0, 28.0], egui::Label::new(muted(format!("#{}", scan.id))));
                    ui.add_sized([140.0, 28.0], egui::Label::new(short_time(&scan.captured_at)));
                    ui.add_sized(
                        [150.0, 28.0],
                        egui::Label::new(strong(scan.item_label.clone().unwrap_or_else(|| "—".into()))).truncate(),
                    );
                    ui.allocate_ui(Vec2::new(200.0, 28.0), |ui| {
                        ui.set_width(200.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(format!("製造 {}", or_dash(scan.manufacture_date.as_deref())));
                            ui.label(small(format!("進貨 {}｜{} 箱", or_dash(scan.receipt_date.as_deref()), scan.qty)));
                        });
                    });
                    tag(ui, status_label(&scan.status), status_color(&scan.status));
                    if scan.detail_pending && scan.status != "blocked_fifo" {
                        tag(ui, "產品待補", ORANGE);
                    }
                    ui.label(muted(format!("機台 {}", scan.field("包裝機台").unwrap_or_else(|| "—".into()))));
                    match scan.field("產品名稱") {
                        Some(p) => ui.label(format!("產品 {p}")),
                        None => ui.label(muted("產品 待補")),
                    };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if scan.status == "blocked_fifo"
                            && user.can("scan.override")
                            && secondary(ui, "覆核放行").clicked()
                        {
                            self.overriding = Some(Override { scan: scan.clone(), reason: None, other: String::new() });
                        }
                        if scan.detail_pending
                            && matches!(scan.status.as_str(), "posted" | "overridden")
                            && user.can("issue.detail")
                            && link(ui, "補明細").clicked()
                        {
                            self.detail = Some(Detail {
                                scan: scan.clone(),
                                machine: scan.field("包裝機台"),
                                product: scan.field("產品名稱"),
                                steps: scan.field("使用步徑").unwrap_or_default(),
                            });
                        }
                    });
                });
                if open {
                    divider(ui);
                    ui.horizontal_top(|ui| {
                        egui::Grid::new(("scan-detail", scan.id)).num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                            let mut kv = |k: &str, v: String| {
                                ui.label(muted(k));
                                ui.label(v);
                                ui.end_row();
                            };
                            kv("登記人", scan.captured_by.clone());
                            kv("辨識讀到（驗收章）", or_dash(scan.ocr_receipt_date.as_deref()));
                            kv("辨識信心", scan.ocr_confidence.map_or("—".into(), |c| format!("{:.0}%", c * 100.0)));
                            kv("FIFO 應領（製造日）", or_dash(scan.fifo_expected_date.as_deref()));
                            for (k, v) in &scan.field_values {
                                let text = match v {
                                    Value::String(s) if !s.is_empty() => s.clone(),
                                    Value::Null | Value::String(_) => "—".into(),
                                    other => other.to_string(),
                                };
                                kv(k, text);
                            }
                            if let Some(reason) = &scan.override_reason {
                                kv("覆核原因", format!("{reason}（{}）", scan.override_by.clone().unwrap_or_default()));
                            }
                        });
                        if let Some(path) = &scan.image_path {
                            ui.add_space(20.0);
                            ui.vertical(|ui| {
                                ui.label(muted("影像"));
                                let abs = shared.store.resolve_image(path);
                                shared.images.show(ui, &abs, Vec2::new(420.0, 320.0));
                            });
                        } else {
                            ui.label(muted("（手動登錄，沒有影像）"));
                        }
                    });
                }
            })
            .response;
        if r.interact(egui::Sense::click()).clicked() {
            if open {
                self.expanded.remove(&scan.id);
            } else {
                self.expanded.insert(scan.id);
            }
        }
    }

    fn override_modal(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        let Some(o) = &mut self.overriding else { return };
        let mut close = false;
        let mut confirm = false;
        egui::Modal::new(egui::Id::unique("override")).show(ui.ctx(), |ui| {
            ui.set_width(480.0);
            ui.label(RichText::new(format!("覆核放行 #{}", o.scan.id)).font(bold_font(20.0)));
            ui.label(format!(
                "{}｜拿了製造 {}，應領製造 {}",
                o.scan.item_label.clone().unwrap_or_default(),
                or_dash(o.scan.manufacture_date.as_deref()),
                or_dash(o.scan.fifo_expected_date.as_deref())
            ));
            ui.label(muted(
                "放行後這筆會扣庫存並改為「已覆核放行」，原因寫入稽核軌跡。沒有這條路，現場就會學會不拍照直接用。",
            ));
            ui.add_space(8.0);
            field(ui, "原因", true, None, |ui| {
                select(ui, "ovr-reason", &mut o.reason, &self.reasons, "選擇原因", 300.0, false)
            });
            if o.reason.as_deref() == Some("其他") {
                field(ui, "說明", true, None, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut o.other).desired_rows(3).desired_width(440.0))
                });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary(ui, "放行並扣庫存").clicked() {
                    confirm = true;
                }
                if secondary(ui, "取消").clicked() {
                    close = true;
                }
            });
        });
        if confirm {
            let reason = match o.reason.as_deref() {
                Some("其他") => o.other.trim().to_string(),
                Some(r) => r.to_string(),
                None => String::new(),
            };
            if reason.is_empty() {
                shared.error("必須填原因");
            } else {
                let id = o.scan.id;
                if shared
                    .act(service::override_scan(&shared.store, user, id, &reason), Some("已放行並扣庫存"))
                    .is_some()
                {
                    close = true;
                }
            }
        }
        if close {
            self.overriding = None;
        }
    }

    fn detail_modal(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        let Some(d) = &mut self.detail else { return };
        let mut close = false;
        let mut save = false;
        egui::Modal::new(egui::Id::unique("scan-detail")).show(ui.ctx(), |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new(format!("補明細 #{}", d.scan.id)).font(bold_font(20.0)));
            ui.label(muted(format!(
                "{}｜{}",
                d.scan.item_label.clone().unwrap_or_default(),
                short_time(&d.scan.captured_at)
            )));
            ui.add_space(8.0);
            field(ui, "包裝機台", false, None, |ui| {
                select(ui, "dt-machine", &mut d.machine, &self.machines, "選機台", 260.0, true)
            });
            field(ui, "產品名稱", false, None, |ui| {
                select(ui, "dt-product", &mut d.product, &self.products, "包了哪個產品", 260.0, true)
            });
            field(ui, "使用步徑（碼表）", false, None, |ui| text_input(ui, &mut d.steps, "例 557", 160.0));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary(ui, "儲存").clicked() {
                    save = true;
                }
                if secondary(ui, "取消").clicked() {
                    close = true;
                }
            });
        });
        if save {
            let mut fields = Map::new();
            fields.insert("包裝機台".into(), d.machine.clone().map_or(Value::Null, Value::String));
            fields.insert("產品名稱".into(), d.product.clone().map_or(Value::Null, Value::String));
            fields.insert(
                "使用步徑".into(),
                if d.steps.trim().is_empty() { Value::Null } else { Value::String(d.steps.trim().into()) },
            );
            let id = d.scan.id;
            if shared.act(service::update_scan_detail(&shared.store, user, id, &fields), Some("已補上明細")).is_some()
            {
                close = true;
            }
        }
        if close {
            self.detail = None;
        }
        let _ = font;
    }
}
