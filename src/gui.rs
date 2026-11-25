//! YBB 图形界面。
//!
//! 布局参照原版 MBB：标题 / 选卡队列 / 开始停止 / 运行日志。
//! 视觉走低饱和暖白纸感，边框和圆角都收着来，避免刺眼的纯黑纯白。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use eframe::egui;
use egui::{
    Align, Color32, Rounding, FontData, FontDefinitions, FontFamily, Layout, Margin, RichText,
    Stroke,
};

use ybb::engine::{self, Config, Level, Msg, Phase};
use ybb::input::InputMode;
use ybb::queue::parse_queue;
use ybb::presets::{self, PresetMap};

const BG: Color32 = Color32::from_rgb(0xFA, 0xF8, 0xF4);
const CARD: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const LINE: Color32 = Color32::from_rgb(0xE8, 0xE3, 0xDA);
const INK: Color32 = Color32::from_rgb(0x33, 0x31, 0x2E);
const SUB: Color32 = Color32::from_rgb(0x8A, 0x85, 0x7D);
const ACCENT: Color32 = Color32::from_rgb(0x5F, 0x84, 0x8C);
const ACCENT_SOFT: Color32 = Color32::from_rgb(0xE7, 0xEF, 0xF0);
const OKC: Color32 = Color32::from_rgb(0x64, 0x8A, 0x6C);
const WARN: Color32 = Color32::from_rgb(0xA8, 0x7F, 0x42);
const ERRC: Color32 = Color32::from_rgb(0xAE, 0x5E, 0x45);
const LOG_BG: Color32 = Color32::from_rgb(0xFB, 0xF9, 0xF5);
const GITHUB_URL: &str = "https://github.com/laz119/FeedTheCups-AutoPick-YBB/tree/main";

/// 运行日志最多保留多少行；超过就丢掉最旧的，防止长时间挂机内存一直涨
const MAX_LOG_LINES: usize = 1200;
/// 单条日志最多保留多少字符，避免异常信息把日志撑爆
const MAX_LOG_LINE_CHARS: usize = 800;

/// 用系统默认浏览器打开链接
fn open_url(url: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    let op = wide("open");
    let file = wide(url);
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// 赞赏码：编进二进制，绿色版不依赖外部文件
fn load_sponsor(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let bytes = include_bytes!("../assets/sponsor.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        img.as_raw(),
    );
    Some(ctx.load_texture("sponsor", color, egui::TextureOptions::LINEAR))
}

/// 最后两轮提醒：响两下
fn beep_twice() {
    std::thread::spawn(|| {
        use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;
        unsafe {
            MessageBeep(MB_ICONASTERISK);
            std::thread::sleep(std::time::Duration::from_millis(260));
            MessageBeep(MB_ICONASTERISK);
        }
    });
}

/// 把编进二进制里的 PNG 解成 egui 需要的形式，用作窗口/任务栏图标
fn load_icon() -> std::sync::Arc<egui::IconData> {
    let bytes = include_bytes!("../assets/app256.png");
    match image::load_from_memory(bytes) {
        Ok(img) => {
            let img = img.to_rgba8();
            let (w, h) = img.dimensions();
            std::sync::Arc::new(egui::IconData {
                rgba: img.into_raw(),
                width: w,
                height: h,
            })
        }
        Err(_) => std::sync::Arc::new(egui::IconData {
            rgba: vec![0; 4],
            width: 1,
            height: 1,
        }),
    }
}

/// 只允许一个 YBB 实例，避免两个窗口同时给游戏发按键。

fn single_instance_ok() -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = "Local\\YBB_SingleInstance\0".encode_utf16().collect();
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if h.is_null() {
            return true;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            windows_sys::Win32::Foundation::CloseHandle(h);
            return false;
        }
        // 句柄故意不关：进程退出时由系统释放，期间一直占住互斥体
        true
    }
}

fn main() -> eframe::Result<()> {
    ybb::assets::chdir_to_app();
    if !single_instance_ok() {
        if let Some(w) = ybb::win::find("YBB ·") {
            ybb::win::force_foreground(w.hwnd);
        }
        return Ok(());
    }

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(load_icon())
            .with_inner_size([372.0, 660.0])
            .with_min_inner_size([340.0, 600.0])
            .with_position([60.0, 30.0])
            .with_title("YBB · 杯杯倒满 地铁自动选卡"),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "YBB",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for path in [
        "C:/Windows/Fonts/Deng.ttf",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("cjk".to_owned(), FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".to_owned());
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push("cjk".to_owned());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = BG;
    style.visuals.extreme_bg_color = CARD;
    style.visuals.faint_bg_color = LOG_BG;
    style.visuals.override_text_color = Some(INK);
    style.visuals.selection.bg_fill = ACCENT_SOFT;
    style.visuals.selection.stroke = Stroke::new(1.0f32, ACCENT);
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    // 菜单：Chrome 风格的白卡片、圆角、轻阴影
    style.spacing.menu_margin = Margin::same(8.0);
    style.spacing.menu_width = 284.0;
    style.spacing.menu_spacing = 6.0;
    style.visuals.menu_rounding = Rounding::same(10.0);
    style.visuals.window_rounding = Rounding::same(10.0);
    style.visuals.window_fill = Color32::WHITE;
    style.visuals.window_stroke = Stroke::new(1.0f32, Color32::from_gray(222));
    style.visuals.popup_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 6.0),
        blur: 18.0,
        spread: 0.0,
        color: Color32::from_black_alpha(38),
    };
    // 赞赏码悬停要跟手：默认 0.5s 延迟太慢了
    style.interaction.tooltip_delay = 0.0;
    style.interaction.tooltip_grace_time = 0.0;
    ctx.set_style(style);
}

fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0f32, LINE))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::same(14.0))
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(13.0)
            .color(SUB)
            .strong(),
    );
    ui.add_space(2.0);
}

struct LogLine {
    level: Level,
    text: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Settings {
    max_reroll: u32,
    #[serde(default = "default_true")]
    preload_model: bool,
    #[serde(default)]
    skip_driver_prompt: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_reroll: 12,
            preload_model: true,
            skip_driver_prompt: false,
        }
    }
}

fn load_settings() -> Settings {
    std::fs::read_to_string("settings.json")
        .ok()
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
        .map(|s| Settings {
            max_reroll: s.max_reroll.min(99),
            preload_model: s.preload_model,
            skip_driver_prompt: s.skip_driver_prompt,
        })
        .unwrap_or_default()
}

fn save_settings(s: &Settings) {
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write("settings.json", text);
    }
}


fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}


fn phase_text(p: Phase) -> &'static str {
    match p {
        Phase::Idle => "待机",
        Phase::WaitingScreen => "等待选卡画面",
        Phase::Reading => "识别卡名",
        Phase::MovingFocus => "移动焦点",
        Phase::Confirming => "确认选卡",
        Phase::WaitingNext => "等待下一屏",
        Phase::Rerolling => "重抽",
        Phase::Done => "完成",
    }
}



/// 自绘开关行：整行可点，右侧一个胶囊开关。
fn toggle_row(ui: &mut egui::Ui, on: &mut bool, label: &str) -> bool {
    let width = ui.available_width();
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, 6.0, Color32::from_gray(243));
    }
    painter.text(
        rect.left_center() + egui::vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        INK,
    );
    let sw = egui::Rect::from_center_size(
        rect.right_center() - egui::vec2(22.0, 0.0),
        egui::vec2(34.0, 18.0),
    );
    painter.rect_filled(
        sw,
        9.0,
        if *on { ACCENT } else { Color32::from_gray(205) },
    );
    let knob_x = if *on { sw.right() - 9.0 } else { sw.left() + 9.0 };
    painter.circle_filled(
        egui::pos2(knob_x, sw.center().y),
        7.0,
        Color32::WHITE,
    );
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.changed()
}

fn menu_section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(7.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new("●").size(7.5).color(ACCENT));
        ui.label(RichText::new(title).size(10.0).strong().color(SUB));
    });
    ui.add_space(1.0);
    ui.separator();
}

fn menu_hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(9.8).color(SUB));
}

struct App {
    ctx: egui::Context,
    queue_text: String,
    log: Vec<LogLine>,
    status: String,
    progress: Option<(usize, usize, String)>,
    running: bool,
    rx: Option<mpsc::Receiver<Msg>>,
    stop: Arc<AtomicBool>,
    go: bool,
    input_mode: InputMode,
    sponsor: Option<egui::TextureHandle>,
    end_warned: bool,
    /// 出错/停止后可以从哪一轮继续（0 起）
    resume_at: Option<usize>,
    /// 每轮最多重抽次数
    max_reroll: u32,
    /// true = 打开程序就预加载模型；false = 只在选卡时加载，跑完释放
    preload_model: bool,
    /// 首次启动检测到没装 Interception 驱动时弹窗
    driver_prompt: bool,
    driver_msg: Option<String>,
    driver_done: bool,
    skip_driver_prompt: bool,
    presets: PresetMap,
    preset_name: String,
    known_cards: Vec<String>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        setup_style(&cc.egui_ctx);

        let queue_text = std::fs::read_to_string("cards.txt")
            .map(|s| {
                s.lines()
                    .filter(|l| !l.trim_start().starts_with('#'))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let settings = load_settings();
        if settings.preload_model {
            ybb::ort_ocr::preload(4);
        }

        Self {
            ctx: cc.egui_ctx.clone(),
            queue_text,
            log: Vec::new(),
            status: "待机".into(),
            progress: None,
            running: false,
            rx: None,
            stop: Arc::new(AtomicBool::new(false)),
            go: true,
            input_mode: InputMode::Keyboard,
            sponsor: load_sponsor(&cc.egui_ctx),
            end_warned: false,
            // 调试用：YBB_UI_RESUME=3 可在启动时直接看「继续」提示条
            resume_at: std::env::var("YBB_UI_RESUME")
                .ok()
                .and_then(|v| v.parse().ok()),
            max_reroll: settings.max_reroll,
            preload_model: settings.preload_model,
            driver_prompt: (!settings.skip_driver_prompt
                && matches!(
                    ybb::input::driver_status(),
                    ybb::input::DriverStatus::NotInstalled
                )),
            driver_msg: None,
            driver_done: false,
            skip_driver_prompt: settings.skip_driver_prompt,
            presets: presets::load(),
            preset_name: String::new(),
            known_cards: std::fs::read_to_string("card_pool.txt")
                .map(|s| {
                    s.lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty() && !l.starts_with('#'))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn rounds(&self) -> usize {
        parse_queue(&self.queue_text).len()
    }

    fn save_preset(&mut self) {
        let mut name = self.preset_name.trim().to_string();
        if name.is_empty() {
            name = format!("预设 {}", self.presets.len() + 1);
        }
        self.presets.insert(name, self.queue_text.clone());
        presets::save(&self.presets);
        self.preset_name.clear();
    }

    fn load_preset(&mut self, name: &str) {
        if let Some(text) = self.presets.get(name) {
            self.queue_text = text.clone();
            self.progress = None;
            self.resume_at = None;
        }
    }

    fn delete_preset(&mut self, name: &str) {
        self.presets.remove(name);
        presets::save(&self.presets);
    }

    fn validation_hints(&self) -> Vec<String> {
        let names = parse_queue(&self.queue_text);
        let mut hints = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for n in &names {
            if !seen.insert(n.clone()) {
                hints.push(format!("队列里重复了「{n}」"));
            }
        }
        if !self.known_cards.is_empty() {
            for n in &names {
                if self.known_cards.iter().any(|k| k == n) {
                    continue;
                }
                if let Some(base) = n.strip_suffix('+') {
                    if self.known_cards.iter().any(|k| k == base) {
                        continue;
                    }
                }
                let close = self
                    .known_cards
                    .iter()
                    .filter(|k| levenshtein(n, k) <= 1)
                    .min_by_key(|k| levenshtein(n, k));
                if let Some(k) = close {
                    hints.push(format!("「{n}」是不是「{k}」？"));
                }
            }
        }
        hints.truncate(4);
        hints
    }

    fn push(&mut self, level: Level, text: String) {
        let text = if text.chars().count() > MAX_LOG_LINE_CHARS {
            let mut t: String = text.chars().take(MAX_LOG_LINE_CHARS).collect();
            t.push('…');
            t
        } else {
            text
        };
        self.log.push(LogLine { level, text });
        if self.log.len() > MAX_LOG_LINES {
            // 一次多丢一些，避免每来一行都搬一次 Vec
            let keep = MAX_LOG_LINES * 3 / 4;
            let drop = self.log.len() - keep;
            self.log.drain(0..drop);
        }
    }

    fn start(&mut self) {
        self.start_at(0, true);
    }

    fn resume(&mut self) {
        if let Some(i) = self.resume_at.take() {
            self.start_at(i, false);
        }
    }

    fn start_at(&mut self, start_index: usize, fresh: bool) {
        if self.running {
            return;
        }
        let queue = parse_queue(&self.queue_text);
        if queue.is_empty() {
            self.push(Level::Err, "队列是空的，先把卡名贴进来".into());
            return;
        }
        let start_index = start_index.min(queue.len() - 1);
        let body = format!("# 由 YBB 自动保存\n{}\n", queue.join("→"));
        let _ = std::fs::write("cards.txt", body);

        if fresh {
            self.log.clear();
            self.progress = None;
        } else {
            self.push(
                Level::Info,
                format!("── 从第 {} 轮继续 ──", start_index + 1),
            );
        }
        self.resume_at = None;
        self.running = true;
        self.stop = Arc::new(AtomicBool::new(false));
        self.end_warned = false;
        self.status = if self.go { "运行中".into() } else { "空跑中".into() };

        let (tx, rx) = mpsc::channel::<Msg>();
        self.rx = Some(rx);

        let cfg = Config {
            queue,
            start_index,
            go: self.go,
            max_reroll: self.max_reroll,
            use_interception: true,
            input: self.input_mode,
            preload_model: self.preload_model,
        };
        let stop = self.stop.clone();
        let ctx = self.ctx.clone();

        std::thread::spawn(move || {
            engine::run(
                cfg,
                move |m| {
                    let _ = tx.send(m);
                    ctx.request_repaint();
                },
                stop,
            );
        });
    }

    fn poll(&mut self) {
        let mut batch: Vec<Msg> = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(m) = rx.try_recv() {
                batch.push(m);
            }
        }
        for m in batch {
            match m {
                Msg::Log(lv, s) => self.push(lv, s),
                Msg::Phase(p) => self.status = phase_text(p).into(),
                Msg::Progress(a, b, t) => {
                    // 进入最后一轮 = 前面都选完了，提醒快结束
                    if a == b && b > 1 && !self.end_warned {
                        self.end_warned = true;
                        self.push(Level::Warn, format!("只剩最后一轮（{a}/{b}）"));
                        beep_twice();
                    }
                    self.progress = Some((a, b, t));
                }
                Msg::Done(r) => {
                    self.running = false;
                    match r {
                        Ok(()) => {
                            self.resume_at = None;
                            self.status = "完成".into();
                            self.push(Level::Ok, "── 全部完成 ──".into());
                        }
                        Err(e) => {
                            self.push(Level::Err, format!("── {e} ──"));
                            if let Some((a, _, _)) = self.progress {
                                self.resume_at = Some(a.saturating_sub(1));
                                self.status = "已停止 · 可继续".into();
                                self.push(
                                    Level::Warn,
                                    format!(
                                        "已停在第 {a} 轮；处理好游戏画面后点「继续」接着跑"
                                    ),
                                );
                            } else {
                                self.status = "已停止".into();
                            }
                        }
                    }
                }
            }
        }
    }

    fn log_text(&self) -> String {
        let mut s = String::new();
        s.push_str("YBB v1.2\n");
        if let Some((a, b, t)) = &self.progress {
            s.push_str(&format!("进度: {a}/{b}  {t}\n"));
        }
        s.push_str(&format!("状态: {}\n", self.status));
        s.push_str("---\n");
        for l in &self.log {
            let tag = match l.level {
                Level::Info => "INFO",
                Level::Step => "STEP",
                Level::Ok => "OK",
                Level::Warn => "WARN",
                Level::Err => "ERR",
            };
            s.push_str(&format!("[{tag}] {}\n", l.text));
        }
        s
    }

    fn copy_log(&mut self, ctx: &egui::Context) {
        let text = self.log_text();
        ctx.output_mut(|o| o.copied_text = text);
        self.push(Level::Ok, "日志已复制到剪贴板".into());
    }

    fn export_log(&mut self) {
        let _ = std::fs::create_dir_all("logs");
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = format!("logs/ybb-log-{secs}.txt");
        match std::fs::write(&path, self.log_text()) {
            Ok(_) => self.push(Level::Ok, format!("日志已导出: {path}")),
            Err(e) => self.push(Level::Err, format!("日志导出失败: {e}")),
        }
    }

    fn settings_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("设置", |ui| {
            ui.set_width(300.0);
            // 菜单内按钮做成「整行悬停」的扁平样式
            ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = Color32::from_gray(243);
            ui.visuals_mut().widgets.active.weak_bg_fill = Color32::from_gray(233);
            ui.spacing_mut().item_spacing.y = 5.0;

            ui.add_space(1.0);
            ui.label(RichText::new("偏好设置").size(14.5).strong().color(INK));
            ui.label(RichText::new("改动会立即保存").size(9.5).color(SUB));
            ui.add_space(3.0);
            ui.separator();

            menu_section(ui, "选卡");
            ui.horizontal(|ui| {
                ui.label(RichText::new("最大重抽").size(12.0).color(INK));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let resp = ui.add_sized(
                        [78.0, 24.0],
                        egui::DragValue::new(&mut self.max_reroll)
                            .range(0..=99)
                            .speed(1.0)
                            .suffix(" 次"),
                    );
                    if resp.changed() {
                        save_settings(&Settings {
                            max_reroll: self.max_reroll,
                            preload_model: self.preload_model,
                            skip_driver_prompt: self.skip_driver_prompt,
                        });
                    }
                });
            });
            menu_hint(ui, "每轮最多重抽几次，0 表示不重抽");

            menu_section(ui, "性能");
            let mut preload = self.preload_model;
            if toggle_row(ui, &mut preload, "启动时预加载模型") {
                self.preload_model = preload;
                save_settings(&Settings {
                    max_reroll: self.max_reroll,
                    preload_model: self.preload_model,
                    skip_driver_prompt: self.skip_driver_prompt,
                });
                if preload {
                    ybb::ort_ocr::preload(4);
                } else {
                    ybb::ort_ocr::clear_cache();
                }
            }
            menu_hint(
                ui,
                if self.preload_model {
                    "打开程序就加载，点开始时不用等"
                } else {
                    "只在选卡时加载，跑完立即释放"
                },
            );

            menu_section(ui, "环境");
            if ui
                .button(RichText::new("安装 Interception 驱动").size(11.5))
                .clicked()
            {
                self.driver_prompt = true;
                self.driver_msg = None;
                self.driver_done = false;
                ui.close_menu();
            }
            menu_hint(ui, "没装的话游戏可能收不到按键");

            menu_section(ui, "队列");
            ui.menu_button(
                RichText::new("队列预设    ›").size(12.0),
                |ui| {
                    ui.set_min_width(228.0);
                    let names: Vec<String> = self.presets.keys().cloned().collect();
                    if names.is_empty() {
                        ui.label(RichText::new("还没有预设").size(10.5).color(SUB));
                    }
                    for name in names {
                        ui.horizontal(|ui| {
                            if ui.button(RichText::new(&name).size(11.5)).clicked() {
                                self.load_preset(&name);
                                ui.close_menu();
                            }
                            if ui.small_button("×").clicked() {
                                self.delete_preset(&name);
                            }
                        });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.preset_name)
                                .hint_text("预设名")
                                .desired_width(126.0),
                        );
                        if ui.button("保存当前").clicked() {
                            self.save_preset();
                        }
                    });
                },
            );

            menu_section(ui, "日志");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if ui.small_button("复制日志").clicked() {
                    let ctx = ui.ctx().clone();
                    self.copy_log(&ctx);
                    ui.close_menu();
                }
                if ui.small_button("导出日志").clicked() {
                    self.export_log();
                    ui.close_menu();
                }
                if ui.small_button("清空日志").clicked() {
                    self.log.clear();
                    ui.close_menu();
                }
            });

            ui.add_space(2.0);
            ui.separator();
            ui.label(
                RichText::new("YBB v1.2  ·  歪歪子")
                    .size(9.5)
                    .color(SUB),
            );
        });
    }

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("YBB").size(26.0).strong().color(ACCENT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("v1.2").size(10.5).color(SUB));
                self.settings_menu(ui);
            });
        });
        ui.label(
            RichText::new("杯杯倒满 · 地铁自动选卡")
                .size(13.0)
                .color(INK),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new("点「开始」后自动切到游戏；按不动就点一下游戏画面")
                .size(10.5)
                .color(SUB),
        );
    }

    /// 页脚「支持作者」：悬停弹出赞赏码
    fn support_button(&self, ui: &mut egui::Ui) {
        ui.scope(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(8.0, 2.0);
            let btn = egui::Button::new(RichText::new("支持作者").size(10.5).color(ACCENT))
                .fill(ACCENT_SOFT)
                .stroke(Stroke::NONE)
                .rounding(Rounding::same(9.0));
            let resp = ui.add(btn).on_hover_cursor(egui::CursorIcon::PointingHand);
            if let Some(tex) = &self.sponsor {
                resp.on_hover_ui(|ui| {
                    ui.set_max_width(212.0);
                    ui.vertical_centered(|ui| {
                        ui.add_space(2.0);
                        ui.image(egui::load::SizedTexture::new(
                            tex.id(),
                            egui::vec2(180.0, 180.0),
                        ));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("微信扫码 · 感谢支持")
                                .size(10.0)
                                .color(SUB),
                        );
                        ui.add_space(2.0);
                    });
                });
            }
        });
    }
    fn driver_prompt_ui(&mut self, ctx: &egui::Context) {
        if !self.driver_prompt {
            return;
        }
        let mut close = false;
        // 背景压暗，突出弹窗
        let screen = ctx.screen_rect();
        egui::Area::new("driver_dim".into())
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.allocate_rect(screen, egui::Sense::hover());
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(55));
            });

        egui::Window::new("driver_prompt")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .frame(
                egui::Frame::none()
                    .fill(Color32::WHITE)
                    .stroke(Stroke::new(1.0f32, LINE))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::same(16.0))
                    .shadow(egui::epaint::Shadow {
                        offset: egui::vec2(0.0, 8.0),
                        blur: 24.0,
                        spread: 0.0,
                        color: Color32::from_black_alpha(45),
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(330.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("安装 Interception 驱动")
                            .size(15.0)
                            .strong()
                            .color(INK),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new("检测到还没安装 Interception 驱动")
                        .size(12.5)
                        .strong()
                        .color(INK),
                );
                ui.add_space(5.0);
                ui.label(
                    RichText::new(
                        "它用于向游戏稳定地发送按键。不装也能用（自动退回 SendInput），但部分游戏可能不响应。",
                    )
                    .size(11.0)
                    .color(SUB),
                );
                ui.add_space(3.0);
                ui.label(
                    RichText::new("安装需要管理员权限，会弹 UAC；装完建议重启一次。")
                        .size(11.0)
                        .color(SUB),
                );
                ui.add_space(10.0);

                let mut skip = self.skip_driver_prompt;
                if ui
                    .checkbox(&mut skip, RichText::new("不再提醒").size(10.5))
                    .changed()
                {
                    self.skip_driver_prompt = skip;
                    save_settings(&Settings {
                        max_reroll: self.max_reroll,
                        preload_model: self.preload_model,
                        skip_driver_prompt: self.skip_driver_prompt,
                    });
                }

                if let Some(msg) = &self.driver_msg {
                    ui.add_space(8.0);
                    egui::Frame::none()
                        .fill(Color32::from_gray(248))
                        .rounding(Rounding::same(6.0))
                        .inner_margin(Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(msg)
                                    .size(10.5)
                                    .color(if self.driver_done { OKC } else { WARN }),
                            );
                        });
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if self.driver_done {
                        let done = egui::Button::new(
                            RichText::new("完成")
                                .size(12.0)
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(ACCENT)
                        .rounding(Rounding::same(6.0))
                        .min_size(egui::vec2(80.0, 30.0));
                        if ui.add(done).clicked() {
                            self.driver_prompt = false;
                        }
                    } else {
                        let install = egui::Button::new(
                            RichText::new("立即安装")
                                .size(12.0)
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(ACCENT)
                        .rounding(Rounding::same(6.0))
                        .min_size(egui::vec2(92.0, 30.0));
                        if ui.add(install).clicked() {
                            self.install_driver();
                        }
                        let later = egui::Button::new(RichText::new("稍后").size(12.0))
                            .rounding(Rounding::same(6.0))
                            .min_size(egui::vec2(64.0, 30.0));
                        if ui.add(later).clicked() {
                            self.driver_prompt = false;
                        }
                    }
                });
            });
        if close {
            self.driver_prompt = false;
        }
    }

    fn install_driver(&mut self) {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let Some(dir) = ybb::assets::find(
            "_interception/Interception/command line installer",
            "install-interception.exe",
        ) else {
            self.driver_msg = Some(
                "找不到安装器。请确认解压完整（_interception 目录要在 YBB 旁边）".into(),
            );
            return;
        };
        let exe = dir.join("install-interception.exe");
        let file: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let params: Vec<u16> = "/install\0".encode_utf16().collect();
        let verb: Vec<u16> = "runas\0".encode_utf16().collect();
        let cwd: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();

        self.driver_msg = Some("正在请求管理员权限…".into());
        let ret = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                params.as_ptr(),
                cwd.as_ptr(),
                SW_SHOWNORMAL,
            )
        };
        if ret as isize > 32 {
            self.driver_done = true;
            self.driver_msg =
                Some("安装程序已启动，请按提示完成；完成后重启电脑。".into());
        } else {
            self.driver_msg = Some(format!(
                "无法启动安装程序（错误码 {}）。可能取消了 UAC。",
                ret as isize
            ));
        }
    }

}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();

        egui::TopBottomPanel::bottom("footer")
            .frame(
                egui::Frame::none()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(14.0, 6.0)),
            )
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                let h = 20.0;
                let full = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), h));

                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(full)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new("作者  歪歪子").size(10.5).color(SUB))
                            .on_hover_text("1443423104@qq.com");
                        ui.add_space(6.0);
                        self.support_button(ui);
                    },
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(full)
                        .layout(Layout::right_to_left(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let gh = ui
                            .add(
                                egui::Label::new(
                                    RichText::new("去 GitHub 获取最新版")
                                        .size(10.5)
                                        .color(ACCENT),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_ui(|ui| {
                                ui.set_max_width(330.0);
                                ui.label(
                                    RichText::new(GITHUB_URL)
                                        .size(10.0)
                                        .monospace()
                                        .color(SUB),
                                );
                                ui.add_space(2.0);
                                ui.label(
                                    RichText::new("点击打开 GitHub 仓库主页")
                                        .size(9.5)
                                        .color(ACCENT),
                                );
                            });
                        if gh.clicked() {
                            open_url(GITHUB_URL);
                        }
                    },
                );
                ui.allocate_space(egui::vec2(rect.width(), h));
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(Margin::same(14.0)))
            .show(ctx, |ui| {
                self.draw_header(ui);
                ui.add_space(10.0);

                // ---- 选卡队列 ----
                section_title(ui, "选卡队列");
                card_frame().show(ui, |ui| {
                    // 竖屏下必须显式限宽，否则 desired_width(INFINITY) 会把内容撑出窗口
                    let w = ui.available_width();
                    egui::ScrollArea::vertical()
                        .id_salt("queue")
                        .max_height(168.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(w);
                            let resp = ui.add(
                                egui::TextEdit::multiline(&mut self.queue_text)
                                    .desired_rows(3)
                                    .desired_width(w)
                                    .font(egui::FontId::proportional(22.0))
                                    .frame(false)
                                    .hint_text("把卡名按顺序贴进来"),
                            );
                            // 改了队列就回到「共 N 轮」，不显示上一轮的旧进度
                            if resp.changed() && !self.running {
                                self.progress = None;
                                self.resume_at = None;
                            }
                        });
                    let hints = self.validation_hints();
                    if !hints.is_empty() {
                        ui.add_space(4.0);
                        for h in hints {
                            ui.label(RichText::new(format!("⚠ {h}")).size(10.0).color(WARN));
                        }
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let total_now = self.rounds();
                        let (cur, total) = match &self.progress {
                            Some((a, b, _)) => (Some(*a), *b),
                            None => (None, total_now),
                        };
                        egui::Frame::none()
                            .fill(ACCENT_SOFT)
                            .rounding(Rounding::same(20.0))
                            .inner_margin(Margin::symmetric(12.0, 4.0))
                            .show(ui, |ui| {
                                if let Some(c) = cur {
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.spacing_mut().item_spacing.x = 3.0;
                                        ui.label(RichText::new("第").size(11.5).color(ACCENT));
                                        ui.label(
                                            RichText::new(c.to_string())
                                                .size(16.0)
                                                .strong()
                                                .color(ACCENT),
                                        );
                                        ui.label(
                                            RichText::new(format!("/ {total} 轮"))
                                                .size(11.5)
                                                .color(ACCENT),
                                        );
                                    });
                                } else {
                                    ui.label(
                                        RichText::new(format!("共 {total} 轮"))
                                            .size(14.0)
                                            .color(ACCENT),
                                    );
                                }
                            });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new("自动存入").size(9.0).color(SUB));
                            ui.add_space(6.0);
                            let can = !self.running && !self.queue_text.trim().is_empty();
                            let test_btn = egui::Button::new(
                                RichText::new("试识别（不按键）").size(9.5).color(ACCENT),
                            )
                            .frame(false);
                            if ui
                                .add_enabled(can, test_btn)
                                .on_hover_text("读当前三张卡和焦点，但不按任何键；用来确认认卡是否正确")
                                .clicked()
                            {
                                self.go = false;
                                self.start();
                            }
                        });
                    });
                });

                ui.add_space(12.0);

                // ---- 按钮（开始 / 停止 / 继续）----
                ui.horizontal(|ui| {
                    let can = !self.running && !self.queue_text.trim().is_empty();

                    let start_btn = egui::Button::new(
                        RichText::new("开始").size(14.0).color(Color32::WHITE).strong(),
                    )
                    .fill(if can { ACCENT } else { Color32::from_rgb(0xC5, 0xCB, 0xCC) })
                    .rounding(Rounding::same(8.0))
                    .min_size(egui::vec2(96.0, 34.0));
                    if ui.add_enabled(can, start_btn).clicked() {
                        self.go = true;
                        self.start();
                    }

                    let stop_btn = egui::Button::new(RichText::new("停止").size(14.0).color(ERRC))
                        .fill(CARD)
                        .stroke(Stroke::new(1.0f32, LINE))
                        .rounding(Rounding::same(8.0))
                        .min_size(egui::vec2(70.0, 34.0));
                    if ui.add_enabled(self.running, stop_btn).clicked() {
                        self.stop.store(true, Ordering::SeqCst);
                        self.status = "正在停止 ...".into();
                    }

                    let resume_label = match self.resume_at {
                        Some(i) => format!("继续 {}", i + 1),
                        None => "继续".to_string(),
                    };
                    let resume_btn = egui::Button::new(
                        RichText::new(resume_label).size(13.0).color(ACCENT),
                    )
                    .fill(CARD)
                    .stroke(Stroke::new(1.0f32, LINE))
                    .rounding(Rounding::same(8.0))
                    .min_size(egui::vec2(70.0, 34.0));
                    let can_resume = !self.running && self.resume_at.is_some();
                    if ui
                        .add_enabled(can_resume, resume_btn)
                        .on_disabled_hover_text("出错或停止后，处理好画面可从当前轮继续")
                        .clicked()
                    {
                        self.resume();
                    }
                });

                // ---- 输入方式 ----
                ui.horizontal(|ui| {
                    ui.label(RichText::new("输入").size(11.5).color(SUB));
                    ui.add_enabled_ui(!self.running, |ui| {
                        ui.selectable_value(&mut self.input_mode, InputMode::Keyboard, "键盘");
                        ui.selectable_value(&mut self.input_mode, InputMode::Gamepad, "手柄")
                            .on_hover_text(
                                "创建虚拟 Xbox360 手柄；需要管理员运行，且实体手柄不能占 1P",
                            );
                    });
                });

                // ---- 状态 ----
                let status_text = match &self.progress {
                    Some((a, b, t)) if self.running => {
                        format!("{} · {a}/{b} · {t}", self.status)
                    }
                    _ => self.status.clone(),
                };
                ui.label(RichText::new(status_text).size(11.5).color(SUB));

                ui.add_space(10.0);

                // ---- 日志 ----
                section_title(ui, "运行日志");
                if let Some(idx) = self.resume_at {
                    if !self.running {
                        egui::Frame::none()
                            .fill(Color32::from_rgb(0xF7, 0xEF, 0xE2))
                            .stroke(Stroke::new(1.0f32, WARN))
                            .rounding(Rounding::same(8.0))
                            .inner_margin(Margin::symmetric(10.0, 7.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("已停在第 {} 轮", idx + 1))
                                            .size(12.0)
                                            .color(WARN)
                                            .strong(),
                                    );
                                    ui.label(
                                        RichText::new("处理好画面后点「继续」接着跑")
                                            .size(11.0)
                                            .color(SUB),
                                    );
                                    ui.with_layout(
                                        Layout::right_to_left(Align::Center),
                                        |ui| {
                                            let btn = egui::Button::new(
                                                RichText::new("继续")
                                                    .size(12.5)
                                                    .color(Color32::WHITE)
                                                    .strong(),
                                            )
                                            .fill(ACCENT)
                                            .rounding(Rounding::same(6.0))
                                            .min_size(egui::vec2(58.0, 26.0));
                                            if ui.add(btn).clicked() {
                                                self.resume();
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(6.0);
                    }
                }
                egui::Frame::none()
                    .fill(LOG_BG)
                    .stroke(Stroke::new(1.0f32, LINE))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::same(10.0))
                    .show(ui, |ui| {
                        let h = (ui.available_height() - 6.0).max(120.0);
                        egui::ScrollArea::vertical()
                            .id_salt("log")
                            .stick_to_bottom(true)
                            .max_height(h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if self.log.is_empty() {
                                    ui.label(
                                        RichText::new("日志会出现在这里")
                                            .size(12.0)
                                            .color(Color32::from_rgb(0xB8, 0xB2, 0xA8)),
                                    );
                                }
                                for l in &self.log {
                                    let (color, mark) = match l.level {
                                        Level::Info => (SUB, "·"),
                                        Level::Step => (INK, "›"),
                                        Level::Ok => (OKC, "✓"),
                                        Level::Warn => (WARN, "!"),
                                        Level::Err => (ERRC, "×"),
                                    };
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing.x = 5.0;
                                        ui.label(RichText::new(mark).size(12.0).color(color));
                                        ui.label(RichText::new(&l.text).size(12.5).color(color));
                                    });
                                }
                            });
                    });
            });

        self.driver_prompt_ui(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
