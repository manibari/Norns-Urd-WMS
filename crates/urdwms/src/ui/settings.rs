//! 系統設定 — the knobs that apply to the whole plant: which model reads the
//! labels, where images come from, when an alert fires, who may do what.
//!
//! The camera test button matters more than it looks: a camera that is wired
//! but answering with a login page or a stream fails in a way nobody can
//! diagnose from the issuing screen at 7am, so it gets diagnosed here — with
//! the frame it returned on screen.

use std::time::Instant;

use egui::{RichText, ScrollArea, Ui, Vec2};
use urdwms_core::camera::{self, CameraConfig, CameraError};

use super::theme::{MUTED, PRIMARY, bold_font, font};
use super::widgets::*;
use super::{Shared, Task};
use crate::auth::{ROLES, Session};
use crate::service::{self, AlertThresholds, NewUser, RECOGNITION_MODELS, UserPatch, UserRow};

struct CameraForm {
    config: CameraConfig,
    port: String,
    timeout: String,
    password: String,
    has_password: bool,
}

impl CameraForm {
    fn from(config: CameraConfig) -> Self {
        CameraForm {
            port: config.port.to_string(),
            timeout: format!("{}", config.timeout),
            has_password: !config.password.is_empty(),
            password: String::new(),
            config: CameraConfig { password: String::new(), ..config },
        }
    }

    fn to_config(&self) -> Result<CameraConfig, String> {
        let port: u16 = self.port.trim().parse().map_err(|_| "連接埠必須在 1–65535".to_string())?;
        let timeout: f64 = self.timeout.trim().parse().map_err(|_| "逾時請填秒數".to_string())?;
        Ok(CameraConfig {
            port,
            timeout: timeout.clamp(1.0, 60.0),
            password: self.password.clone(),
            ..self.config.clone()
        })
    }
}

struct TestResult {
    ok: bool,
    text: String,
    image: Option<egui::ColorImage>,
}

struct UserForm {
    id: Option<i64>,
    username: String,
    name: String,
    title: String,
    role: String,
    password: String,
    active: bool,
}

#[derive(Default)]
pub struct SettingsPage {
    loaded: Option<u64>,
    tab: usize,
    model: String,
    api_key: String,
    api_key_set: bool,
    camera: Option<CameraForm>,
    test: Task<TestResult>,
    result: Option<(bool, String)>,
    preview: Option<egui::TextureHandle>,
    thresholds: [String; 3],
    users: Vec<UserRow>,
    roles: Vec<(String, String)>,
    role_cells: Vec<String>,
    titles: Vec<String>,
    user_form: Option<UserForm>,
}

fn describe_age(seconds: f64) -> String {
    if seconds < 90.0 {
        format!("{:.0} 秒前", seconds)
    } else if seconds < 5400.0 {
        format!("{:.0} 分鐘前", seconds / 60.0)
    } else {
        format!("{:.0} 小時前", seconds / 3600.0)
    }
}

impl SettingsPage {
    fn load(&mut self, shared: &mut Shared, user: &Session) {
        let conn = &shared.store.conn;
        self.model = service::recognition_model(conn);
        self.api_key_set = service::api_key(conn).is_some();
        self.camera = Some(CameraForm::from(service::camera_config(conn).unwrap_or_default()));
        let t = service::alert_thresholds(conn).unwrap_or_default();
        self.thresholds = [t.expiry_days.to_string(), t.stale_days.to_string(), t.pending_hours.to_string()];
        self.roles = service::roles(conn);
        self.role_cells = self.roles.iter().map(|(_, l)| l.clone()).collect();
        self.titles = service::values(conn, "job_title");
        if user.can("user.manage") {
            self.users = service::users(&shared.store, user).unwrap_or_default();
        }
        self.loaded = Some(shared.generation);
    }

    pub fn show(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        if self.loaded != Some(shared.generation) {
            self.load(shared, user);
        }
        if let Some(r) = self.test.poll() {
            self.preview = r.image.map(|img| ui.ctx().load_texture("camera-test", img, egui::TextureOptions::LINEAR));
            self.result = Some((r.ok, r.text));
        }
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(1100.0);
            page_title(
                ui,
                "系統設定",
                Some("整廠共用的設定。品項、廠商、機台那些「資料」在「基本資料」，這裡是「系統怎麼運作」。"),
            );
            card(ui, |ui| {
                let mut tabs = vec!["影像設定", "提醒門檻"];
                if user.can("user.manage") {
                    tabs.push("人員與權限");
                }
                ui.horizontal(|ui| {
                    for (i, t) in tabs.iter().enumerate() {
                        let selected = self.tab == i;
                        let text = RichText::new(*t)
                            .font(if selected { bold_font(16.0) } else { font(16.0) })
                            .color(if selected { PRIMARY } else { super::theme::TEXT });
                        let r = ui.add(egui::Button::new(text).frame(false));
                        if selected {
                            ui.painter().hline(
                                r.rect.x_range(),
                                r.rect.bottom() + 3.0,
                                egui::Stroke::new(2.5, PRIMARY),
                            );
                        }
                        if r.clicked() {
                            self.tab = i;
                        }
                    }
                });
                divider(ui);
                match self.tab {
                    0 => self.vision(ui, shared, user),
                    1 => self.alerts(ui, shared, user),
                    _ => self.people(ui, shared, user),
                }
            });
        });
        self.user_modal(ui, shared, user);
    }

    fn vision(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        ui.label(RichText::new("辨識模型").font(bold_font(17.0)));
        ui.label(muted("讀標籤型號與驗收章的模型。秒數是拿現場實照量出來的中位值，不是估計。"));
        for (value, label, note) in RECOGNITION_MODELS {
            let selected = self.model == *value;
            ui.horizontal(|ui| {
                if ui.radio(selected, *label).clicked()
                    && !selected
                    && shared
                        .act(
                            service::save_recognition_model(&shared.store, user, value),
                            Some("已更改辨識模型，下一張照片就會用新的"),
                        )
                        .is_some()
                {
                    self.model = value.to_string();
                }
                ui.label(small(*note));
            });
        }
        ui.add_space(10.0);
        ui.label(RichText::new("辨識 API 金鑰（Google Gemini）").font(bold_font(17.0)));
        if service::api_key_from_env() {
            ui.label(muted("目前使用環境變數 GEMINI_API_KEY 的金鑰。"));
        } else {
            ui.horizontal(|ui| {
                password_input(
                    ui,
                    &mut self.api_key,
                    if self.api_key_set { "已設定，輸入新的會取代" } else { "貼上金鑰" },
                    380.0,
                );
                if primary(ui, "儲存金鑰").clicked() && !self.api_key.trim().is_empty() {
                    let key = self.api_key.clone();
                    if shared.act(service::save_api_key(&shared.store, user, &key), Some("已儲存金鑰")).is_some() {
                        self.api_key.clear();
                    }
                }
                if self.api_key_set && link_danger(ui, "清除", true).clicked() {
                    shared.act(service::save_api_key(&shared.store, user, ""), Some("已清除金鑰"));
                }
            });
            ui.label(small(if self.api_key_set {
                "已設定。金鑰不會顯示出來。"
            } else {
                "沒有金鑰時，拍照仍會留存影像，但不會辨識 —— 一律改人工選批次。"
            }));
        }

        ui.add_space(14.0);
        ui.label(RichText::new("影像來源").font(bold_font(17.0)));
        alert(
            ui,
            Kind::Info,
            "接一台固定相機當影像來源",
            "接上之後，領用登錄多一個「拍這一箱」—— 拍完直接辨識，跟選照片檔走完全一樣的流程。資料夾來源可以接工業相機（Hikrobot 之類）的存圖資料夾，也可以接平板內建相機 App 的存檔資料夾（例如「圖片\\本機照片」）。",
        );
        ui.add_space(6.0);
        let Some(form) = &mut self.camera else { return };
        let mut save = false;
        let mut test = false;
        let mut trigger_test = false;
        egui::Grid::new("camera").num_columns(2).spacing([16.0, 12.0]).show(ui, |ui| {
            ui.label("啟用");
            toggle(ui, &mut form.config.enabled);
            ui.end_row();
            ui.label("連線方式");
            ui.vertical(|ui| {
                let options = ["http", "raw", "folder"];
                let labels = ["HTTP snapshot", "原始 TCP", "存檔資料夾"];
                ui.horizontal(|ui| {
                    for (o, l) in options.iter().zip(labels) {
                        ui.radio_value(&mut form.config.transport, o.to_string(), l);
                    }
                });
                ui.label(small(match form.config.transport.as_str() {
                    "http" => "抓一個 snapshot 網址（多數 IP 相機）",
                    "raw" => "直接開 TCP socket 讀影像",
                    _ => "取資料夾裡最新的一張圖",
                }));
            });
            ui.end_row();
            if form.config.transport == "folder" {
                ui.label("影像資料夾");
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        text_input(ui, &mut form.config.folder, r"例 C:\Users\Public\Pictures\Camera", 460.0);
                        #[cfg(windows)]
                        if secondary(ui, "瀏覽…").clicked()
                            && let Some(dir) = rfd::FileDialog::new().pick_folder()
                        {
                            form.config.folder = dir.display().to_string();
                        }
                    });
                    ui.label(small("相機軟體存圖的地方。網路分享資料夾可以用 \\\\電腦名稱\\分享名稱"));
                });
                ui.end_row();
                ui.label("TCP 觸發（選填）");
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        text_input(ui, &mut form.config.host, "相機 IP，例 169.254.187.1", 220.0);
                        ui.label("埠");
                        number_input(ui, &mut form.port, "8192", 80.0);
                        ui.label("字串");
                        text_input(ui, &mut form.config.trigger, r"例 T1\r\n", 120.0);
                    });
                    ui.label(small("填了的話，領用畫面的「拍這一箱」會先送這個字串叫相機拍照，再等資料夾出現新照片。字串要跟相機軟體（SCMVS）通訊觸發的設定一致。"));
                });
                ui.end_row();
            } else {
                ui.label("IP 位址");
                ui.horizontal(|ui| {
                    text_input(ui, &mut form.config.host, "例 192.168.1.50", 220.0);
                    ui.label("連接埠");
                    number_input(ui, &mut form.port, "80", 80.0);
                });
                ui.end_row();
                if form.config.transport == "http" {
                    ui.label("影像路徑");
                    text_input(ui, &mut form.config.path, "/snapshot.jpg", 300.0);
                    ui.end_row();
                    ui.label("帳號");
                    text_input(ui, &mut form.config.username, "沒有就留空", 220.0);
                    ui.end_row();
                    ui.label("密碼");
                    ui.vertical(|ui| {
                        password_input(ui, &mut form.password, if form.has_password { "••••••" } else { "" }, 220.0);
                        ui.label(small(if form.has_password {
                            "已設定，留空不變更"
                        } else {
                            "沒有就留空"
                        }));
                    });
                    ui.end_row();
                } else {
                    ui.label("觸發字串");
                    ui.vertical(|ui| {
                        text_input(ui, &mut form.config.trigger, r"例 SNAP\r\n", 220.0);
                        ui.label(small("連上後先送出的指令，沒有就留空。\\r \\n 會轉成換行字元"));
                    });
                    ui.end_row();
                }
            }
            ui.label("逾時（秒）");
            number_input(ui, &mut form.timeout, "8", 80.0);
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if primary(ui, "儲存").clicked() {
                save = true;
            }
            if ui
                .add_enabled(!self.test.running(), egui::Button::new("測試連線").min_size(Vec2::new(110.0, 40.0)))
                .clicked()
            {
                test = true;
            }
            if form.config.transport == "folder"
                && !form.config.host.is_empty()
                && ui
                    .add_enabled(!self.test.running(), egui::Button::new("測試觸發").min_size(Vec2::new(110.0, 40.0)))
                    .clicked()
            {
                trigger_test = true;
            }
            if self.test.running() {
                ui.spinner();
                ui.label(muted("連線中…"));
            }
            ui.label(muted(format!(
                "目前：{}",
                service::camera_config(&shared.store.conn).map(|c| c.endpoint()).unwrap_or_default()
            )));
        });
        if save {
            match form.to_config() {
                Ok(config) => {
                    if shared.act(service::save_camera(&shared.store, user, config), Some("已儲存影像來源")).is_some()
                    {
                        self.result = None;
                    }
                }
                Err(e) => shared.error(e),
            }
        }
        if trigger_test {
            match form.to_config() {
                Ok(config) => {
                    self.result = None;
                    self.preview = None;
                    self.test = Task::spawn(ui.ctx(), move || match camera::send_trigger(&config) {
                        Ok(reply) => TestResult {
                            ok: true,
                            text: format!(
                                "已送出觸發到 {}:{}。相機回覆：{}",
                                config.host,
                                config.port,
                                reply.unwrap_or_else(|| "（沒有回覆）".into())
                            ),
                            image: None,
                        },
                        Err(e) => TestResult { ok: false, text: e.to_string(), image: None },
                    });
                }
                Err(e) => shared.error(e),
            }
        }
        if test {
            // Test what is on the form, with the stored password if the field
            // is blank — so a setting can be checked before it is saved.
            match form.to_config() {
                Ok(mut config) => {
                    if config.password.is_empty() {
                        config.password =
                            service::camera_config(&shared.store.conn).map(|c| c.password).unwrap_or_default();
                    }
                    self.result = None;
                    self.preview = None;
                    self.test = Task::spawn(ui.ctx(), move || run_test(&config));
                }
                Err(e) => shared.error(e),
            }
        }
        if let Some((ok, text)) = &self.result {
            ui.add_space(8.0);
            if *ok {
                alert(ui, Kind::Success, "相機正常", text);
            } else {
                alert(ui, Kind::Error, "連不上相機", text);
            }
        }
        if let Some(tex) = &self.preview {
            ui.add_space(6.0);
            let size = tex.size_vec2();
            let scale = (520.0 / size.x).min(360.0 / size.y).min(1.0);
            ui.image((tex.id(), size * scale));
        }
    }

    fn alerts(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        alert(
            ui,
            Kind::Info,
            "這些數字決定「提醒」頁上什麼時候跳出來",
            "調寬會漏掉真正該處理的；調窄會天天跳、然後被當背景雜訊忽略 —— 兩種失效方式都存在。",
        );
        ui.add_space(8.0);
        let can = user.can("dictionary.manage");
        ui.add_enabled_ui(can, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                let labels = [
                    ("效期將屆（天）", "剩多少天以內開始提醒"),
                    ("呆滯批次（天）", "進貨後放多久還沒領完"),
                    ("明細待補（小時）", "領走後多久還沒補產品名稱"),
                ];
                for (i, (l, h)) in labels.iter().enumerate() {
                    field(ui, l, false, Some(h), |ui| number_input(ui, &mut self.thresholds[i], "", 120.0));
                }
            });
            ui.add_space(8.0);
            if primary(ui, "儲存").clicked() {
                let parse = |s: &String| s.trim().parse::<i64>().unwrap_or(0);
                let t = AlertThresholds {
                    expiry_days: parse(&self.thresholds[0]),
                    stale_days: parse(&self.thresholds[1]),
                    pending_hours: parse(&self.thresholds[2]),
                };
                shared.act(service::save_alert_thresholds(&shared.store, user, t), Some("已更新提醒門檻"));
            }
        });
    }

    fn people(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        ui.label(RichText::new("權限層級").font(bold_font(17.0)));
        ui.label(muted(
            "職位（倉管／廠長／品管）是給人看的；權限層級是系統檢查的三層。名稱可以改成你們的叫法，權限不會跟著變。",
        ));
        egui::Grid::new("roles").striped(true).spacing([20.0, 8.0]).min_row_height(36.0).show(ui, |ui| {
            for h in ["層級", "顯示名稱", "能做的事"] {
                ui.label(RichText::new(h).font(font(14.0)).color(MUTED));
            }
            ui.end_row();
            for (i, (code, label)) in self.roles.clone().iter().enumerate() {
                ui.label(muted(code));
                if text_input(ui, &mut self.role_cells[i], "", 160.0).lost_focus() && self.role_cells[i].trim() != label
                {
                    let value = self.role_cells[i].clone();
                    shared.act(service::rename_role(&shared.store, user, code, &value), Some("已更名"));
                }
                ui.add(egui::Label::new(small(service::permission_summary(code))).wrap());
                ui.end_row();
            }
        });
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("人員").font(bold_font(17.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary(ui, "＋ 新增人員").clicked() {
                    self.user_form = Some(UserForm {
                        id: None,
                        username: String::new(),
                        name: String::new(),
                        title: String::new(),
                        role: "user".into(),
                        password: String::new(),
                        active: true,
                    });
                }
            });
        });
        egui::Grid::new("users").striped(true).spacing([20.0, 8.0]).min_row_height(36.0).show(ui, |ui| {
            for h in ["帳號", "顯示名（簽核用）", "職位", "權限層級", "狀態", ""] {
                ui.label(RichText::new(h).font(font(14.0)).color(MUTED));
            }
            ui.end_row();
            for u in self.users.clone() {
                ui.label(&u.username);
                ui.label(strong(&u.name));
                ui.label(or_dash(u.title.as_deref()));
                ui.label(&u.role_label);
                ui.horizontal(|ui| {
                    if u.active {
                        tag(ui, "啟用", super::theme::OK)
                    } else {
                        tag_grey(ui, "停用")
                    };
                    if u.must_change {
                        tag(ui, "待改密碼", super::theme::ORANGE).on_hover_text("下次登入會被要求改密碼");
                    }
                });
                if link(ui, "編輯").clicked() {
                    self.user_form = Some(UserForm {
                        id: Some(u.id),
                        username: u.username.clone(),
                        name: u.name.clone(),
                        title: u.title.clone().unwrap_or_default(),
                        role: u.role.clone(),
                        password: String::new(),
                        active: u.active,
                    });
                }
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        ui.label(small("管理者建的帳號第一次登入會被強制改密碼，否則管理者永遠知道別人的密碼。密碼只要求長度 ≥ 8。"));
    }

    fn user_modal(&mut self, ui: &mut Ui, shared: &mut Shared, user: &Session) {
        let Some(f) = &mut self.user_form else { return };
        let mut close = false;
        let mut save = false;
        let roles = self.roles.clone();
        let titles = self.titles.clone();
        egui::Modal::new(egui::Id::unique("user-form")).show(ui.ctx(), |ui| {
            ui.set_width(460.0);
            ui.label(
                RichText::new(if f.id.is_some() { format!("編輯 {}", f.username) } else { "新增人員".into() })
                    .font(bold_font(20.0)),
            );
            ui.add_space(6.0);
            egui::Grid::new("user-grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                ui.label("帳號");
                ui.add_enabled_ui(f.id.is_none(), |ui| text_input(ui, &mut f.username, "登入用，例 kuo", 260.0));
                ui.end_row();
                ui.label("顯示名");
                ui.vertical(|ui| {
                    text_input(ui, &mut f.name, "例 郭", 260.0);
                    ui.label(small("會寫進收貨單的記錄人與系統日誌"));
                });
                ui.end_row();
                ui.label("職位");
                suggest_input(ui, "user-title", &mut f.title, &titles, "例 倉管", 260.0);
                ui.end_row();
                ui.label("權限層級");
                ui.vertical(|ui| {
                    for code in ROLES {
                        let label = roles.iter().find(|(c, _)| c == code).map(|(_, l)| l.clone()).unwrap_or_default();
                        ui.radio_value(
                            &mut f.role,
                            code.to_string(),
                            format!("{label}　").to_string() + &service::permission_summary(code),
                        );
                    }
                });
                ui.end_row();
                ui.label(if f.id.is_some() { "重設密碼" } else { "初始密碼" });
                ui.vertical(|ui| {
                    password_input(
                        ui,
                        &mut f.password,
                        if f.id.is_some() { "留空不變更" } else { "至少 8 個字元" },
                        260.0,
                    );
                    ui.label(small("對方第一次登入會被要求改掉"));
                });
                ui.end_row();
                if f.id.is_some() {
                    ui.label("啟用");
                    toggle(ui, &mut f.active);
                    ui.end_row();
                }
            });
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
            let result = match f.id {
                None => service::create_user(
                    &shared.store,
                    user,
                    &NewUser {
                        username: f.username.clone(),
                        name: f.name.clone(),
                        title: Some(f.title.clone()),
                        role: f.role.clone(),
                        password: f.password.clone(),
                    },
                )
                .map(|_| ()),
                Some(id) => service::update_user(
                    &shared.store,
                    user,
                    id,
                    &UserPatch {
                        name: Some(f.name.clone()),
                        title: Some(f.title.clone()),
                        role: Some(f.role.clone()),
                        password: Some(f.password.clone()).filter(|p| !p.is_empty()),
                        active: Some(f.active),
                    },
                ),
            };
            if shared.act(result, Some("已儲存")).is_some() {
                close = true;
            }
        }
        if close {
            self.user_form = None;
        }
    }
}

fn run_test(config: &CameraConfig) -> TestResult {
    let started = Instant::now();
    match camera::capture(config) {
        Ok(frame) => {
            let source = frame
                .source
                .as_ref()
                .map(|s| format!("讀到 {}（{}），", s.name, describe_age(s.age_seconds)))
                .unwrap_or_default();
            let image = super::images::decode(&frame.data).ok();
            let dims = image.as_ref().map(|i| format!("，{}×{}", i.size[0], i.size[1])).unwrap_or_default();
            TestResult {
                ok: true,
                text: format!(
                    "{source}{} KB{dims}，耗時 {} ms（{}）",
                    frame.data.len() / 1024,
                    started.elapsed().as_millis(),
                    config.endpoint()
                ),
                image,
            }
        }
        // The folder is readable and the path is right — the camera just is
        // not taking pictures. Saying that plainly is the point of this button.
        Err(CameraError::Stale { message, .. }) => {
            TestResult { ok: false, text: format!("資料夾讀得到，但{message}"), image: None }
        }
        Err(e) => TestResult { ok: false, text: e.to_string(), image: None },
    }
}
