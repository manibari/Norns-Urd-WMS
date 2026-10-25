//! 庫存總覽的三張圖, hand-drawn. With a handful of items a chart library buys
//! nothing, and drawing them ourselves keeps the type large enough to read
//! from across the packing line.
//!
//! Each answers one question:
//!   1. 夠不夠用 — on hand against the safety level
//!   2. 老不老   — age of stock by 製造日 (the same ruler FIFO uses)
//!   3. 來不來得及 — days to the recorded expiry

use egui::{Color32, CornerRadius, RichText, Sense, Stroke, Ui, Vec2};

use super::theme::{BAD, MUTED, OK, PRIMARY, SUBTLE, WARN, bold_font, font};
use super::widgets::{empty, muted};
use crate::db::today;
use crate::service::{Item, Lot};

const LABEL_W: f32 = 150.0;

#[allow(clippy::too_many_arguments)]
fn bar_row(
    ui: &mut Ui,
    name: &str,
    sub: Option<&str>,
    fraction: f32,
    color: Color32,
    marker: Option<f32>,
    value: &str,
    tip: &str,
) {
    ui.horizontal(|ui| {
        ui.allocate_ui(Vec2::new(LABEL_W, 36.0), |ui| {
            ui.set_width(LABEL_W);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add(egui::Label::new(RichText::new(name).font(bold_font(15.0))).truncate());
                if let Some(s) = sub {
                    ui.label(RichText::new(s).font(font(12.0)).color(MUTED));
                }
            });
        });
        let width = (ui.available_width() - 70.0).max(40.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(4), SUBTLE);
        if fraction > 0.0 {
            let mut filled = rect;
            filled.set_width((rect.width() * fraction).max(4.0));
            painter.rect_filled(filled, CornerRadius::same(4), color);
        }
        if let Some(m) = marker {
            let x = rect.left() + rect.width() * m.clamp(0.0, 1.0);
            painter.line_segment(
                [egui::pos2(x, rect.top() - 4.0), egui::pos2(x, rect.bottom() + 4.0)],
                Stroke::new(2.5, Color32::from_rgb(0x26, 0x26, 0x26)),
            );
        }
        response.on_hover_text(tip);
        ui.label(RichText::new(value).font(bold_font(15.0)));
    });
}

/// The bar is stock; the black line is the safety level — line to the right
/// of the bar means short.
pub fn stock_vs_safety(ui: &mut Ui, items: &[Item]) {
    let shown: Vec<&Item> = items.iter().filter(|i| i.on_hand > 0 || i.safety_stock > 0).collect();
    if shown.is_empty() {
        return empty(ui, "尚無庫存");
    }
    let max = shown.iter().map(|i| i.on_hand.max(i.safety_stock)).max().unwrap_or(1).max(1) as f32;
    for item in shown {
        let low = item.is_low();
        let tight = !low && item.safety_stock > 0 && (item.on_hand as f32) < item.safety_stock as f32 * 1.5;
        let color = if low {
            BAD
        } else if tight {
            WARN
        } else {
            OK
        };
        let marker = (item.safety_stock > 0).then(|| item.safety_stock as f32 / max);
        bar_row(
            ui,
            &item.name,
            item.model.as_deref(),
            item.on_hand as f32 / max,
            color,
            marker,
            &format!("{} 箱", item.on_hand),
            &format!("在庫 {} 箱．安全水位 {} 箱", item.on_hand, item.safety_stock),
        );
    }
    ui.label(muted("長條＝可領用箱數，黑線＝安全水位。紅色＝低於水位，黃色＝不到水位的 1.5 倍。"));
}

/// Boxes on hand bucketed by age of 製造日. Undated lots are their own bucket:
/// unknown age is not young.
pub fn age_distribution(ui: &mut Ui, lots: &[Lot]) {
    const BUCKETS: [(&str, i64, Color32); 4] =
        [("半年內", 182, OK), ("半年～一年", 365, PRIMARY), ("一年～一年半", 547, WARN), ("超過一年半", i64::MAX, BAD)];
    let mut counts = [0i64; 5];
    let today = today();
    for lot in lots.iter().filter(|l| l.drawable()) {
        match lot.manufacture_date.as_deref().and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
            Some(d) => {
                let age = (today - d).num_days();
                let i = BUCKETS.iter().position(|(_, limit, _)| age < *limit).unwrap_or(3);
                counts[i] += lot.qty_on_hand;
            }
            None => counts[4] += lot.qty_on_hand,
        }
    }
    let total: i64 = counts.iter().sum();
    if total == 0 {
        return empty(ui, "尚無在庫批次");
    }
    let max = *counts.iter().max().unwrap() as f32;
    for (i, (label, _, color)) in BUCKETS.iter().enumerate() {
        bar_row(
            ui,
            label,
            None,
            counts[i] as f32 / max,
            *color,
            None,
            &format!("{} 箱", counts[i]),
            "依製造日計算庫齡",
        );
    }
    if counts[4] > 0 {
        bar_row(
            ui,
            "未填製造日",
            None,
            counts[4] as f32 / max,
            Color32::from_rgb(0xbf, 0xbf, 0xbf),
            None,
            &format!("{} 箱", counts[4]),
            "沒填製造日的批次 FIFO 會排最後",
        );
    }
    ui.label(muted("庫齡看製造日 —— 跟先進先出用的是同一把尺。"));
}

/// Lots with a recorded expiry, soonest first.
pub fn expiry_countdown(ui: &mut Ui, lots: &[Lot], window_days: i64) {
    let mut dated: Vec<&Lot> = lots.iter().filter(|l| l.drawable() && l.days_left.is_some()).collect();
    dated.sort_by_key(|l| l.days_left);
    if dated.is_empty() {
        return empty(ui, "在庫批次都沒有標示有效期限");
    }
    let horizon = (window_days * 4).max(365) as f32;
    for lot in dated.iter().take(8) {
        let days = lot.days_left.unwrap_or(0);
        let color = if days <= 0 {
            BAD
        } else if days <= window_days {
            WARN
        } else {
            OK
        };
        let text = if days <= 0 { format!("已過期 {} 天", -days) } else { format!("剩 {days} 天") };
        bar_row(
            ui,
            lot.item_label(),
            lot.expiry_date.as_deref(),
            (days.max(0) as f32 / horizon).min(1.0),
            color,
            Some(window_days as f32 / horizon),
            &text,
            &format!("有效期限 {}．在庫 {} 箱", lot.expiry_date.as_deref().unwrap_or(""), lot.qty_on_hand),
        );
    }
    let painter_note = format!("黑線＝提醒門檻 {window_days} 天。只看收貨時填的有效日期，不做推算。");
    ui.label(muted(painter_note));
}
