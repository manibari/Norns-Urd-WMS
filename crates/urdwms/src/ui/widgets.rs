//! Small building blocks every screen uses, so the screens read as one system.

use egui::{Align, Color32, CornerRadius, Frame, Layout, Margin, Response, RichText, Sense, Shadow, Stroke, Ui, Vec2};

use super::theme::{self, BAD, BORDER, CARD, MUTED, OK, PRIMARY, PRIMARY_BG, TEXT, WARN, bold_font, font};

/// White surface with a soft shadow — the unit every screen is laid out in.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    card_frame()
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub fn card_frame() -> Frame {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(20))
        .shadow(Shadow { offset: [0, 1], blur: 4, spread: 0, color: Color32::from_black_alpha(10) })
}

/// A card with a title row and optional right-aligned extras.
pub fn titled_card<R>(
    ui: &mut Ui,
    title: impl Into<RichText>,
    extra: impl FnOnce(&mut Ui),
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(title.into().font(bold_font(18.0)));
            ui.with_layout(Layout::right_to_left(Align::Center), extra);
        });
        ui.add_space(6.0);
        add(ui)
    })
}

pub fn page_title(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    ui.label(RichText::new(title).font(bold_font(28.0)));
    if let Some(s) = subtitle {
        ui.label(RichText::new(s).color(MUTED));
    }
    ui.add_space(8.0);
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(MUTED)
}

pub fn strong(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).font(bold_font(16.0))
}

pub fn small(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).font(font(13.0)).color(MUTED)
}

/// Light pill: coloured text on a tint of the same colour.
pub fn tag(ui: &mut Ui, text: impl Into<String>, color: Color32) -> Response {
    let bg = tint(color, 0.12);
    Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, tint(color, 0.35)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| ui.label(RichText::new(text.into()).font(font(14.0)).color(darken(color))))
        .response
}

pub fn tag_grey(ui: &mut Ui, text: impl Into<String>) -> Response {
    tag(ui, text, Color32::from_rgb(0x59, 0x59, 0x59))
}

pub fn tint(color: Color32, amount: f32) -> Color32 {
    let mix = |c: u8| (255.0 - (255.0 - c as f32) * amount) as u8;
    Color32::from_rgb(mix(color.r()), mix(color.g()), mix(color.b()))
}

pub fn darken(color: Color32) -> Color32 {
    let d = |c: u8| (c as f32 * 0.78) as u8;
    Color32::from_rgb(d(color.r()), d(color.g()), d(color.b()))
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Info,
    Success,
    Warning,
    Error,
}

impl Kind {
    pub fn color(self) -> Color32 {
        match self {
            Kind::Info => PRIMARY,
            Kind::Success => OK,
            Kind::Warning => WARN,
            Kind::Error => BAD,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Kind::Info => "ℹ",
            Kind::Success => "✔",
            Kind::Warning => "⚠",
            Kind::Error => "✖",
        }
    }
}

/// Banner with a title and an explanation — for things that need reading, not a toast.
pub fn alert(ui: &mut Ui, kind: Kind, title: &str, description: &str) {
    alert_with(ui, kind, title, description, |_| {});
}

pub fn alert_with(ui: &mut Ui, kind: Kind, title: &str, description: &str, actions: impl FnOnce(&mut Ui)) {
    let c = kind.color();
    Frame::new()
        .fill(tint(c, 0.08))
        .stroke(Stroke::new(1.0, tint(c, 0.4)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(kind.icon()).font(font(18.0)).color(c));
                ui.vertical(|ui| {
                    if !title.is_empty() {
                        ui.label(RichText::new(title).font(bold_font(16.0)));
                    }
                    if !description.is_empty() {
                        ui.label(RichText::new(description).color(Color32::from_rgb(0x43, 0x43, 0x43)));
                    }
                    actions(ui);
                });
            });
        });
}

fn styled(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text.into()).font(font(size)).color(color)
}

/// Filled blue button.
pub fn primary(ui: &mut Ui, text: impl Into<String>) -> Response {
    sized(ui, text, PRIMARY, Color32::WHITE, theme::CONTROL, 16.0, true)
}

pub fn danger(ui: &mut Ui, text: impl Into<String>) -> Response {
    sized(ui, text, BAD, Color32::WHITE, theme::CONTROL, 16.0, true)
}

/// Outline button.
pub fn secondary(ui: &mut Ui, text: impl Into<String>) -> Response {
    sized(ui, text, Color32::WHITE, TEXT, theme::CONTROL, 16.0, true)
}

/// Text-only action inside tables.
pub fn link(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.add(egui::Button::new(styled(text, 15.0, PRIMARY)).frame(false))
}

pub fn link_danger(ui: &mut Ui, text: impl Into<String>, enabled: bool) -> Response {
    ui.add_enabled(enabled, egui::Button::new(styled(text, 15.0, BAD)).frame(false))
}

pub fn sized(
    ui: &mut Ui,
    text: impl Into<String>,
    fill: Color32,
    fg: Color32,
    height: f32,
    size: f32,
    enabled: bool,
) -> Response {
    let stroke =
        if fill == Color32::WHITE { Stroke::new(1.0, Color32::from_rgb(0xd9, 0xd9, 0xd9)) } else { Stroke::NONE };
    let button = egui::Button::new((egui::Atom::grow(), styled(text, size, fg), egui::Atom::grow()))
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(height * 1.5, height));
    ui.add_enabled(enabled, button)
}

/// Full-width touch button for the issuing screen.
pub fn touch(
    ui: &mut Ui,
    text: impl Into<String>,
    fill: Color32,
    fg: Color32,
    height: f32,
    size: f32,
    enabled: bool,
) -> Response {
    let stroke =
        if fill == Color32::WHITE { Stroke::new(1.5, Color32::from_rgb(0xd9, 0xd9, 0xd9)) } else { Stroke::NONE };
    let button = egui::Button::new((egui::Atom::grow(), styled(text, size, fg), egui::Atom::grow()))
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(10))
        .min_size(Vec2::new(ui.available_width(), height));
    ui.add_enabled(enabled, button)
}

/// Big number with a caption, read from a few steps away.
pub fn stat(ui: &mut Ui, title: &str, value: impl ToString, suffix: &str, color: Option<Color32>) {
    ui.vertical(|ui| {
        ui.label(RichText::new(title).font(font(15.0)).color(MUTED));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(value.to_string()).font(bold_font(38.0)).color(color.unwrap_or(TEXT)));
            ui.add_space(2.0);
            ui.label(RichText::new(suffix).font(font(16.0)).color(MUTED));
        });
    });
}

/// Equal-width columns that wrap to fewer per row on a narrow screen.
pub fn columns_responsive(ui: &mut Ui, count: usize, min_width: f32, mut add: impl FnMut(&mut Ui, usize)) {
    let gap = 16.0;
    let width = ui.available_width();
    let per_row = ((width + gap) / (min_width + gap)).floor().clamp(1.0, count as f32) as usize;
    let cell = (width - gap * (per_row as f32 - 1.0)) / per_row as f32;
    for chunk_start in (0..count).step_by(per_row) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for i in chunk_start..(chunk_start + per_row).min(count) {
                ui.allocate_ui_with_layout(Vec2::new(cell, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(cell);
                    add(ui, i);
                });
            }
        });
    }
}

/// Two (or more) mutually exclusive choices drawn as a segmented control.
/// Used instead of a checkbox wherever "not ticked" would be ambiguous.
pub fn segmented(ui: &mut Ui, value: &mut String, options: &[&str]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, option) in options.iter().enumerate() {
            let selected = value == option;
            let radius = match i {
                0 => CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 },
                n if n == options.len() - 1 => CornerRadius { nw: 0, sw: 0, ne: 6, se: 6 },
                _ => CornerRadius::ZERO,
            };
            let (fill, fg) = if selected { (PRIMARY, Color32::WHITE) } else { (Color32::WHITE, TEXT) };
            let button = egui::Button::new(styled(*option, 15.0, fg))
                .fill(fill)
                .stroke(Stroke::new(1.0, if selected { PRIMARY } else { Color32::from_rgb(0xd9, 0xd9, 0xd9) }))
                .corner_radius(radius)
                .min_size(Vec2::new(72.0, 34.0));
            if ui.add(button).clicked() && !selected {
                *value = option.to_string();
                changed = true;
            }
        }
    });
    changed
}

/// iOS-style switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = Vec2::new(40.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let bg = if *on { PRIMARY } else { Color32::from_rgb(0xbf, 0xbf, 0xbf) };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(11), bg);
    let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    painter.circle_filled(egui::pos2(x, rect.center().y), 9.0, Color32::WHITE);
    response
}

/// Single-line input with a fixed width and an optional hint.
/// Sized explicitly: inside a grid, a text box asked for its "desired" width
/// gets the column's width from the previous frame instead, and never grows.
pub fn text_input(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    let edit = egui::TextEdit::singleline(value).hint_text(muted(hint)).margin(Margin::symmetric(8, 7));
    ui.add_sized(Vec2::new(width, 34.0), edit)
}

pub fn password_input(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    let edit = egui::TextEdit::singleline(value).password(true).hint_text(muted(hint)).margin(Margin::symmetric(8, 7));
    ui.add_sized(Vec2::new(width, 34.0), edit)
}

/// Integer input that keeps the text the person typed until it parses.
pub fn number_input(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    let r = text_input(ui, value, hint, width);
    if r.changed() {
        value.retain(|c| c.is_ascii_digit() || c == '-');
    }
    r
}

/// Dropdown over a fixed list; `None` shows the placeholder.
pub fn select(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut Option<String>,
    options: &[String],
    placeholder: &str,
    width: f32,
    clearable: bool,
) -> bool {
    let mut changed = false;
    let shown = value.clone().map(RichText::new).unwrap_or_else(|| muted(placeholder));
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(width).height(360.0).show_ui(ui, |ui| {
        if clearable && ui.selectable_label(value.is_none(), muted("（不選）")).clicked() {
            *value = None;
            changed = true;
        }
        for option in options {
            if ui.selectable_label(value.as_deref() == Some(option), option).clicked() {
                *value = Some(option.clone());
                changed = true;
            }
        }
    });
    changed
}

/// Dropdown with a search box at the top — for item pickers, which get long.
pub fn search_select<T: Clone + PartialEq>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    value: &mut Option<T>,
    options: &[(T, String, bool)],
    placeholder: &str,
    width: f32,
) -> bool {
    let mut changed = false;
    let label = options.iter().find(|(v, _, _)| Some(v) == value.as_ref()).map(|(_, l, _)| l.clone());
    let shown = label.map(RichText::new).unwrap_or_else(|| muted(placeholder));
    let filter_id = ui.make_persistent_id((id, "filter"));
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(width).height(420.0).show_ui(ui, |ui| {
        let mut filter: String = ui.data_mut(|d| d.get_temp(filter_id).unwrap_or_default());
        let r = ui.add(egui::TextEdit::singleline(&mut filter).hint_text("🔍 搜尋").desired_width(f32::INFINITY));
        if !r.has_focus() && filter.is_empty() {
            r.request_focus();
        }
        ui.separator();
        let needle = filter.to_lowercase();
        for (v, l, enabled) in options {
            if !needle.is_empty() && !l.to_lowercase().contains(&needle) {
                continue;
            }
            let selected = value.as_ref() == Some(v);
            if ui.add_enabled(*enabled, egui::Button::selectable(selected, l)).clicked() {
                *value = Some(v.clone());
                changed = true;
            }
        }
        ui.data_mut(|d| d.insert_temp(filter_id, filter));
    });
    changed
}

/// Free text with suggestions: pick one of the known values or type a new one.
pub fn suggest_input(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut String,
    options: &[String],
    hint: &str,
    width: f32,
) -> Response {
    let r = text_input(ui, value, hint, width);
    let popup_id = ui.make_persistent_id((id, "suggest"));
    let needle = value.trim().to_lowercase();
    let matches: Vec<&String> = options
        .iter()
        .filter(|o| needle.is_empty() || o.to_lowercase().contains(&needle))
        .filter(|o| **o != *value)
        .take(12)
        .collect();
    if r.has_focus() && !matches.is_empty() {
        egui::Popup::from_response(&r)
            .id(popup_id)
            .open(true)
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
            .width(width)
            .show(|ui| {
                for m in matches {
                    if ui
                        .add(egui::Button::selectable(false, m.as_str()).min_size(Vec2::new(width - 16.0, 30.0)))
                        .clicked()
                    {
                        *value = m.clone();
                    }
                }
            });
    }
    r
}

/// Label above a control, with an optional hint underneath — form rows.
pub fn field<R>(ui: &mut Ui, label: &str, required: bool, hint: Option<&str>, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if required {
                ui.label(RichText::new("*").color(BAD));
            }
            ui.label(RichText::new(label).font(font(15.0)));
        });
        let out = add(ui);
        if let Some(h) = hint {
            ui.label(small(h));
        }
        out
    })
    .inner
}

/// Empty state: say what is missing rather than show a blank.
pub fn empty(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("∅").font(font(30.0)).color(Color32::from_rgb(0xd0, 0xd0, 0xd0)));
        ui.label(muted(text));
    });
    ui.add_space(12.0);
}

/// Horizontal rule inside cards.
pub fn divider(ui: &mut Ui) {
    let rect = ui.available_rect_before_wrap();
    let y = ui.cursor().top() + 4.0;
    ui.painter().hline(rect.x_range(), y, Stroke::new(1.0, theme::SUBTLE));
    ui.add_space(9.0);
}

/// Highlighted row background used for selectable list rows.
pub fn row_frame(selected: bool) -> Frame {
    Frame::new()
        .fill(if selected { PRIMARY_BG } else { Color32::WHITE })
        .stroke(Stroke::new(1.0, if selected { PRIMARY } else { BORDER }))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(14, 10))
}

/// Short, human time: `2026-10-25 19:05`.
pub fn short_time(iso: &str) -> String {
    iso.get(..16).unwrap_or(iso).replace('T', " ")
}

pub fn or_dash(value: Option<&str>) -> String {
    value.filter(|v| !v.is_empty()).unwrap_or("—").to_string()
}

fn parse_jiff(value: &str) -> Option<jiff::civil::Date> {
    value.parse::<jiff::civil::Date>().ok()
}

/// Date picker over an ISO string. `None` shows an "add" button, so a blank
/// date is visibly blank rather than defaulting to today by accident.
pub fn date_field(ui: &mut Ui, id: &str, value: &mut Option<String>, clearable: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| match value.as_deref().and_then(parse_jiff) {
        Some(mut date) => {
            let before = date;
            ui.add(egui_extras::DatePickerButton::new(&mut date).id_salt(id).format("%Y-%m-%d").calendar_week(false));
            if date != before {
                *value = Some(date.to_string());
                changed = true;
            }
            if clearable && ui.add(egui::Button::new(muted("× 清除")).frame(false)).clicked() {
                *value = None;
                changed = true;
            }
        }
        None => {
            if secondary(ui, "＋ 選日期").clicked() {
                *value = Some(crate::db::today_iso());
                changed = true;
            }
            if let Some(raw) = value.as_deref() {
                ui.colored_label(BAD, format!("無法解讀：{raw}"));
            }
        }
    });
    changed
}
