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

use ybb::engine::{self, Config, Level, Msg};
use ybb::input::InputMode;
use ybb::queue::parse_queue;

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

fn main() -> eframe::Result<()> {
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
        }
    }

    fn rounds(&self) -> usize {
        parse_queue(&self.queue_text).len()
    }

    fn push(&mut self, level: Level, text: String) {
        self.log.push(LogLine { level, text });
        if self.log.len() > 3000 {
            self.log.drain(0..800);
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
            max_reroll: 12,
            use_interception: true,
            input: self.input_mode,
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

    fn draw_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("YBB").size(26.0).strong().color(ACCENT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("v1.1").size(10.5).color(SUB));
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
                ui.horizontal(|ui| {
                    ui.label(RichText::new("作者  歪歪子").size(10.5).color(SUB));
                    ui.add_space(6.0);
                    self.support_button(ui);
                    ui.add_space(4.0);
                    let gh = ui
                        .add(
                            egui::Label::new(RichText::new("GitHub").size(10.5).color(ACCENT))
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
                                RichText::new("点击用浏览器打开")
                                    .size(9.5)
                                    .color(ACCENT),
                            );
                        });
                    if gh.clicked() {
                        open_url(GITHUB_URL);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new("1443423104@qq.com")
                                .size(10.5)
                                .color(Color32::from_rgb(0xA6, 0xA0, 0x96)),
                        );
                    });
                });
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
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
