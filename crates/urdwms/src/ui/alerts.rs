//! Alerts.
//!
//! The panel always shows when it was checked, even when nothing is wrong. A
//! blank screen cannot distinguish "nothing is wrong" from "the check stopped
//! running" (M7 architecture, constraint 2) — and a monitoring surface that
//! can be silently dead is worse than none, because people trust it.

use egui::{Grid, RichText, ScrollArea, Ui};

use super::Shared;
use super::theme::{BAD, MUTED, OK, ORANGE, PRIMARY, WARN_DARK, font};
use super::widgets::*;
use crate::auth::Session;
use crate::service::{self, Alerts};

#[derive(Default)]
pub struct AlertsPage {
    loaded: Option<u64>,
    data: Option<Alerts>,
}

fn table(ui: &mut Ui, id: &str, headers: &[&str], rows: Vec<Vec<RichText>>, empty_text: &str) {
    if rows.is_empty() {
        return empty(ui, empty_text);
    }
    egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
        Grid::new(id).striped(true).spacing([28.0, 10.0]).min_row_height(30.0).show(ui, |ui| {
            for h in headers {
                ui.label(RichText::new(*h).font(font(14.0)).color(MUTED));
            }
            ui.end_row();
            for row in rows {
                for cell in row {
                    ui.label(cell);
                }
                ui.end_row();
            }
        });
    });
}

fn t(s: impl Into<String>) -> RichText {
    RichText::new(s.into())
}

impl AlertsPage {
    pub fn show(&mut self, ui: &mut Ui, shared: &mut Shared, _user: &Session) {
        if self.loaded != Some(shared.generation) {
            match service::alerts(&shared.store.conn) {
                Ok(a) => self.data = Some(a),
                Err(e) => shared.error(format!("提醒載入失敗：{e}")),
            }
            self.loaded = Some(shared.generation);
        }
        let Some(a) = self.data.clone() else { return };
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                page_title(ui, "提醒", None);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary(ui, "↻ 重新檢查").clicked() {
                        self.loaded = None;
                    }
                });
            });
            card(ui, |ui| {
                columns_responsive(ui, 5, 150.0, |ui, i| {
                    let warn = |n: usize| (n > 0).then_some(WARN_DARK);
                    match i {
                        0 => stat(ui, "效期將屆", a.expiring.len(), "批", warn(a.expiring.len())),
                        1 => stat(ui, "呆滯批次", a.stale.len(), "批", warn(a.stale.len())),
                        2 => stat(ui, "低於安全水位", a.low_stock.len(), "項", warn(a.low_stock.len())),
                        3 => stat(ui, "產品名稱待補", a.pending_detail.len(), "筆", warn(a.pending_detail.len())),
                        _ => stat(ui, "驗收不合格待處理", a.rejected.len(), "批", (!a.rejected.is_empty()).then_some(BAD)),
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("✔").color(OK));
                    ui.label(muted(format!(
                        "最後檢查 {}｜門檻：效期 {} 天內、呆滯 {} 天、明細待補 {} 小時（系統設定可調）｜有效期限就是收貨時填的那個日期，不做推算",
                        short_time(&a.checked_at),
                        a.thresholds.expiry_days,
                        a.thresholds.stale_days,
                        a.thresholds.pending_hours
                    )));
                });
            });
            ui.add_space(12.0);

            titled_card(ui, "驗收不合格待處理", |_| {}, |ui| {
                let rows = a
                    .rejected
                    .iter()
                    .map(|l| {
                        vec![
                            t(or_dash(l.model.as_deref())),
                            t(&l.name),
                            t(or_dash(l.supplier.as_deref())),
                            t(&l.receipt_date),
                            t(format!("{} 箱", l.qty_on_hand)),
                            t(or_dash(l.remark.as_deref())),
                            t(format!("{}／{}", or_dash(l.recorded_by.as_deref()), or_dash(l.confirmed_by.as_deref()))),
                        ]
                    })
                    .collect();
                table(ui, "rejected", &["型號", "原物料名稱", "廠商", "進貨日", "數量", "原因", "記錄／確認"], rows, "沒有不合格批次");
                ui.label(small("這些批次還在架上但不計入在庫、FIFO 不會指到、也領不出來。退貨或報廢後請到庫存總覽把數量改成 0，紀錄仍會留著。"));
            });
            ui.add_space(12.0);

            titled_card(ui, "效期將屆", |_| {}, |ui| {
                let rows = a
                    .expiring
                    .iter()
                    .map(|l| {
                        let d = l.days_left.unwrap_or(0);
                        let c = if d <= 0 { BAD } else if d <= 30 { ORANGE } else { PRIMARY };
                        vec![
                            t(&l.name),
                            t(&l.receipt_date),
                            t(or_dash(l.manufacture_date.as_deref())),
                            t(or_dash(l.expiry_date.as_deref())),
                            RichText::new(if d <= 0 { format!("已過期 {} 天", -d) } else { format!("剩 {d} 天") }).color(c),
                            t(format!("{} 箱", l.qty_on_hand)),
                        ]
                    })
                    .collect();
                table(ui, "expiring", &["原物料名稱", "進貨日", "製造日", "有效期限", "剩餘", "在庫"], rows, "尚無將到期批次");
            });
            ui.add_space(12.0);

            titled_card(ui, "呆滯批次", |_| {}, |ui| {
                let rows = a
                    .stale
                    .iter()
                    .map(|l| vec![t(&l.name), t(or_dash(l.model.as_deref())), t(&l.receipt_date), t(format!("{} 天", l.age_days)), t(format!("{} 箱", l.qty_on_hand))])
                    .collect();
                table(ui, "stale", &["原物料名稱", "型號", "進貨日", "停留", "在庫"], rows, "尚無呆滯批次");
            });
            ui.add_space(12.0);

            titled_card(ui, "低於安全水位", |_| {}, |ui| {
                let rows = a
                    .low_stock
                    .iter()
                    .map(|l| vec![t(or_dash(l.model.as_deref())), t(&l.name), RichText::new(format!("{} 箱", l.on_hand)).color(BAD), t(format!("{} 箱", l.safety_stock))])
                    .collect();
                table(ui, "low", &["型號", "原物料名稱", "在庫", "安全水位"], rows, "庫存都在水位之上");
            });
            ui.add_space(12.0);

            titled_card(ui, "產品名稱待補", |_| {}, |ui| {
                let rows = a.pending_detail.iter().map(|p| vec![t(format!("#{}", p.scan_id)), t(or_dash(p.label.as_deref())), t(short_time(&p.captured_at))]).collect();
                table(ui, "pending", &["紀錄", "品項", "領用時間"], rows, "沒有待補明細");
                ui.label(small("到「紀錄與追溯」按「補明細」補上產品名稱。"));
            });
        });
    }
}
