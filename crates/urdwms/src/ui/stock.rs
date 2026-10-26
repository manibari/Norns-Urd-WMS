//! 庫存總覽 — the landing screen.
//!
//! It answers the question people walk up with: what have we got, and which
//! box should be taken next. So the FIFO target is on the main row rather than
//! hidden behind an expand — knowing there are 9 boxes is useless if you still
//! have to dig to find which one is next.
//!
//! Rejected stock is shown separately, never summed into available: it is on
//! the shelf but must not be drawn, and one number covering both would be the
//! wrong number for every purpose.

use std::collections::{HashMap, HashSet};

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::theme::{BAD, MUTED, OK, ORANGE, PRIMARY, WARN, bold_font, font};
use super::widgets::*;
use super::{Nav, Page, Shared, charts};
use crate::auth::Session;
use crate::service::{self, Alerts, FAIL, Item, ItemOrder, Lot, LotPatch, PASS};

#[derive(Default)]
pub struct StockPage {
    loaded: Option<u64>,
    items: Vec<Item>,
    lots: Vec<Lot>,
    alerts: Option<Alerts>,
    suppliers: Vec<String>,
    expanded: HashSet<i64>,
    editing: Option<LotEdit>,
    deleting: Option<Lot>,
}

struct LotEdit {
    lot: Lot,
    receipt_date: Option<String>,
    manufacture_date: Option<String>,
    expiry_date: Option<String>,
    supplier: String,
    qty_on_hand: String,
    verdict: String,
    remark: String,
}

impl LotEdit {
    fn new(lot: &Lot) -> Self {
        LotEdit {
            lot: lot.clone(),
            receipt_date: Some(lot.receipt_date.clone()),
            manufacture_date: lot.manufacture_date.clone(),
            expiry_date: lot.expiry_date.clone(),
            supplier: lot.supplier.clone().unwrap_or_default(),
            qty_on_hand: lot.qty_on_hand.to_string(),
            verdict: lot.verdict.clone().unwrap_or_else(|| PASS.into()),
            remark: lot.remark.clone().unwrap_or_default(),
        }
    }

    /// Only what actually changed goes into the patch — and so into the audit
    /// trail's before/after.
    fn patch(&self) -> Result<LotPatch, String> {
        let l = &self.lot;
        let differs = |a: &Option<String>, b: &Option<String>| a != b;
        let qty: i64 = self.qty_on_hand.trim().parse().map_err(|_| "在庫數量請填整數".to_string())?;
        Ok(LotPatch {
            receipt_date: differs(&self.receipt_date, &Some(l.receipt_date.clone()))
                .then(|| self.receipt_date.clone().unwrap_or_default()),
            manufacture_date: differs(&self.manufacture_date, &l.manufacture_date)
                .then(|| self.manufacture_date.clone().unwrap_or_default()),
            expiry_date: differs(&self.expiry_date, &l.expiry_date)
                .then(|| self.expiry_date.clone().unwrap_or_default()),
            supplier: (Some(self.supplier.trim().to_string()).filter(|s| !s.is_empty()) != l.supplier)
                .then(|| self.supplier.clone()),
            qty_on_hand: (qty != l.qty_on_hand).then_some(qty),
            verdict: (Some(self.verdict.clone()) != l.verdict).then(|| self.verdict.clone()),
            remark: (Some(self.remark.trim().to_string()).filter(|s| !s.is_empty()) != l.remark)
                .then(|| self.remark.clone()),
        })
    }
}

impl StockPage {
    fn load(&mut self, shared: &mut Shared) {
        let conn = &shared.store.conn;
        let loaded = service::items(conn, ItemOrder::Recent).and_then(|i| Ok((i, service::lots(conn, None)?)));
        match loaded {
            Ok((i, l)) => {
                self.items = i;
                self.lots = l;
            }
            Err(e) => shared.error(format!("庫存載入失敗：{e}")),
        }
        let conn = &shared.store.conn;
        self.alerts = service::alerts(conn).ok();
        self.suppliers = service::item_options(conn).map(|o| o.supplier).unwrap_or_default();
        self.loaded = Some(shared.generation);
    }

    pub fn show(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        if self.loaded != Some(shared.generation) {
            self.load(shared);
        }
        let mut by_item: HashMap<i64, Vec<&Lot>> = HashMap::new();
        for lot in &self.lots {
            by_item.entry(lot.item_id).or_default().push(lot);
        }

        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                page_title(ui, "庫存總覽", None);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if user.can("lot.create") && primary(ui, "📦 去收貨建批").clicked() {
                        shared.nav = Some(Nav::Go(Page::Receiving));
                    }
                    if user.can("issue.create") && secondary(ui, "📷 領用登錄").clicked() {
                        shared.nav = Some(Nav::Go(Page::Issue));
                    }
                });
            });

            let total: i64 = self.items.iter().map(|i| i.on_hand).sum();
            let rejected: i64 = self.items.iter().map(|i| i.rejected_qty).sum();
            let low = self.alerts.as_ref().map_or(0, |a| a.low_stock.len());
            let expiring = self.alerts.as_ref().map_or(0, |a| a.expiring.len());
            card(ui, |ui| {
                columns_responsive(ui, 4, 180.0, |ui, i| match i {
                    0 => stat(ui, "品項", self.items.len(), "項", None),
                    1 => stat(ui, "可領用", total, "箱", None),
                    2 => stat(ui, "低於安全水位", low, "項", (low > 0).then_some(WARN_DARK_TEXT)),
                    _ => stat(ui, "效期將屆", expiring, "批", (expiring > 0).then_some(WARN_DARK_TEXT)),
                });
                if rejected > 0 {
                    ui.add_space(4.0);
                    ui.colored_label(
                        BAD,
                        format!("另有 {rejected} 箱驗收不合格的庫存，不計入可領用，也不會被 FIFO 指到。"),
                    );
                }
            });
            ui.add_space(12.0);

            let window = self.alerts.as_ref().map_or(60, |a| a.thresholds.expiry_days);
            columns_responsive(ui, 3, 320.0, |ui, i| {
                let titles = ["夠不夠用 —— 在庫 vs 安全水位", "老不老 —— 庫齡（依製造日）", "來不來得及 —— 效期倒數"];
                titled_card(
                    ui,
                    RichText::new(titles[i]).font(bold_font(16.0)),
                    |_| {},
                    |ui| match i {
                        0 => charts::stock_vs_safety(ui, &self.items),
                        1 => charts::age_distribution(ui, &self.lots),
                        _ => charts::expiry_countdown(ui, &self.lots, window),
                    },
                );
            });
            ui.add_space(12.0);

            titled_card(
                ui,
                "各品項庫存（最近進貨的在最上面）",
                |ui| {
                    ui.label(muted("點一列看批次"));
                },
                |ui| {
                    if self.items.is_empty() {
                        return empty(ui, "尚無品項 —— 先到「基本資料」或「收貨建批」建立");
                    }
                    for item in &self.items {
                        let rows = by_item.get(&item.id).cloned().unwrap_or_default();
                        let open = self.expanded.contains(&item.id);
                        let clicked = item_row(ui, item, &rows, open, user, shared);
                        if clicked {
                            if open {
                                self.expanded.remove(&item.id);
                            } else {
                                self.expanded.insert(item.id);
                            }
                        }
                        if open {
                            lot_table(ui, &rows, user, &mut self.editing, &mut self.deleting);
                        }
                        ui.add_space(6.0);
                    }
                },
            );
        });

        self.edit_modal(ui, shared, user);
        self.delete_modal(ui, shared, user);
    }

    fn edit_modal(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        let Some(edit) = &mut self.editing else { return };
        let mut close = false;
        let mut save = false;
        let suppliers = self.suppliers.clone();
        egui::Modal::new(egui::Id::unique("lot-edit")).show(ui.ctx(), |ui| {
            ui.set_width(640.0);
            ui.label(
                RichText::new(format!("編輯批次 #{}　{}", edit.lot.id, edit.lot.item_label())).font(bold_font(20.0)),
            );
            let note = if edit.lot.draw_count > 0 {
                format!(
                    "這批已有 {} 筆領用紀錄。改製造日會改變 FIFO 順序，改數量會影響既有帳 —— 變更前後值都會留紀錄。",
                    edit.lot.draw_count
                )
            } else {
                "改動前後的值都會寫入系統日誌，不會靜默變更。".into()
            };
            alert(ui, Kind::Info, "變更會寫入系統日誌", &note);
            ui.add_space(6.0);
            egui::Grid::new("lot-edit-grid").num_columns(2).spacing([16.0, 12.0]).show(ui, |ui| {
                ui.label("進貨日期");
                date_field(ui, "edit-receipt", &mut edit.receipt_date, false);
                ui.end_row();
                ui.label("製造日期");
                ui.vertical(|ui| {
                    date_field(ui, "edit-made", &mut edit.manufacture_date, true);
                    ui.label(small("FIFO 依製造日排序"));
                });
                ui.end_row();
                ui.label("有效期限");
                date_field(ui, "edit-expiry", &mut edit.expiry_date, true);
                ui.end_row();
                ui.label("進貨廠商");
                suggest_input(ui, "edit-supplier", &mut edit.supplier, &suppliers, "選或輸入廠商", 260.0);
                ui.end_row();
                ui.label("在庫數量（箱）");
                number_input(ui, &mut edit.qty_on_hand, "", 120.0);
                ui.end_row();
                ui.label("判定");
                segmented(ui, &mut edit.verdict, &[PASS, FAIL]);
                ui.end_row();
                ui.label("備註");
                text_input(ui, &mut edit.remark, "", 360.0);
                ui.end_row();
            });
            ui.add_space(10.0);
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
            match edit.patch() {
                Err(e) => shared.error(e),
                Ok(patch) if patch_is_empty(&patch) => close = true,
                Ok(patch) => {
                    let id = edit.lot.id;
                    let result = service::update_lot(&shared.store, user, id, &patch);
                    if let Some(drawn) = shared.act(result, Some("已更新，變更前後值已寫入系統日誌")) {
                        if drawn > 0 {
                            shared.warn(format!("這批已有 {drawn} 筆領用紀錄，改動會影響既有帳。"));
                        }
                        close = true;
                    }
                }
            }
        }
        if close {
            self.editing = None;
        }
    }

    fn delete_modal(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        let Some(lot) = &self.deleting else { return };
        let mut close = false;
        let mut confirm = false;
        egui::Modal::new(egui::Id::unique("lot-delete")).show(ui.ctx(), |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(format!("刪除批次 #{}？", lot.id)).font(bold_font(20.0)));
            ui.label(format!("{}｜進貨 {}｜{} 箱", lot.item_label(), lot.receipt_date, lot.qty_on_hand));
            ui.label(muted("沒有任何領用紀錄，可以安全刪除。刪除會寫入系統日誌。"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if danger(ui, "刪除").clicked() {
                    confirm = true;
                }
                if secondary(ui, "取消").clicked() {
                    close = true;
                }
            });
        });
        if confirm {
            let id = lot.id;
            let result = service::delete_lot(&shared.store, user, id);
            shared.act(result, Some("已刪除"));
            close = true;
        }
        if close {
            self.deleting = None;
        }
    }
}

const WARN_DARK_TEXT: egui::Color32 = super::theme::WARN_DARK;

fn patch_is_empty(p: &LotPatch) -> bool {
    p.receipt_date.is_none()
        && p.manufacture_date.is_none()
        && p.expiry_date.is_none()
        && p.supplier.is_none()
        && p.qty_on_hand.is_none()
        && p.verdict.is_none()
        && p.remark.is_none()
}

/// One item: name, stock, next box, flags. Returns whether it was clicked.
fn item_row(ui: &mut Ui, item: &Item, lots: &[&Lot], open: bool, user: &Session, shared: &mut Shared) -> bool {
    let drawable: Vec<&&Lot> = lots.iter().filter(|l| l.drawable()).collect();
    let next = drawable.iter().find(|l| l.is_fifo_next).or(drawable.first());
    let response = row_frame(open)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(if open { "▼" } else { "▶" }).color(MUTED));
                ui.allocate_ui(egui::vec2(230.0, 40.0), |ui| {
                    ui.set_width(230.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add(egui::Label::new(RichText::new(&item.name).font(bold_font(17.0))).truncate());
                        ui.label(small(format!(
                            "{}　{}",
                            item.model.as_deref().unwrap_or("（無型號）"),
                            item.spec.as_deref().unwrap_or("")
                        )));
                    });
                });
                ui.allocate_ui(egui::vec2(170.0, 40.0), |ui| {
                    ui.set_width(170.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(small("可領用"));
                        let mut text = RichText::new(format!("{} 箱", item.on_hand)).font(bold_font(20.0));
                        if item.on_hand == 0 {
                            text = text.color(MUTED);
                        }
                        ui.horizontal(|ui| {
                            ui.label(text);
                            if let (Some(m), true) = (item.on_hand_m(), item.on_hand > 0) {
                                ui.label(small(format!(
                                    "{}{}",
                                    urdwms_core::units::thousands(m),
                                    item.pack_unit.as_deref().unwrap_or("")
                                )));
                            }
                        });
                    });
                });
                ui.allocate_ui(egui::vec2(250.0, 40.0), |ui| {
                    ui.set_width(250.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(small("下一個該領"));
                        ui.horizontal(|ui| match next {
                            Some(l) => {
                                tag(
                                    ui,
                                    format!("製造 {}", l.manufacture_date.as_deref().unwrap_or(&l.receipt_date)),
                                    OK,
                                );
                                if l.fifo_basis == "進貨日期" {
                                    tag(ui, "依進貨日", ORANGE)
                                        .on_hover_text("這個品項沒有批次填製造日期，暫時用進貨日排序");
                                }
                                if drawable.len() > 1 {
                                    ui.label(small(format!("共 {} 批", drawable.len())));
                                }
                            }
                            None => {
                                ui.label(muted("無可領批次"));
                            }
                        });
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    if item.is_low() {
                        tag(ui, format!("低於水位 {}", item.safety_stock), ORANGE);
                    }
                    if item.rejected_qty > 0 {
                        tag(ui, format!("{} 箱不合格", item.rejected_qty), BAD);
                    }
                    if !item.recognisable() {
                        let tip = if item.matchable() {
                            "這個品項的影像辨識已關閉，領用時人工選"
                        } else {
                            "沒有型號也沒登記箱上料號，領用時人工選"
                        };
                        tag_grey(ui, "人工選").on_hover_text(tip);
                    }
                    if let Some(d) = &item.last_receipt_date {
                        ui.label(small(format!("最近進貨 {d}")));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if item.on_hand > 0 && user.can("issue.create") && link(ui, "領用 →").clicked() {
                        shared.nav = Some(Nav::IssueItem(item.id));
                    }
                });
            });
        })
        .response;
    response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn lot_table(ui: &mut Ui, lots: &[&Lot], user: &Session, editing: &mut Option<LotEdit>, deleting: &mut Option<Lot>) {
    egui::Frame::new().inner_margin(egui::Margin { left: 28, right: 8, top: 4, bottom: 8 }).show(ui, |ui| {
        if lots.is_empty() {
            ui.label(muted("尚無批次"));
            return;
        }
        let can_edit = user.can("lot.edit") || user.can("lot.delete");
        egui::ScrollArea::horizontal().id_salt(("lots", lots[0].item_id)).show(ui, |ui| {
            egui::Grid::new(("lot-grid", lots[0].item_id))
                .striped(true)
                .spacing([18.0, 8.0])
                .min_row_height(32.0)
                .show(ui, |ui| {
                    for h in ["製造日", "進貨日", "有效期限", "廠商", "判定", "狀態", "在庫", ""] {
                        ui.label(RichText::new(h).font(font(14.0)).color(MUTED));
                    }
                    ui.end_row();
                    for lot in lots {
                        let dim = lot.qty_on_hand == 0;
                        let text = |s: String| if dim { RichText::new(s).color(MUTED) } else { RichText::new(s) };
                        ui.horizontal(|ui| {
                            ui.label(text(lot.manufacture_date.clone().unwrap_or_else(|| "未填".into())));
                            if lot.is_fifo_next {
                                tag(ui, "FIFO 應領", OK);
                            }
                            if lot.fifo_also_ok {
                                tag_grey(ui, "同製造日．可領").on_hover_text("跟應領那批同一個製造日，領這批也合法");
                            }
                        });
                        ui.label(text(lot.receipt_date.clone()));
                        expiry_cell(ui, lot);
                        ui.label(text(or_dash(lot.supplier.as_deref())));
                        match lot.verdict.as_deref() {
                            Some(FAIL) => tag(ui, FAIL, BAD),
                            Some(PASS) => tag(ui, PASS, OK),
                            _ => ui.label(muted("—")),
                        };
                        match lot.state() {
                            "已領完" => tag_grey(ui, "已領完"),
                            "領貨中" => tag(ui, format!("領貨中 {}/{}", lot.qty_drawn, lot.qty_received), PRIMARY),
                            s => tag_grey(ui, s),
                        };
                        ui.horizontal(|ui| {
                            ui.label(text(format!("{} 箱", lot.qty_on_hand)));
                            if let Some(m) = lot.entered_meters {
                                ui.label(small(format!("（單上 {}）", urdwms_core::units::thousands(m))));
                            }
                        });
                        ui.horizontal(|ui| {
                            if can_edit {
                                if user.can("lot.edit") && link(ui, "編輯").clicked() {
                                    *editing = Some(LotEdit::new(lot));
                                }
                                if user.can("lot.delete") {
                                    let used = lot.draw_count > 0;
                                    let r = link_danger(ui, "刪除", !used);
                                    if used {
                                        r.on_disabled_hover_text(format!(
                                            "已有 {} 筆領用紀錄，刪掉追溯會斷。要退出流通請把數量改成 0。",
                                            lot.draw_count
                                        ));
                                    } else if r.clicked() {
                                        *deleting = Some((*lot).clone());
                                    }
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
        });
    });
}

/// The recorded expiry, coloured by how close it is. An item that requires
/// one but has none is flagged — that lot would never raise an expiry alert.
pub fn expiry_cell(ui: &mut Ui, lot: &Lot) {
    match (&lot.expiry_date, lot.days_left) {
        (Some(d), Some(left)) => {
            let color = if left <= 0 {
                BAD
            } else if left <= 90 {
                WARN
            } else {
                MUTED
            };
            let text = if left <= 0 { format!("{d}（已過期）") } else { format!("{d}（剩 {left} 天）") };
            ui.label(RichText::new(text).color(color));
        }
        _ if lot.item_has_expiry => {
            tag(ui, "缺有效期限", BAD).on_hover_text("這個品項有效期，但這批沒有填 —— 不會發效期提醒");
        }
        _ => {
            ui.label(muted("—"));
        }
    }
}
