//! The desktop shell: sign-in, navigation, notifications, background work.

mod alerts;
mod basics;
mod charts;
mod images;
mod issue;
mod logs;
mod receiving;
mod records;
mod settings;
mod stock;
pub mod theme;
pub mod widgets;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

use egui::{Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2};

use crate::auth::Session;
use crate::service::{self, Store};
use theme::{PAGE, SIDEBAR, SIDEBAR_HOVER, SIDEBAR_TEXT, bold_font, font};
use widgets::{Kind, card_frame, muted, password_input, primary, secondary, text_input};

pub use images::ImageCache;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Stock,
    Basics,
    Receiving,
    Issue,
    Records,
    Alerts,
    Logs,
    Settings,
}

impl Page {
    pub const ALL: [Page; 8] = [
        Page::Stock,
        Page::Basics,
        Page::Receiving,
        Page::Issue,
        Page::Records,
        Page::Alerts,
        Page::Logs,
        Page::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::Stock => "庫存總覽",
            Page::Basics => "1. 基本資料",
            Page::Receiving => "2. 收貨建批",
            Page::Issue => "3. 領用登錄",
            Page::Records => "4. 紀錄與追溯",
            Page::Alerts => "提醒",
            Page::Logs => "系統日誌",
            Page::Settings => "系統設定",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Page::Stock => "📊",
            Page::Basics => "📁",
            Page::Receiving => "📦",
            Page::Issue => "📷",
            Page::Records => "📋",
            Page::Alerts => "🔔",
            Page::Logs => "📜",
            Page::Settings => "⚙",
        }
    }

    /// What a role must hold to use the screen. Hiding the entry is a
    /// convenience; the service layer enforces the same permission on every
    /// write, because a hidden screen is not a locked one.
    pub fn needs(self) -> Option<&'static str> {
        match self {
            Page::Basics => Some("item.manage"),
            Page::Receiving => Some("lot.create"),
            Page::Issue => Some("issue.create"),
            Page::Logs => Some("audit.read"),
            Page::Settings => Some("dictionary.manage"),
            _ => None,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Page::Stock => "stock",
            Page::Basics => "basics",
            Page::Receiving => "receiving",
            Page::Issue => "issue",
            Page::Records => "records",
            Page::Alerts => "alerts",
            Page::Logs => "logs",
            Page::Settings => "settings",
        }
    }
}

/// A request from one screen to open another.
#[derive(Debug, Clone, PartialEq)]
pub enum Nav {
    Go(Page),
    /// 庫存總覽's 領用 link names the item — a deliberate manual entry.
    IssueItem(i64),
}

pub struct Toast {
    kind: Kind,
    text: String,
    until: Instant,
}

/// What every screen gets: the store, notifications, and a way to ask for
/// another screen. `generation` bumps after every write, so screens that
/// cache rows know to reload them.
pub struct Shared {
    pub store: Store,
    pub data_dir: PathBuf,
    pub toasts: Vec<Toast>,
    pub nav: Option<Nav>,
    pub images: ImageCache,
    pub generation: u64,
}

impl Shared {
    pub fn toast(&mut self, kind: Kind, text: impl Into<String>) {
        let secs = match kind {
            Kind::Success | Kind::Info => 4,
            Kind::Warning => 8,
            Kind::Error => 10,
        };
        self.toasts.push(Toast { kind, text: text.into(), until: Instant::now() + Duration::from_secs(secs) });
    }

    pub fn ok(&mut self, text: impl Into<String>) {
        self.toast(Kind::Success, text);
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.toast(Kind::Warning, text);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.toast(Kind::Error, text);
    }

    /// Run a write; on success bump the generation and toast, on failure toast
    /// the reason. Returns the value when it worked.
    pub fn act<T>(&mut self, result: crate::error::Res<T>, success: Option<&str>) -> Option<T> {
        match result {
            Ok(v) => {
                self.generation += 1;
                if let Some(s) = success {
                    self.ok(s);
                }
                Some(v)
            }
            Err(e) => {
                self.error(e.0);
                None
            }
        }
    }
}

/// Work that takes seconds (recognition, a camera on the network) runs off
/// the UI thread; the screen polls for the answer each frame.
pub struct Task<T> {
    rx: Option<Receiver<T>>,
    pub started: Option<Instant>,
}

impl<T> Default for Task<T> {
    fn default() -> Self {
        Task { rx: None, started: None }
    }
}

impl<T: Send + 'static> Task<T> {
    pub fn spawn(ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
        Task { rx: Some(rx), started: Some(Instant::now()) }
    }

    pub fn running(&self) -> bool {
        self.rx.is_some()
    }

    pub fn poll(&mut self) -> Option<T> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(v) => {
                self.rx = None;
                Some(v)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                None
            }
        }
    }

    pub fn cancel(&mut self) {
        self.rx = None;
    }
}

#[derive(Default)]
struct LoginForm {
    username: String,
    password: String,
    error: Option<String>,
}

#[derive(Default)]
struct PasswordForm {
    current: String,
    new: String,
    confirm: String,
    error: Option<String>,
}

/// Screenshot run for documentation and CI: visit each page, capture, exit.
#[derive(Clone, Copy, PartialEq)]
pub enum Shot {
    Page(Page),
    /// The issuing verdict for a demo photo: (manufacture date read, slug).
    Verdict(&'static str, &'static str),
}

pub struct ScreenshotPlan {
    pub dir: PathBuf,
    pub pages: Vec<Shot>,
    index: usize,
    wait: u32,
    pending: Option<Receiver<std::sync::Arc<egui::ColorImage>>>,
}

impl ScreenshotPlan {
    pub fn new(dir: PathBuf) -> Self {
        let mut pages: Vec<Shot> = Page::ALL.iter().map(|p| Shot::Page(*p)).collect();
        pages.push(Shot::Verdict("2026-06-02", "issue-wrong-box"));
        pages.push(Shot::Verdict("2025年09月26日", "issue-ok"));
        ScreenshotPlan { dir, pages, index: 0, wait: 0, pending: None }
    }
}

pub struct App {
    shared: Shared,
    session: Option<Session>,
    page: Page,
    login: LoginForm,
    password: Option<PasswordForm>,
    first_run_password: Option<&'static str>,
    stock: stock::StockPage,
    basics: basics::BasicsPage,
    receiving: receiving::ReceivingPage,
    issue: issue::IssuePage,
    records: records::RecordsPage,
    alerts: alerts::AlertsPage,
    logs: logs::LogsPage,
    settings: settings::SettingsPage,
    screenshots: Option<ScreenshotPlan>,
    alert_count: (u64, usize),
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        store: Store,
        data_dir: PathBuf,
        first_run_password: Option<&'static str>,
        auto_login: Option<Session>,
        screenshots: Option<ScreenshotPlan>,
    ) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);
        let mut app = App {
            shared: Shared { store, data_dir, toasts: vec![], nav: None, images: ImageCache::default(), generation: 0 },
            session: None,
            page: Page::Stock,
            login: LoginForm::default(),
            password: None,
            first_run_password,
            stock: Default::default(),
            basics: Default::default(),
            receiving: Default::default(),
            issue: Default::default(),
            records: Default::default(),
            alerts: Default::default(),
            logs: Default::default(),
            settings: Default::default(),
            screenshots,
            alert_count: (u64::MAX, 0),
        };
        if let Some(s) = auto_login {
            app.signed_in(s);
        }
        app
    }

    fn signed_in(&mut self, session: Session) {
        if session.must_change {
            self.password = Some(PasswordForm::default());
        }
        // Land on the first screen this role can use, with the issuing screen
        // first for operators — it is the only thing they came to do.
        self.page = if session.can("lot.create") { Page::Stock } else { Page::Issue };
        self.session = Some(session);
        self.shared.generation += 1;
    }

    fn sign_out(&mut self) {
        if let Some(s) = self.session.take() {
            service::logout(&self.shared.store, &s);
        }
        self.issue = Default::default();
        self.receiving = Default::default();
        self.password = None;
        self.login = LoginForm::default();
    }

    fn navigate(&mut self, page: Page) {
        // Pick up a role change or deactivation made by an admin.
        if let Some(s) = &self.session {
            match service::refresh_session(&self.shared.store, s) {
                Some(fresh) => self.session = Some(fresh),
                None => {
                    self.shared.error("此帳號已停用或不存在，請重新登入");
                    self.sign_out();
                    return;
                }
            }
        }
        self.page = page;
        self.shared.generation += 1;
    }

    fn login_view(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default().frame(Frame::new().fill(PAGE)).show(ui, |ui| {
            ui.add_space((ui.available_height() * 0.18).max(20.0));
            ui.vertical_centered(|ui| {
                ui.set_max_width(420.0);
                card_frame().inner_margin(Margin::same(32)).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Urd-WMS").font(bold_font(30.0)));
                        ui.label(muted("包材批次管理 —— 拍箱子取代抄表單"));
                    });
                    ui.add_space(20.0);
                    let width = ui.available_width();
                    ui.label("帳號");
                    let user = text_input(ui, &mut self.login.username, "例 kuo", width);
                    ui.add_space(4.0);
                    ui.label("密碼");
                    let pass = password_input(ui, &mut self.login.password, "", width);
                    if self.login.username.is_empty() && !user.has_focus() && !pass.has_focus() {
                        user.request_focus();
                    }
                    ui.add_space(12.0);
                    let enter = pass.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let clicked = ui
                        .add(
                            egui::Button::new(RichText::new("登入").font(font(17.0)).color(Color32::WHITE))
                                .fill(theme::PRIMARY)
                                .corner_radius(CornerRadius::same(8))
                                .min_size(Vec2::new(width, 46.0)),
                        )
                        .clicked();
                    if clicked || enter {
                        match service::login(&self.shared.store, &self.login.username, &self.login.password) {
                            Ok(session) => {
                                self.login = LoginForm::default();
                                self.signed_in(session);
                            }
                            Err(e) => {
                                self.login.error = Some(e.0);
                                self.login.password.clear();
                            }
                        }
                    }
                    if let Some(e) = &self.login.error {
                        ui.add_space(8.0);
                        widgets::alert(ui, Kind::Error, e, "");
                    }
                    if let Some(pw) = self.first_run_password {
                        ui.add_space(12.0);
                        widgets::alert(
                            ui,
                            Kind::Info,
                            "第一次使用",
                            &format!("帳號 admin，密碼 {pw}。登入後會要求改成只有你知道的密碼，再到「系統設定 → 人員與權限」建立其他人的帳號。"),
                        );
                    }
                });
            });
        });
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let Some(session) = self.session.clone() else { return };
        egui::Panel::left("nav")
            .exact_size(220.0)
            .resizable(false)
            .frame(Frame::new().fill(SIDEBAR).inner_margin(Margin { left: 12, right: 12, top: 18, bottom: 12 }))
            .show(ui, |ui| {
                ui.label(RichText::new("Urd-WMS").font(bold_font(22.0)).color(Color32::WHITE));
                ui.label(RichText::new("包材批次管理").font(font(13.0)).color(Color32::from_rgb(0x8c, 0x99, 0xa8)));
                ui.add_space(18.0);
                if self.alert_count.0 != self.shared.generation {
                    let n = service::alerts(&self.shared.store.conn).map(|a| a.total()).unwrap_or(0);
                    self.alert_count = (self.shared.generation, n);
                }
                let alerts = self.alert_count.1;
                for page in Page::ALL {
                    if page.needs().is_some_and(|p| !session.can(p)) {
                        continue;
                    }
                    let selected = self.page == page;
                    let badge = (page == Page::Alerts && alerts > 0).then_some(alerts);
                    if nav_item(ui, page, selected, badge).clicked() && !selected {
                        self.navigate(page);
                    }
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new("登出").color(SIDEBAR_TEXT)).frame(false)).clicked()
                        {
                            self.sign_out();
                        }
                        if ui.add(egui::Button::new(RichText::new("改密碼").color(SIDEBAR_TEXT)).frame(false)).clicked()
                        {
                            self.password = Some(PasswordForm::default());
                        }
                    });
                    let title = session.title.clone().unwrap_or_default();
                    ui.label(
                        RichText::new(format!("{} · {}", title, session.role_label))
                            .font(font(13.0))
                            .color(Color32::from_rgb(0x8c, 0x99, 0xa8)),
                    );
                    ui.label(RichText::new(&session.name).font(bold_font(17.0)).color(Color32::WHITE));
                    ui.add_space(6.0);
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().hline(
                        rect.x_range(),
                        ui.cursor().bottom(),
                        Stroke::new(1.0, Color32::from_white_alpha(25)),
                    );
                });
            });
    }

    fn password_modal(&mut self, ui: &mut Ui) {
        let Some(session) = self.session.clone() else { return };
        let Some(form) = &mut self.password else { return };
        let forced = session.must_change;
        let mut close = false;
        let mut done = false;
        egui::Modal::new(egui::Id::unique("password")).show(ui.ctx(), |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("更改密碼").font(bold_font(20.0)));
            if forced {
                widgets::alert(
                    ui,
                    Kind::Warning,
                    "這是別人幫你設的密碼",
                    "請改成只有你知道的 —— 收貨單上的簽名代表「這個人」，前提是只有他知道密碼。",
                );
            }
            ui.add_space(6.0);
            ui.label("目前密碼");
            password_input(ui, &mut form.current, "", 360.0);
            ui.label("新密碼（至少 8 個字元）");
            password_input(ui, &mut form.new, "", 360.0);
            ui.label("再輸入一次");
            password_input(ui, &mut form.confirm, "", 360.0);
            if let Some(e) = &form.error {
                widgets::alert(ui, Kind::Error, e, "");
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary(ui, "更改").clicked() {
                    if form.new != form.confirm {
                        form.error = Some("兩次輸入的新密碼不一樣".into());
                    } else {
                        match service::change_password(&self.shared.store, &session, &form.current, &form.new) {
                            Ok(()) => done = true,
                            Err(e) => form.error = Some(e.0),
                        }
                    }
                }
                if forced {
                    if secondary(ui, "登出").clicked() {
                        close = true;
                    }
                } else if secondary(ui, "取消").clicked() {
                    close = true;
                }
            });
        });
        if done {
            self.password = None;
            self.shared.ok("密碼已更改");
            if let Some(fresh) = service::refresh_session(&self.shared.store, &session) {
                self.session = Some(fresh);
            }
        } else if close {
            self.password = None;
            if forced {
                self.sign_out();
            }
        }
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.shared.toasts.retain(|t| t.until > now);
        if self.shared.toasts.is_empty() {
            return;
        }
        let soonest = self.shared.toasts.iter().map(|t| t.until).min().unwrap();
        ctx.request_repaint_after(soonest.saturating_duration_since(now));
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 16.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                let mut dismiss = None;
                for (i, t) in self.shared.toasts.iter().enumerate() {
                    let c = t.kind.color();
                    let r = Frame::new()
                        .fill(Color32::WHITE)
                        .stroke(Stroke::new(1.0, widgets::tint(c, 0.5)))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(18, 12))
                        .shadow(egui::Shadow {
                            offset: [0, 6],
                            blur: 20,
                            spread: 0,
                            color: Color32::from_black_alpha(35),
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(560.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(match t.kind {
                                        Kind::Success => "✔",
                                        Kind::Error => "✖",
                                        Kind::Warning => "⚠",
                                        Kind::Info => "ℹ",
                                    })
                                    .color(c)
                                    .font(font(18.0)),
                                );
                                ui.label(RichText::new(&t.text).font(font(16.0)));
                            });
                        })
                        .response
                        .interact(Sense::click());
                    if r.clicked() {
                        dismiss = Some(i);
                    }
                }
                if let Some(i) = dismiss {
                    self.shared.toasts.remove(i);
                }
            });
    }

    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some(plan) = &mut self.screenshots else { return };
        if plan.index >= plan.pages.len() {
            return;
        }
        if let Some(rx) = &plan.pending {
            if let Ok(image) = rx.try_recv() {
                let slug = match plan.pages[plan.index] {
                    Shot::Page(p) => p.slug(),
                    Shot::Verdict(_, slug) => slug,
                };
                let path = plan.dir.join(format!("{:02}-{slug}.png", plan.index + 1));
                let [w, h] = image.size;
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                if let Err(e) = image::save_buffer(&path, &bytes, w as u32, h as u32, image::ColorType::Rgba8) {
                    eprintln!("screenshot {path:?}: {e}");
                }
                plan.pending = None;
                plan.index += 1;
                plan.wait = 0;
                if plan.index >= plan.pages.len() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
            ctx.request_repaint();
            return;
        }
        let shot = plan.pages[plan.index];
        if plan.wait == 0 {
            match shot {
                Shot::Page(page) => self.page = page,
                Shot::Verdict(made, _) => {
                    self.page = Page::Issue;
                    self.issue.demo(ctx, &mut self.shared, made);
                }
            }
            self.shared.generation += 1;
        }
        plan.wait += 1;
        // A few frames for layout, fonts and textures to settle.
        if plan.wait > 12 {
            let (tx, rx) = channel();
            ctx.request_screenshot(move |image| {
                let _ = tx.send(image);
            });
            plan.pending = Some(rx);
        }
        ctx.request_repaint();
    }
}

fn nav_item(ui: &mut Ui, page: Page, selected: bool, badge: Option<usize>) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(8), theme::PRIMARY);
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(8), SIDEBAR_HOVER);
    }
    let color = if selected { Color32::WHITE } else { SIDEBAR_TEXT };
    painter.text(rect.left_center() + Vec2::new(14.0, 0.0), Align2::LEFT_CENTER, page.icon(), font(17.0), color);
    painter.text(rect.left_center() + Vec2::new(44.0, 0.0), Align2::LEFT_CENTER, page.label(), font(16.0), color);
    if let Some(n) = badge {
        let center = rect.right_center() - Vec2::new(22.0, 0.0);
        painter.circle_filled(center, 11.0, theme::BAD);
        painter.text(center, Align2::CENTER_CENTER, n.to_string(), font(12.0), Color32::WHITE);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.screenshot_step(&ctx);
        let Some(session) = self.session.clone() else {
            self.login_view(ui);
            self.toasts(&ctx);
            return;
        };

        if let Some(nav) = self.shared.nav.take() {
            match nav {
                Nav::Go(page) => self.navigate(page),
                Nav::IssueItem(id) => {
                    self.navigate(Page::Issue);
                    self.issue.start_manual(&self.shared, id);
                }
            }
        }

        self.sidebar(ui);
        let Some(session_now) = self.session.clone() else { return };
        let _ = session;
        egui::CentralPanel::default().frame(Frame::new().fill(PAGE).inner_margin(Margin::symmetric(28, 22))).show(
            ui,
            |ui| {
                if self.page.needs().is_some_and(|p| !session_now.can(p)) {
                    widgets::alert(
                        ui,
                        Kind::Warning,
                        "這個畫面不開放給你的身分",
                        &format!("{}（{}）沒有這個畫面的權限。", session_now.name, session_now.role_label),
                    );
                    return;
                }
                let shared = &mut self.shared;
                match self.page {
                    Page::Stock => self.stock.show(ui, shared, &session_now),
                    Page::Basics => self.basics.show(ui, shared, &session_now),
                    Page::Receiving => self.receiving.show(ui, shared, &session_now),
                    Page::Issue => self.issue.show(ui, shared, &session_now),
                    Page::Records => self.records.show(ui, shared, &session_now),
                    Page::Alerts => self.alerts.show(ui, shared, &session_now),
                    Page::Logs => self.logs.show(ui, shared, &session_now),
                    Page::Settings => self.settings.show(ui, shared, &session_now),
                }
            },
        );
        self.password_modal(ui);
        self.toasts(&ctx);
    }
}
