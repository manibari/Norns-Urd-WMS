//! 收貨建批 — two steps, mirroring how the paper 包材驗收單 is filled in.
//!
//! Step 1 is the delivery: what arrived, from whom, when, how much. Step 2 is
//! the inspection of it. On paper these are one row, but they are two acts by
//! two people at two moments, and one long form made the screen read as twelve
//! equally important fields when four are the delivery and the rest a verdict.
//!
//! 規格尺寸 is shown from the item master rather than typed: the inspection is
//! "does this match what we expect", which needs the expectation on screen.

use egui::{RichText, ScrollArea, Ui};
use serde_json::{Map, Value, json};

use super::theme::{BAD, MUTED, PRIMARY, bold_font, font};
use super::widgets::*;
use super::{Nav, Page, Shared};
use crate::auth::Session;
use crate::db::today_iso;
use crate::service::{self, FAIL, Item, ItemOptions, ItemOrder, LotInput, PASS};

const BY_FORM: &str = "單上數量";
const BY_BOX: &str = "箱數";

pub struct ReceivingPage {
    loaded: Option<u64>,
    items: Vec<Item>,
    options: ItemOptions,
    units: Vec<String>,
    signers: Vec<(String, String)>,
    step: u8,
    entered: usize,
    item_id: Option<i64>,
    item_name: String,
    model: String,
    spec: String,
    supplier: String,
    supplier_code: String,
    per_box: String,
    pack_unit: String,
    has_expiry: bool,
    receipt_date: Option<String>,
    qty_mode: String,
    qty: String,
    errors: Vec<String>,
    spec_result: String,
    spec_note: String,
    manufacture_date: Option<String>,
    expiry_date: Option<String>,
    appearance: String,
    appearance_note: String,
    colour: String,
    colour_note: String,
    verdict: String,
    remark: String,
    confirmed_by: Option<String>,
}

impl Default for ReceivingPage {
    fn default() -> Self {
        ReceivingPage {
            loaded: None,
            items: vec![],
            options: ItemOptions::default(),
            units: vec![],
            signers: vec![],
            step: 0,
            entered: 0,
            item_id: None,
            item_name: String::new(),
            model: String::new(),
            spec: String::new(),
            supplier: String::new(),
            supplier_code: String::new(),
            per_box: String::new(),
            pack_unit: String::new(),
            has_expiry: false,
            receipt_date: Some(today_iso()),
            qty_mode: BY_FORM.into(),
            qty: String::new(),
            errors: vec![],
            spec_result: "相符".into(),
            spec_note: String::new(),
            manufacture_date: None,
            expiry_date: None,
            appearance: "正常".into(),
            appearance_note: String::new(),
            colour: "正常".into(),
            colour_note: String::new(),
            verdict: PASS.into(),
            remark: String::new(),
            confirmed_by: None,
        }
    }
}

fn some(s: &str) -> Option<String> {
    Some(s.trim().to_string()).filter(|v| !v.is_empty())
}

impl ReceivingPage {
    fn load(&mut self, shared: &mut Shared) {
        let conn = &shared.store.conn;
        self.items = service::items(conn, ItemOrder::Sort).unwrap_or_default();
        self.options = service::item_options(conn).unwrap_or_default();
        self.units = service::values(conn, "pack_unit");
        self.signers = service::signers(conn).unwrap_or_default();
        self.loaded = Some(shared.generation);
    }

    fn picked(&self) -> Option<&Item> {
        self.item_id.and_then(|id| self.items.iter().find(|i| i.id == id))
    }

    fn on_item_change(&mut self) {
        let found = self.picked().cloned();
        let text = |v: &Option<String>| v.clone().unwrap_or_default();
        match found {
            Some(i) => {
                self.item_name = i.name.clone();
                self.model = text(&i.model);
                self.spec = text(&i.spec);
                self.supplier = text(&i.supplier);
                self.per_box = i.meters_per_box.map(|m| m.to_string()).unwrap_or_default();
                self.pack_unit = text(&i.pack_unit);
                self.supplier_code = text(&i.supplier_code);
                self.has_expiry = i.has_expiry;
            }
            None => {
                for f in [
                    &mut self.item_name,
                    &mut self.model,
                    &mut self.spec,
                    &mut self.supplier,
                    &mut self.per_box,
                    &mut self.pack_unit,
                    &mut self.supplier_code,
                ] {
                    f.clear();
                }
                self.has_expiry = false;
            }
        }
    }

    fn rate(&self) -> Option<i64> {
        self.per_box.trim().parse::<i64>().ok().filter(|&r| r > 0)
    }

    fn validate_delivery(&mut self) -> bool {
        self.errors.clear();
        if self.item_name.trim().is_empty() {
            self.errors.push("原物料名稱必填（選一個品項，或填名稱新增）".into());
        }
        match &self.receipt_date {
            None => self.errors.push("請填進貨日期".into()),
            Some(d) if *d > today_iso() => self.errors.push(format!("進貨日 {d} 在未來，請確認是否打錯")),
            _ => {}
        }
        let qty: Option<i64> = self.qty.trim().parse().ok().filter(|&q| q > 0);
        if qty.is_none() {
            self.errors.push(if self.qty_mode == BY_BOX { "請填箱數".into() } else { "請填單上數量".into() });
        }
        if self.qty_mode == BY_FORM && self.rate().is_none() {
            self.errors.push("這個品項沒有每箱數量，無法換算 —— 請改用箱數，或填每箱數量".into());
        }
        self.errors.is_empty()
    }

    fn validate_inspection(&mut self) -> bool {
        self.errors.clear();
        // "不正常" without a reason recorded a problem and lost what it was —
        // the note is the only part a recall can act on.
        if self.spec_result == "不符" && self.spec_note.trim().is_empty() {
            self.errors.push("規格尺寸不符，請說明哪裡不符".into());
        }
        if self.appearance == "不正常" && self.appearance_note.trim().is_empty() {
            self.errors.push("外觀不正常，請說明".into());
        }
        if self.colour == "不正常" && self.colour_note.trim().is_empty() {
            self.errors.push("顏色不正常，請說明".into());
        }
        if self.has_expiry && self.expiry_date.is_none() {
            self.errors.push("這個品項有效期，必須填有效期限".into());
        }
        self.errors.is_empty()
    }

    fn submit(&mut self, shared: &mut Shared, user: &Session) {
        if !self.validate_inspection() {
            return;
        }
        let qty: i64 = self.qty.trim().parse().unwrap_or(0);
        let new_item = self.item_id.is_none();
        let mut inspection = Map::new();
        let opt = |s: &str| some(s).map_or(Value::Null, Value::String);
        inspection.insert("規格尺寸".into(), json!(self.spec_result));
        inspection.insert("規格尺寸備註".into(), opt(&self.spec_note));
        inspection
            .insert("標示製造日期".into(), if self.manufacture_date.is_some() { json!("已填") } else { Value::Null });
        inspection.insert("標示有效日期".into(), if self.expiry_date.is_some() { json!("已填") } else { Value::Null });
        inspection.insert("外觀".into(), json!(self.appearance));
        inspection.insert("外觀備註".into(), opt(&self.appearance_note));
        inspection.insert("顏色".into(), json!(self.colour));
        inspection.insert("顏色備註".into(), opt(&self.colour_note));
        let input = LotInput {
            item_id: self.item_id,
            receipt_date: self.receipt_date.clone().unwrap_or_default(),
            supplier: some(&self.supplier),
            manufacture_date: self.manufacture_date.clone(),
            qty: (self.qty_mode == BY_BOX).then_some(qty),
            qty_entered: (self.qty_mode == BY_FORM).then_some(qty),
            item_name: some(&self.item_name),
            model: new_item.then(|| some(&self.model)).flatten(),
            spec: new_item.then(|| some(&self.spec)).flatten(),
            meters_per_box: new_item.then(|| self.rate()).flatten(),
            pack_unit: new_item.then(|| some(&self.pack_unit)).flatten(),
            has_expiry: new_item.then_some(self.has_expiry),
            supplier_code: new_item.then(|| some(&self.supplier_code)).flatten(),
            expiry_date: self.expiry_date.clone(),
            entered_unit: self.qty_mode.clone(),
            inspection,
            verdict: Some(self.verdict.clone()),
            confirmed_by: self.confirmed_by.clone(),
            remark: some(&self.remark),
            ..Default::default()
        };
        // A supplier typed at the counter goes back into the list.
        if let Some(s) = some(&self.supplier)
            && !self.options.supplier.contains(&s)
        {
            let _ = service::add_supplier_from_receiving(&shared.store, user, &s);
        }
        let result = service::create_lot(&shared.store, user, &input);
        let Some(made) = shared.act(result, None) else { return };
        shared.ok(format!(
            "{}：{} 箱（進貨日 {}）",
            if made.created_item { "已新增品項並建立批次" } else { "已建立批次" },
            made.qty,
            made.receipt_date
        ));
        if let Some(note) = made.conversion_note {
            shared.warn(note);
        }
        if made.verdict.as_deref() == Some(FAIL) {
            shared.warn("判定不合格：已留紀錄，但不計入可領用庫存，也不會被 FIFO 指到。");
        }
        if made.duplicate_lot_exists {
            shared.warn("同品項、同進貨日、同製造日已有一批 —— 確認不是重複登錄。");
        }
        if made.same_signer {
            shared.warn("記錄人與確認人是同一個人 —— 雙簽的意義就是兩個人，請確認。");
        }
        self.entered += 1;
        // Header fields repeat down the sheet — on paper they are literally 〃.
        let keep = (
            self.receipt_date.clone(),
            self.supplier.clone(),
            self.confirmed_by.clone(),
            self.qty_mode.clone(),
            self.entered,
        );
        *self = ReceivingPage::default();
        (self.receipt_date, self.supplier, self.confirmed_by, self.qty_mode, self.entered) = keep;
    }

    pub fn show(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        if self.loaded != Some(shared.generation) {
            self.load(shared);
        }
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(1100.0);
            page_title(ui, "收貨建批", Some("對應紙本「包材驗收單」：先填進貨資料，再逐項檢驗、雙簽。"));
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    steps(ui, self.step);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.entered > 0 {
                            if link(ui, "清空重填").clicked() {
                                *self = ReceivingPage { loaded: None, ..Default::default() };
                            }
                            ui.label(muted(format!("本次已登錄 {} 批", self.entered)));
                        }
                    });
                });
                divider(ui);
                if self.step == 0 {
                    self.delivery(ui);
                } else {
                    self.inspection(ui, user);
                }
                for e in &self.errors {
                    ui.colored_label(BAD, format!("• {e}"));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if self.step == 0 {
                        if primary(ui, "下一步：檢驗 →").clicked() && self.validate_delivery() {
                            self.step = 1;
                        }
                    } else {
                        if secondary(ui, "← 上一步").clicked() {
                            self.errors.clear();
                            self.step = 0;
                        }
                        if primary(ui, "＋ 建立批次").clicked() {
                            self.submit(shared, user);
                        }
                    }
                });
            });
            ui.add_space(12.0);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(muted("建好的批次在「庫存總覽」看，也在那裡修正或刪除。"));
                    if link(ui, "去庫存總覽").clicked() {
                        shared.nav = Some(Nav::Go(Page::Stock));
                    }
                });
            });
        });
    }

    fn delivery(&mut self, ui: &mut Ui) {
        let picked = self.picked().cloned();
        let w = ((ui.available_width() - 40.0) / 3.0).clamp(220.0, 340.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            field(ui, "品項", false, Some("找不到就留空，右邊填名稱新增"), |ui| {
                let mut options: Vec<(i64, String, bool)> = vec![];
                for i in &self.items {
                    let spec = i.spec.as_deref().map(|s| format!(" {s}")).unwrap_or_default();
                    options.push((i.id, format!("{}｜{}{}", i.label(), i.name, spec), true));
                }
                ui.horizontal(|ui| {
                    if search_select(ui, "recv-item", &mut self.item_id, &options, "選品項", w - 40.0) {
                        self.on_item_change();
                    }
                    if self.item_id.is_some()
                        && ui.add(egui::Button::new(muted("×")).frame(false)).on_hover_text("改成新品項").clicked()
                    {
                        self.item_id = None;
                        self.on_item_change();
                    }
                });
            });
            field(
                ui,
                "原物料名稱",
                true,
                Some(if picked.is_some() { "由品項決定" } else { "新品項就填這個" }),
                |ui| {
                    ui.add_enabled_ui(picked.is_none(), |ui| {
                        suggest_input(
                            ui,
                            "recv-name",
                            &mut self.item_name,
                            &self.options.material_name,
                            "選或直接輸入",
                            w,
                        )
                    });
                },
            );
            field(
                ui,
                "型號",
                false,
                Some(if picked.is_some() { "由品項決定" } else { "沒有型號可留空" }),
                |ui| {
                    ui.add_enabled_ui(picked.is_none(), |ui| text_input(ui, &mut self.model, "例 T6050BSW", w));
                },
            );
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            let locked_supplier = picked.as_ref().is_some_and(|p| p.supplier.is_some());
            field(
                ui,
                "進貨廠商",
                false,
                Some(if locked_supplier {
                    "品項預設廠商（換廠商請在基本資料改）"
                } else {
                    "選或輸入；新的會加進廠商清單"
                }),
                |ui| {
                    ui.add_enabled_ui(!locked_supplier, |ui| {
                        suggest_input(ui, "recv-supplier", &mut self.supplier, &self.options.supplier, "選廠商", w)
                    });
                },
            );
            field(
                ui,
                "進貨日期",
                true,
                Some(if self.entered > 0 { "沿用上一批（單上的〃）" } else { "到貨那天" }),
                |ui| {
                    date_field(ui, "recv-date", &mut self.receipt_date, false);
                },
            );
            let rate = self.rate();
            let unit = self.pack_unit.clone();
            let hint = if self.qty_mode == BY_FORM {
                match (rate, self.qty.trim().parse::<i64>().ok().filter(|&q| q > 0)) {
                    (Some(r), Some(q)) => {
                        let rem = if q % r > 0 {
                            format!("，剩 {}{unit} 對不上整箱", urdwms_core::units::thousands(q % r))
                        } else {
                            String::new()
                        };
                        format!("= {} 箱{rem}", q / r)
                    }
                    (Some(r), None) => format!("每箱 {}{unit}，輸入單上數量自動換算", urdwms_core::units::thousands(r)),
                    (None, _) => "此品項未設每箱數量，請改用箱數".into(),
                }
            } else {
                "收進來幾箱".into()
            };
            field(ui, "數量", true, Some(&hint), |ui| {
                ui.horizontal(|ui| {
                    number_input(ui, &mut self.qty, if self.qty_mode == BY_FORM { "例 3600" } else { "例 4" }, 120.0);
                    segmented(ui, &mut self.qty_mode, &[BY_FORM, BY_BOX]);
                });
            });
        });
        if picked.is_none() {
            ui.add_space(10.0);
            ui.label(RichText::new("新品項設定（可事後在基本資料調整）").font(bold_font(15.0)).color(MUTED));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 20.0;
                field(ui, "規格", false, None, |ui| {
                    suggest_input(ui, "recv-spec", &mut self.spec, &self.options.spec, "例 340mm x 900M", 200.0)
                });
                field(ui, "每箱數量", false, Some("單上數量 ÷ 這個 = 箱數"), |ui| {
                    number_input(ui, &mut self.per_box, "例 600", 120.0)
                });
                field(ui, "單位", false, None, |ui| {
                    suggest_input(ui, "recv-unit", &mut self.pack_unit, &self.units, "米／張／包", 110.0)
                });
                field(ui, "箱上完整料號", false, Some("影像辨識用來對回品項"), |ui| {
                    text_input(ui, &mut self.supplier_code, "例 2003.T7320BC-340X900-P1", 280.0)
                });
                field(ui, "有效期", false, Some("有的話收貨必須填有效期限"), |ui| {
                    ui.horizontal(|ui| {
                        toggle(ui, &mut self.has_expiry);
                        ui.label(if self.has_expiry { "有" } else { "無" });
                    })
                });
            });
        }
    }

    fn inspection(&mut self, ui: &mut Ui, user: &Session) {
        let label = some(&self.model).unwrap_or_else(|| self.item_name.clone());
        alert(ui, Kind::Info, &format!("檢驗：{label}"), "對應驗收單的「檢驗項目」欄，一項一項往下確認。");
        ui.add_space(8.0);
        let spec = some(&self.spec).unwrap_or_else(|| "（未登記規格）".into());
        // Two explicit options rather than a checkbox: an unticked box cannot
        // say whether the item failed or was never looked at.
        row(ui, Some(1), "規格尺寸", false, Some("核對實物與主檔登記的規格"), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(spec).font(font(16.0)).background_color(super::theme::SUBTLE));
                segmented(ui, &mut self.spec_result, &["相符", "不符"]);
            });
            if self.spec_result == "不符" {
                text_input(ui, &mut self.spec_note, "哪裡不符？", 360.0);
            }
        });
        row(
            ui,
            Some(2),
            "製造日期",
            false,
            Some("先進先出就是照這個排序 —— 不填的話這批會被排到最後"),
            |ui| {
                date_field(ui, "recv-made", &mut self.manufacture_date, true);
            },
        );
        let needs = self.has_expiry;
        row(
            ui,
            Some(3),
            "有效期限",
            needs,
            Some(if needs {
                "這個品項有效期，必須填"
            } else {
                "此品項未標記有效期，留空是正常的"
            }),
            |ui| {
                date_field(ui, "recv-expiry", &mut self.expiry_date, true);
            },
        );
        row(ui, Some(4), "外觀", false, None, |ui| {
            segmented(ui, &mut self.appearance, &["正常", "不正常"]);
            if self.appearance == "不正常" {
                text_input(ui, &mut self.appearance_note, "哪裡不正常？", 360.0);
            }
        });
        row(ui, Some(5), "顏色", false, None, |ui| {
            segmented(ui, &mut self.colour, &["正常", "不正常"]);
            if self.colour == "不正常" {
                text_input(ui, &mut self.colour_note, "哪裡不正常？", 360.0);
            }
        });
        row(ui, Some(6), "判定", false, Some("不合格會留紀錄，但不進可領用庫存"), |ui| {
            segmented(ui, &mut self.verdict, &[PASS, FAIL]);
        });
        row(ui, None, "備註", false, None, |ui| {
            text_input(ui, &mut self.remark, "不合格請寫原因", 360.0);
        });

        let failed: Vec<&str> = [
            (self.spec_result == "不符").then_some("規格尺寸"),
            (self.appearance == "不正常").then_some("外觀"),
            (self.colour == "不正常").then_some("顏色"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !failed.is_empty() && self.verdict == PASS {
            alert(
                ui,
                Kind::Warning,
                &format!("{}有異常，判定卻是合格", failed.join("、")),
                "這樣寫是可以的（有些瑕疵不影響使用），但請確認是刻意的 —— 異常內容會留在紀錄裡。",
            );
        }
        if self.verdict == FAIL {
            alert(
                ui,
                Kind::Warning,
                "判定不合格：會留紀錄，但不進可領用庫存",
                "這批仍然存在系統裡（誰收的、什麼時候、為什麼不合格），只是不計入在庫、FIFO 不會指到、也領不出來。請在備註寫原因。",
            );
        }

        ui.add_space(14.0);
        ui.label(RichText::new("簽核").font(bold_font(16.0)).color(PRIMARY));
        // 記錄人 is who is signed in: dual sign-off is the form's control, and
        // a dropdown of names would let anyone sign as anyone.
        row(ui, None, "記錄人", false, Some("登入身分，不能改"), |ui| {
            ui.label(strong(&user.name));
        });
        let same = self.confirmed_by.as_deref() == Some(user.name.as_str());
        row(
            ui,
            None,
            "確認人",
            false,
            Some(if same {
                "跟記錄人是同一個人 —— 雙簽的意義就是兩個人"
            } else {
                "覆核這批的人"
            }),
            |ui| {
                let names: Vec<String> =
                    self.signers.iter().filter(|(n, _)| *n != user.name).map(|(n, t)| format!("{n}（{t}）")).collect();
                let mut shown = self
                    .confirmed_by
                    .as_ref()
                    .and_then(|c| names.iter().find(|n| n.starts_with(&format!("{c}（"))).cloned());
                if select(ui, "recv-confirm", &mut shown, &names, "選覆核的人", 240.0, true) {
                    self.confirmed_by = shown.map(|s| s.split('（').next().unwrap_or("").to_string());
                }
            },
        );
    }
}

fn steps(ui: &mut Ui, current: u8) {
    ui.horizontal(|ui| {
        for (i, name) in ["進貨資料", "檢驗項目"].iter().enumerate() {
            let active = i as u8 == current;
            let done = (i as u8) < current;
            let color = if active || done { PRIMARY } else { MUTED };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
            if active || done {
                ui.painter().circle_filled(rect.center(), 15.0, color);
            } else {
                ui.painter().circle_stroke(rect.center(), 14.0, egui::Stroke::new(1.5, color));
            }
            let mark = if done { "✓".to_string() } else { (i + 1).to_string() };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                mark,
                font(15.0),
                if active || done { egui::Color32::WHITE } else { color },
            );
            ui.label(RichText::new(*name).font(if active { bold_font(16.0) } else { font(16.0) }).color(if active {
                super::theme::TEXT
            } else {
                MUTED
            }));
            if i == 0 {
                let (r, _) = ui.allocate_exact_size(egui::vec2(60.0, 30.0), egui::Sense::hover());
                ui.painter().hline(
                    r.x_range(),
                    r.center().y,
                    egui::Stroke::new(1.5, if done { PRIMARY } else { super::theme::BORDER }),
                );
            }
        }
    });
}

/// One numbered line of the inspection checklist: number, label, control, hint.
fn row(ui: &mut Ui, n: Option<u8>, label: &str, required: bool, hint: Option<&str>, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.add_sized([26.0, 34.0], egui::Label::new(muted(n.map(|n| n.to_string()).unwrap_or_default())));
        ui.allocate_ui(egui::vec2(110.0, 34.0), |ui| {
            ui.set_width(110.0);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if required {
                    ui.label(RichText::new("*").color(BAD));
                }
                ui.label(RichText::new(label).font(font(16.0)));
            });
        });
        ui.vertical(|ui| {
            add(ui);
            if let Some(h) = hint {
                ui.label(small(h));
            }
        });
    });
    divider(ui);
}
