//! System log.
//!
//! Every write lands in audit_log: logins, receiving, corrections, deletions,
//! dictionary and role edits, account changes, draws and overrides. US-1
//! requires corrections to be recorded rather than silent, and a record nobody
//! can read is silent in every way that matters.
//!
//! Corrections render before → after. "Someone changed the receipt date" says
//! nothing without the old value.

use egui::{RichText, ScrollArea, Ui};
use serde_json::Value;

use super::Shared;
use super::theme::{MUTED, bold_font, font};
use super::widgets::*;
use crate::auth::Session;
use crate::service::{self, AuditEntry, action_label};

const GROUPS: &[(&str, &str)] = &[
    ("批次", "lot"),
    ("型號", "item"),
    ("領用", "scan"),
    ("登入／密碼", "auth"),
    ("人員", "user"),
    ("選項", "dictionary"),
    ("角色", "role"),
    ("設定", "setting"),
    ("影像來源", "camera"),
];

const PAGE_SIZE: usize = 50;

#[derive(Default)]
pub struct LogsPage {
    loaded: Option<(u64, Option<String>)>,
    rows: Vec<AuditEntry>,
    group: Option<String>,
    search: String,
    page: usize,
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Null => "—".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn detail(ui: &mut Ui, entry: &AuditEntry) {
    let d = &entry.detail;
    if let (Some(before), Some(after)) =
        (d.get("before").and_then(Value::as_object), d.get("after").and_then(Value::as_object))
    {
        for (k, v) in after {
            ui.horizontal(|ui| {
                ui.label(muted(format!("{k}：")));
                ui.label(RichText::new(value_text(before.get(k).unwrap_or(&Value::Null))).strikethrough().color(MUTED));
                ui.label("→");
                ui.label(RichText::new(value_text(v)).font(bold_font(15.0)));
            });
        }
        return;
    }
    let text = match d.as_object() {
        Some(map) => map.iter().map(|(k, v)| format!("{k}={}", value_text(v))).collect::<Vec<_>>().join("　"),
        None => value_text(d),
    };
    ui.add(egui::Label::new(RichText::new(text).font(font(13.0)).color(MUTED)).wrap());
}

impl LogsPage {
    pub fn show(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        let key = (shared.generation, self.group.clone());
        if self.loaded.as_ref() != Some(&key) {
            let prefix = self.group.as_ref().and_then(|g| GROUPS.iter().find(|(l, _)| l == g)).map(|(_, p)| *p);
            match service::audit(&shared.store, user, prefix, 5000) {
                Ok(r) => self.rows = r,
                Err(e) => shared.error(e.0),
            }
            self.loaded = Some(key);
            self.page = 0;
        }
        let needle = self.search.trim().to_lowercase();
        let filtered: Vec<&AuditEntry> = self
            .rows
            .iter()
            .filter(|r| {
                needle.is_empty()
                    || r.actor.to_lowercase().contains(&needle)
                    || action_label(&r.action).to_lowercase().contains(&needle)
                    || r.detail.to_string().to_lowercase().contains(&needle)
            })
            .collect();
        let pages = filtered.len().div_ceil(PAGE_SIZE).max(1);
        self.page = self.page.min(pages - 1);

        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            page_title(ui, "系統日誌", Some("所有寫入操作都留在這裡：登入、收貨、修正、刪除、選項與角色異動、領用與覆核。修正會顯示改動前後的值。"));
            card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let groups: Vec<String> = GROUPS.iter().map(|(l, _)| l.to_string()).collect();
                    select(ui, "log-group", &mut self.group, &groups, "全部", 160.0, true);
                    if text_input(ui, &mut self.search, "🔍 搜尋操作者、動作、內容", 320.0).changed() {
                        self.page = 0;
                    }
                    ui.label(muted(format!("{} 筆", filtered.len())));
                });
                ui.add_space(6.0);
                if filtered.is_empty() {
                    return empty(ui, "沒有符合的紀錄");
                }
                egui::Grid::new("audit").striped(true).num_columns(4).spacing([18.0, 10.0]).min_row_height(30.0).show(ui, |ui| {
                    for h in ["時間", "操作者", "動作", "內容"] {
                        ui.label(RichText::new(h).font(font(14.0)).color(MUTED));
                    }
                    ui.end_row();
                    for entry in filtered.iter().skip(self.page * PAGE_SIZE).take(PAGE_SIZE) {
                        ui.label(entry.at.get(..19).unwrap_or(&entry.at).replace('T', " "));
                        ui.label(&entry.actor);
                        tag_grey(ui, action_label(&entry.action));
                        ui.vertical(|ui| {
                            ui.set_max_width(640.0);
                            detail(ui, entry);
                        });
                        ui.end_row();
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.page > 0, egui::Button::new("← 較新")).clicked() {
                        self.page -= 1;
                    }
                    ui.label(muted(format!("第 {} / {} 頁", self.page + 1, pages)));
                    if ui.add_enabled(self.page + 1 < pages, egui::Button::new("較舊 →")).clicked() {
                        self.page += 1;
                    }
                });
            });
        });
    }
}
