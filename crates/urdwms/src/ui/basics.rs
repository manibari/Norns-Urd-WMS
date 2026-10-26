//! 基本資料 — every dropdown in the app is fed from here.
//!
//! First in the pipeline: without a 型號 and its per-box rate, receiving
//! cannot record a delivery; without machines and products, issuing cannot
//! record a draw. A second factory changes these tables, not the code (US-11).
//!
//! A 型號 carries its 廠商 / 原物料名稱 / 規格 rather than those being picked
//! separately at receiving — letting someone combine them freely allows
//! combinations that do not exist.

use std::collections::HashMap;

use egui::{RichText, ScrollArea, Ui};

use super::Shared;
use super::theme::{BAD, MUTED, OK, PRIMARY, bold_font, font};
use super::widgets::*;
use crate::auth::Session;
use crate::service::{self, CATEGORIES, DictEntry, Item, ItemInput, ItemOptions, ItemOrder};

#[derive(Default)]
pub struct BasicsPage {
    loaded: Option<u64>,
    tab: usize,
    items: Vec<Item>,
    options: ItemOptions,
    units: Vec<String>,
    /// Cell buffers: what is being typed, saved when the cell loses focus.
    cells: HashMap<(i64, &'static str), String>,
    draft: Draft,
    entries: Vec<DictEntry>,
    dict_cells: HashMap<i64, String>,
    dict_draft: String,
    deleting_item: Option<Item>,
    deleting_entry: Option<DictEntry>,
}

#[derive(Default)]
struct Draft {
    name: String,
    model: String,
    spec: String,
    supplier: String,
    per_box: String,
    unit: String,
}

const TEXT_FIELDS: [&str; 7] = ["name", "model", "spec", "supplier", "supplier_code", "meters_per_box", "safety_stock"];

fn cell_value(item: &Item, field: &str) -> String {
    let text = |v: &Option<String>| v.clone().unwrap_or_default();
    match field {
        "name" => item.name.clone(),
        "model" => text(&item.model),
        "spec" => text(&item.spec),
        "supplier" => text(&item.supplier),
        "supplier_code" => text(&item.supplier_code),
        "pack_unit" => text(&item.pack_unit),
        "meters_per_box" => item.meters_per_box.map(|v| v.to_string()).unwrap_or_default(),
        "safety_stock" => item.safety_stock.to_string(),
        _ => String::new(),
    }
}

fn patch_for(field: &str, value: &str) -> Result<ItemInput, String> {
    let v = value.trim().to_string();
    let mut p = ItemInput::default();
    let int = |s: &str, empty: i64| -> Result<i64, String> {
        if s.is_empty() { Ok(empty) } else { s.parse::<i64>().map_err(|_| "請填整數".to_string()) }
    };
    match field {
        "name" => p.name = Some(v),
        "model" => p.model = Some(v),
        "spec" => p.spec = Some(v),
        "supplier" => p.supplier = Some(v),
        "supplier_code" => p.supplier_code = Some(v),
        "pack_unit" => p.pack_unit = Some(v),
        "meters_per_box" => p.meters_per_box = Some(int(&v, 0)?),
        "safety_stock" => p.safety_stock = Some(int(&v, 0)?),
        _ => {}
    }
    Ok(p)
}

impl BasicsPage {
    fn load(&mut self, shared: &mut Shared) {
        let conn = &shared.store.conn;
        self.items = service::items(conn, ItemOrder::Sort).unwrap_or_default();
        self.options = service::item_options(conn).unwrap_or_default();
        self.units = service::values(conn, "pack_unit");
        self.cells.clear();
        for item in &self.items {
            for f in TEXT_FIELDS.iter().chain(["pack_unit"].iter()) {
                self.cells.insert((item.id, f), cell_value(item, f));
            }
        }
        if self.tab > 0 {
            let (cat, _) = CATEGORIES[self.tab - 1];
            self.entries = service::dictionary(conn, cat, true).unwrap_or_default();
            self.dict_cells = self.entries.iter().map(|e| (e.id, e.value.clone())).collect();
        }
        self.loaded = Some(shared.generation);
    }

    fn save_cell(&mut self, shared: &mut Shared, user: &Session, item: &Item, field: &'static str) {
        let value = self.cells.get(&(item.id, field)).cloned().unwrap_or_default();
        if value.trim() == cell_value(item, field).trim() {
            return;
        }
        match patch_for(field, &value) {
            Ok(p) => {
                if shared.act(service::update_item(&shared.store, user, item.id, &p), None).is_none() {
                    self.cells.insert((item.id, field), cell_value(item, field));
                }
            }
            Err(e) => {
                shared.error(e);
                self.cells.insert((item.id, field), cell_value(item, field));
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        if self.loaded != Some(shared.generation) {
            self.load(shared);
        }
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            page_title(
                ui,
                "基本資料",
                Some("收貨與領用畫面的每一個下拉選單都從這裡來。換一家工廠就是換這幾張表的內容，不用改程式。"),
            );
            card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let mut tabs = vec![format!("品項（{}）", self.items.len())];
                    tabs.extend(CATEGORIES.iter().map(|(_, l)| l.to_string()));
                    for (i, t) in tabs.iter().enumerate() {
                        let selected = self.tab == i;
                        let text = RichText::new(t)
                            .font(if selected { bold_font(16.0) } else { font(16.0) })
                            .color(if selected { PRIMARY } else { super::theme::TEXT });
                        let r = ui.add(egui::Button::new(text).frame(false));
                        if selected {
                            let rect = r.rect;
                            ui.painter().hline(rect.x_range(), rect.bottom() + 3.0, egui::Stroke::new(2.5, PRIMARY));
                        }
                        if r.clicked() && !selected {
                            self.tab = i;
                            self.loaded = None;
                        }
                    }
                });
                divider(ui);
                if self.tab == 0 {
                    self.items_tab(ui, shared, user);
                } else {
                    self.dictionary_tab(ui, shared, user);
                }
            });
        });
        self.delete_modals(ui, shared, user);
    }

    fn items_tab(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        alert(
            ui,
            Kind::Info,
            "必填的是原物料名稱，型號可以沒有",
            "驗收單上「脫氧劑」那列就沒有型號，所以型號不是識別。「每箱數量」是驗收單數量換算成箱的依據，單位隨品項而定（膜是米、袋是張、劑是包）。只有有型號（或登記過箱上完整料號）的品項，領用時才能靠影像辨識認出來；其餘一律人工選。",
        );
        ui.add_space(6.0);
        let unset = self.items.iter().filter(|i| i.meters_per_box.is_none()).count();
        ui.horizontal(|ui| {
            ui.label(muted("直接在格子裡改，離開欄位就存。這裡只有基本資料，不看庫存 —— 現在有多少去「庫存總覽」。"));
            if unset > 0 {
                ui.label(small(format!("｜{unset} 項未設每箱數量（只能用箱數收貨）")));
            }
        });
        ui.add_space(6.0);
        let items = self.items.clone();
        let mut moved: Option<(usize, usize)> = None;
        egui::ScrollArea::horizontal().id_salt("items-h").show(ui, |ui| {
            egui::Grid::new("items").striped(true).spacing([8.0, 8.0]).min_row_height(36.0).show(ui, |ui| {
                for h in [
                    "順序",
                    "原物料名稱 *",
                    "型號",
                    "影像辨識",
                    "規格",
                    "廠商",
                    "箱上完整料號",
                    "每箱數量",
                    "單位",
                    "有效期",
                    "安全水位",
                    "",
                ] {
                    ui.label(RichText::new(h).font(font(14.0)).color(MUTED));
                }
                ui.end_row();
                for (idx, item) in items.iter().enumerate() {
                    // Row order is hand-arranged and shared: films together, the
                    // daily items on top.
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ui.add_enabled(idx > 0, egui::Button::new("▲").small()).on_hover_text("往上移").clicked()
                        {
                            moved = Some((idx, idx - 1));
                        }
                        if ui
                            .add_enabled(idx + 1 < items.len(), egui::Button::new("▼").small())
                            .on_hover_text("往下移")
                            .clicked()
                        {
                            moved = Some((idx, idx + 1));
                        }
                    });
                    for (field, width, hint) in [("name", 150.0, ""), ("model", 100.0, "（無）")] {
                        let buf = self.cells.entry((item.id, field)).or_default();
                        if text_input(ui, buf, hint, width).lost_focus() {
                            self.save_cell(shared, user, item, field);
                        }
                    }
                    if item.matchable() {
                        let mut on = item.use_recognition;
                        ui.horizontal(|ui| {
                            if toggle(ui, &mut on).changed() {
                                let p = ItemInput { use_recognition: Some(on), ..Default::default() };
                                shared.act(service::update_item(&shared.store, user, item.id, &p), None);
                            }
                            if on {
                                tag(ui, "開啟", PRIMARY)
                            } else {
                                tag_grey(ui, "關閉")
                            };
                        });
                    } else {
                        ui.label(small("需型號或料號")).on_hover_text("填了型號或箱上完整料號，才有東西可以跟標籤對映");
                    }
                    let spec = self.cells.entry((item.id, "spec")).or_default();
                    if text_input(ui, spec, "—", 110.0).lost_focus() {
                        self.save_cell(shared, user, item, "spec");
                    }
                    let supplier = self.cells.entry((item.id, "supplier")).or_default();
                    if suggest_input(ui, ("sup", item.id), supplier, &self.options.supplier, "—", 110.0).lost_focus()
                    {
                        self.save_cell(shared, user, item, "supplier");
                    }
                    let code = self.cells.entry((item.id, "supplier_code")).or_default();
                    if text_input(ui, code, "未登記", 180.0).lost_focus() {
                        self.save_cell(shared, user, item, "supplier_code");
                    }
                    let per_box = self.cells.entry((item.id, "meters_per_box")).or_default();
                    if number_input(ui, per_box, "未設", 64.0).lost_focus() {
                        self.save_cell(shared, user, item, "meters_per_box");
                    }
                    let unit = self.cells.entry((item.id, "pack_unit")).or_default();
                    if suggest_input(ui, ("unit", item.id), unit, &self.units, "—", 56.0).lost_focus() {
                        self.save_cell(shared, user, item, "pack_unit");
                    }
                    let mut expiry = item.has_expiry;
                    ui.horizontal(|ui| {
                        if toggle(ui, &mut expiry).changed() {
                            let p = ItemInput { has_expiry: Some(expiry), ..Default::default() };
                            shared.act(service::update_item(&shared.store, user, item.id, &p), None);
                        }
                        ui.label(small(if expiry { "有" } else { "無" })).on_hover_text(if expiry {
                            "收貨時必須填有效期限"
                        } else {
                            "收貨不會要求有效期限，也不發效期提醒"
                        });
                    });
                    let safety = self.cells.entry((item.id, "safety_stock")).or_default();
                    if number_input(ui, safety, "0", 56.0).lost_focus() {
                        self.save_cell(shared, user, item, "safety_stock");
                    }
                    let used = item.on_hand > 0
                        || item.open_lots > 0
                        || item.rejected_qty > 0
                        || item.last_receipt_date.is_some();
                    let r = link_danger(ui, "刪除", !used);
                    if used {
                        r.on_disabled_hover_text("已有進貨或領用紀錄，刪掉那些紀錄會失去對應");
                    } else if r.clicked() {
                        self.deleting_item = Some(item.clone());
                    }
                    ui.end_row();
                }
            });
        });
        if let Some((from, to)) = moved {
            let mut ids: Vec<i64> = items.iter().map(|i| i.id).collect();
            ids.swap(from, to);
            shared.act(service::reorder_items(&shared.store, user, &ids), None);
        }

        ui.add_space(10.0);
        ui.label(RichText::new("新增品項").font(bold_font(16.0)));
        ui.horizontal_wrapped(|ui| {
            let d = &mut self.draft;
            text_input(ui, &mut d.name, "原物料名稱（必填）", 180.0);
            text_input(ui, &mut d.model, "型號（可留空）", 130.0);
            suggest_input(ui, "draft-spec", &mut d.spec, &self.options.spec, "規格", 130.0);
            suggest_input(ui, "draft-sup", &mut d.supplier, &self.options.supplier, "廠商", 130.0);
            number_input(ui, &mut d.per_box, "每箱數量", 90.0);
            suggest_input(ui, "draft-unit", &mut d.unit, &self.units, "單位", 80.0);
            if primary(ui, "＋ 新增").clicked() {
                let input = ItemInput {
                    name: Some(d.name.clone()),
                    model: Some(d.model.clone()),
                    spec: Some(d.spec.clone()),
                    supplier: Some(d.supplier.clone()),
                    meters_per_box: d.per_box.trim().parse().ok(),
                    pack_unit: Some(d.unit.clone()),
                    ..Default::default()
                };
                let label = if d.model.trim().is_empty() { d.name.clone() } else { d.model.clone() };
                if shared
                    .act(service::create_item(&shared.store, user, &input), Some(&format!("已新增 {label}")))
                    .is_some()
                {
                    self.draft = Draft::default();
                }
            }
        });
    }

    fn dictionary_tab(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        let (category, label) = CATEGORIES[self.tab - 1];
        let can = user.can("dictionary.manage");
        if !can {
            alert(ui, Kind::Info, "只有系統管理者可以改選項", "你可以看，但新增、改名、停用要請系統管理者。");
        }
        ui.add_enabled_ui(can, |ui| {
            ui.horizontal(|ui| {
                let r = text_input(ui, &mut self.dict_draft, &format!("新增{label}"), 300.0);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (primary(ui, "＋ 新增").clicked() || enter) && !self.dict_draft.trim().is_empty() {
                    let value = self.dict_draft.trim().to_string();
                    if let Some(revived) = shared.act(service::add_entry(&shared.store, user, category, &value), None) {
                        shared.ok(if revived {
                            format!("已重新啟用「{value}」")
                        } else {
                            format!("已新增「{value}」")
                        });
                        self.dict_draft.clear();
                    }
                }
            });
            ui.add_space(6.0);
            if self.entries.is_empty() {
                return empty(ui, &format!("尚無{label}"));
            }
            egui::Grid::new("dict").striped(true).spacing([24.0, 8.0]).min_row_height(36.0).show(ui, |ui| {
                for h in [label, "狀態", ""] {
                    ui.label(RichText::new(h).font(font(14.0)).color(MUTED));
                }
                ui.end_row();
                for entry in self.entries.clone() {
                    let buf = self.dict_cells.entry(entry.id).or_insert_with(|| entry.value.clone());
                    let r = ui.add_enabled_ui(entry.active, |ui| text_input(ui, buf, "", 280.0)).inner;
                    if r.lost_focus() && buf.trim() != entry.value && !buf.trim().is_empty() {
                        let value = buf.clone();
                        if shared
                            .act(service::rename_entry(&shared.store, user, entry.id, &value), Some("已更名"))
                            .is_none()
                        {
                            self.dict_cells.insert(entry.id, entry.value.clone());
                        }
                    }
                    let mut active = entry.active;
                    ui.horizontal(|ui| {
                        if toggle(ui, &mut active).changed() {
                            shared.act(service::set_entry_active(&shared.store, user, entry.id, active), None);
                        }
                        if entry.active {
                            tag(ui, "啟用中", OK)
                        } else {
                            tag_grey(ui, "已停用")
                        };
                    });
                    if link_danger(ui, "刪除", true).clicked() {
                        self.deleting_entry = Some(entry.clone());
                    }
                    ui.end_row();
                }
            });
        });
        ui.add_space(10.0);
        ui.label(small("停用：不再出現在下拉選單，但保留在清單裡，隨時可以再啟用 —— 用在「這個先不用了」。"));
        ui.label(small(
            "刪除：整個移除。既有紀錄不受影響（存的是當時選的文字，不是參照）—— 用在「這一筆本來就是打錯的」。",
        ));
    }

    fn delete_modals(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        if let Some(item) = self.deleting_item.clone() {
            let mut close = false;
            egui::Modal::new(egui::Id::unique("del-item")).show(ui.ctx(), |ui| {
                ui.set_width(400.0);
                ui.label(RichText::new(format!("刪除 {}？", item.label())).font(bold_font(20.0)));
                ui.label(muted("這個品項沒有任何進貨或領用紀錄，可以安全刪除。"));
                ui.horizontal(|ui| {
                    if danger(ui, "刪除").clicked() {
                        shared.act(service::delete_item(&shared.store, user, item.id), Some("已刪除"));
                        close = true;
                    }
                    if secondary(ui, "取消").clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.deleting_item = None;
            }
        }
        if let Some(entry) = self.deleting_entry.clone() {
            let mut close = false;
            egui::Modal::new(egui::Id::unique("del-entry")).show(ui.ctx(), |ui| {
                ui.set_width(420.0);
                ui.label(RichText::new(format!("刪除「{}」？", entry.value)).font(bold_font(20.0)));
                ui.label(muted(
                    "既有紀錄不受影響（存的是文字，不是參照），只是不再出現在選單。只是暫時不用的話，停用就好。",
                ));
                ui.horizontal(|ui| {
                    if danger(ui, "刪除").clicked() {
                        shared.act(
                            service::delete_entry(&shared.store, user, entry.id),
                            Some(&format!("已刪除「{}」", entry.value)),
                        );
                        close = true;
                    }
                    if secondary(ui, "取消").clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.deleting_entry = None;
            }
        }
        let _ = BAD;
    }
}
