//! Look and feel.
//!
//! Palette follows the ChimesFlow / antd colours the web version used, so
//! people who learned the old screens recognise the new ones. Type is larger
//! than desktop defaults on purpose: this runs on a tablet at arm's length
//! next to a packing line, where 14px is unreadable without leaning in.

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Visuals,
};

pub const PRIMARY: Color32 = Color32::from_rgb(0x16, 0x77, 0xff);
pub const PRIMARY_BG: Color32 = Color32::from_rgb(0xe6, 0xf4, 0xff);
pub const OK: Color32 = Color32::from_rgb(0x52, 0xc4, 0x1a);
pub const OK_DARK: Color32 = Color32::from_rgb(0x38, 0x9e, 0x0d);
pub const BAD: Color32 = Color32::from_rgb(0xff, 0x4d, 0x4f);
pub const WARN: Color32 = Color32::from_rgb(0xfa, 0xad, 0x14);
pub const WARN_DARK: Color32 = Color32::from_rgb(0xd4, 0x8a, 0x06);
pub const ORANGE: Color32 = Color32::from_rgb(0xfa, 0x8c, 0x16);
pub const TEXT: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
pub const MUTED: Color32 = Color32::from_rgb(0x8c, 0x8c, 0x8c);
pub const BORDER: Color32 = Color32::from_rgb(0xe5, 0xe7, 0xeb);
pub const SUBTLE: Color32 = Color32::from_rgb(0xf0, 0xf0, 0xf0);
pub const PAGE: Color32 = Color32::from_rgb(0xf3, 0xf4, 0xf6);
pub const CARD: Color32 = Color32::WHITE;
pub const SIDEBAR: Color32 = Color32::from_rgb(0x0b, 0x1a, 0x2e);
pub const SIDEBAR_HOVER: Color32 = Color32::from_rgb(0x16, 0x2b, 0x45);
pub const SIDEBAR_TEXT: Color32 = Color32::from_rgb(0xc8, 0xd0, 0xdb);

/// Touch target for the issuing screen (declared override: gloves, standing).
pub const TOUCH: f32 = 64.0;
/// Ordinary control height elsewhere — still finger-sized.
pub const CONTROL: f32 = 40.0;

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold_font(size: f32) -> FontId {
    FontId::new(size, bold())
}

/// Noto Sans TC (SIL Open Font License, see assets/fonts/OFL.txt), carried
/// inside the exe: a factory tablet with a trimmed or broken font set must
/// still show every character on the screen.
static REGULAR: &[u8] = include_bytes!("../../assets/fonts/NotoSansTC-Regular.otf");
static BOLD: &[u8] = include_bytes!("../../assets/fonts/NotoSansTC-Bold.otf");

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // `URDWMS_FONT` overrides the face, for a plant that wants its own.
    let regular = std::env::var("URDWMS_FONT")
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .map(FontData::from_owned)
        .unwrap_or_else(|| FontData::from_static(REGULAR));
    fonts.font_data.insert("cjk".into(), regular.into());
    fonts.font_data.insert("cjk-bold".into(), FontData::from_static(BOLD).into());
    let base = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    // Noto first for every glyph it has (Latin included, so mixed text reads
    // as one face); egui's own fonts after it for the icon glyphs.
    let mut regular_chain = vec!["cjk".to_string()];
    regular_chain.extend(base.iter().cloned());
    let mut bold_chain = vec!["cjk-bold".to_string(), "cjk".to_string()];
    bold_chain.extend(base);
    fonts.families.insert(FontFamily::Proportional, regular_chain);
    fonts.families.entry(FontFamily::Monospace).or_default().push("cjk".into());
    fonts.families.insert(bold(), bold_chain);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
    style.text_styles = [
        (TextStyle::Small, font(13.0)),
        (TextStyle::Body, font(16.0)),
        (TextStyle::Button, font(16.0)),
        (TextStyle::Monospace, FontId::monospace(14.0)),
        (TextStyle::Heading, bold_font(24.0)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size = egui::vec2(40.0, CONTROL - 8.0);
    style.spacing.combo_height = 320.0;
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.floating = false;
    style.spacing.icon_width = 20.0;
    style.spacing.icon_width_inner = 12.0;
    style.spacing.menu_margin = Margin::same(8);

    let mut v = Visuals::light();
    v.override_text_color = Some(TEXT);
    v.panel_fill = PAGE;
    v.window_fill = CARD;
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = Color32::from_rgb(0xfa, 0xfa, 0xfb);
    v.code_bg_color = SUBTLE;
    v.hyperlink_color = PRIMARY;
    v.selection.bg_fill = PRIMARY_BG;
    v.selection.stroke = Stroke::new(1.5, PRIMARY);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(40) };
    v.popup_shadow = Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(30) };
    v.window_stroke = Stroke::new(1.0, BORDER);
    let radius = CornerRadius::same(6);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.bg_fill = Color32::WHITE;
    v.widgets.inactive.weak_bg_fill = Color32::WHITE;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xd9, 0xd9, 0xd9));
    // Text colour comes from override_text_color; this stroke draws scroll
    // handles, combo arrows and radio dots, which read better in grey.
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0x9a, 0x9f, 0xa8));
    v.widgets.hovered.bg_fill = Color32::WHITE;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xf5, 0xf9, 0xff);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, PRIMARY);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, PRIMARY);
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = PRIMARY_BG;
    v.widgets.active.weak_bg_fill = PRIMARY_BG;
    v.widgets.active.bg_stroke = Stroke::new(1.0, PRIMARY);
    v.widgets.active.fg_stroke = Stroke::new(1.0, PRIMARY);
    v.widgets.active.expansion = 0.0;
    v.widgets.open.weak_bg_fill = Color32::WHITE;
    v.widgets.open.bg_stroke = Stroke::new(1.0, PRIMARY);
    style.visuals = v;
    ctx.set_style_of(egui::Theme::Light, style);
}
