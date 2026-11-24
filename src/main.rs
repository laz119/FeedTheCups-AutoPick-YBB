//! 杯杯倒满自动选卡（Rust 版）
//!
//! 当前阶段只做「能不能截到画面」的探针：不动游戏、不输入任何按键。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ybb::consts::WINDOW_KEY;
use ybb::maa::{self, Controller, Maa};
use ybb::queue::{parse_queue, parse_queue_file};
use ybb::win;

/// 同时输出到控制台和 run.log，方便事后分析每一段的耗时
mod logfile {
    use std::io::Write;
    use std::sync::OnceLock;

    static START: OnceLock<std::time::Instant> = OnceLock::new();

    pub fn stamp() -> String {
        let t = START.get_or_init(std::time::Instant::now).elapsed();
        format!("[{:>7.2}s]", t.as_secs_f64())
    }

    pub fn init() {
        let _ = START.set(std::time::Instant::now());
        let _ = std::fs::write("run.log", "");
    }

    pub fn append(line: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("run.log")
        {
            let _ = writeln!(f, "{} {}", stamp(), line);
        }
    }
}

macro_rules! logln {
    ($($t:tt)*) => {{
        let line = format!($($t)*);
        println!("{line}");
        logfile::append(&line);
    }};
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "probe" => probe(),
        "tap" => tap_cmd(&args[1..]),
        "run" => run_cmd(&args[1..]),
        "grab" => grab_cmd(&args[1..]),
        "dry" => dry_cmd(&args[1..]),
        "learn" => learn_cmd(&args[1..]),
        "match" => match_cmd(&args[1..]),
        "ocr" => ocr_cmd(&args[1..]),
        "scan" => scan_cmd(&args[1..]),
        "autofocus" => autofocus_cmd(&args[1..]),
        "focus" => focus_cmd(&args[1..]),
        "focust" => focust_cmd(&args[1..]),
        "parse" => parse_cmd(&args[1..]),
        "selftest" => selftest(),
        "-V" | "--version" | "version" => {
            println!("YBB v1.1 —— 杯杯倒满 地铁自动选卡");
            println!("作者: 歪歪子  <1443423104@qq.com>");
            println!("开源协议: MIT（可自由复制修改，保留版权声明即可）");
            println!("查看完整协议: ybb-cli.exe --license");
        }
        "-L" | "--license" | "license" => {
            print!("{}", ybb::LICENSE_TEXT);
        }
        "windows" => list_windows(&args[1..]),
        "focuswin" => {
            let key = args.get(1).cloned().unwrap_or_default();
            match ybb::win::find(&key) {
                Some(w) => {
                    let ok = ybb::win::force_foreground(w.hwnd);
                    let (x, y, cw, ch) = ybb::win::rect(w.hwnd);
                    println!("{} {:?} 前置={} 位置=({x},{y}) 尺寸={cw}x{ch}", w.title, w.hwnd, ok);
                }
                None => println!("没找到标题含 {key:?} 的窗口"),
            }
        }
        _ => {
            println!("用法:");
            println!("  ybb-cli.exe probe            找游戏窗口，试各种截图方式并验证画面正确性");
            println!("  ybb-cli.exe ocr <图片>       对一张截图跑 OCR，认三张卡名并报耗时");
            println!("  ybb-cli.exe focus <图片>     读三张卡名 + 判定焦点在第几张（调试用）");
            println!("  ybb-cli.exe windows [关键字]  列出可见窗口");
            println!("  ybb-cli.exe parse \"卡片串\"   只解析队列，验证分隔符");
        }
    }
}

/// 最保守的输入测试：只按一次方向键，用来确认焦点真的会动
fn tap_cmd(rest: &[String]) {
    let which = rest
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "right".into());
    let key = match which.as_str() {
        "left" => ybb::input::Key::Left,
        "right" => ybb::input::Key::Right,
        _ => die("只支持 left / right"),
    };
    let use_icpt = !rest.iter().any(|a| a == "--sendinput");

    let Some(win) = win::find(WINDOW_KEY) else {
        die("没找到游戏窗口");
    };
    let maa_dir = find_asset("_internal/maa/bin", "MaaFramework.dll")
        .unwrap_or_else(|| die("找不到 MaaFramework.dll"));
    let maa = Arc::new(Maa::load(&maa_dir).expect("加载 MaaFramework 失败"));
    let ctrl = Controller::create(
        maa.clone(),
        win.hwnd,
        maa::screencap::DXGI_DESKTOP_DUP_WINDOW,
        maa::input::INTERCEPTION,
    )
    .unwrap_or_else(|e| die(&e));
    ctrl.connect().unwrap_or_else(|e| die(&e));
    ctrl.set_screenshot_long_side(1920);

    println!("切到前台: {}", win.title);
    let ok = win::force_foreground(win.hwnd);
    std::thread::sleep(std::time::Duration::from_millis(300));
    println!(
        "  设置结果={}  当前前台是否为目标={}",
        ok,
        win::is_foreground(win.hwnd)
    );

    let pad = rest.iter().any(|a| a == "gamepad")
        || rest
            .iter()
            .position(|a| a == "--input")
            .and_then(|i| rest.get(i + 1))
            .map(|v| v.eq_ignore_ascii_case("gamepad") || v.eq_ignore_ascii_case("pad"))
            .unwrap_or(false);
    let mode = if pad {
        ybb::input::InputMode::Gamepad
    } else {
        ybb::input::InputMode::Keyboard
    };
    let inj = ybb::input::build_mode(mode, maa.clone(), win.hwnd, use_icpt, 60)
        .unwrap_or_else(|e| die(&e));
    println!("注入方式: {}", inj.name());
    println!("按一下 {} ...", key.name());

    // 按之前先截一帧，按完再截一帧，用像素差验证"真的动了"
    let before = ctrl.screencap().unwrap_or_else(|e| die(&e));
    let band = ybb::fingerprint::name_band(&ybb::picker::name_rois_abs(&before));

    inj.tap(key).unwrap_or_else(|e| die(&e));
    std::thread::sleep(std::time::Duration::from_millis(400));

    let after = ctrl.screencap().unwrap_or_else(|e| die(&e));
    let d = ybb::fingerprint::band_diff(&before, &after, band);
    println!("卡名带像素差 = {:.2}", d);
    if d > 0.5 {
        println!("=> 画面变了，按键生效");
    } else {
        println!("=> 画面没变：要么焦点已经在边上推不动，要么按键没被游戏接受");
    }
}

/// 主循环
fn run_cmd(rest: &[String]) {
    let go = rest.iter().any(|a| a == "--go");
    let use_icpt = !rest.iter().any(|a| a == "--sendinput");
    let max_reroll: u32 = rest
        .iter()
        .position(|a| a == "--max-reroll")
        .and_then(|i| rest.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);

    let mut pos: Vec<String> = Vec::new();
    let mut it = rest.iter().peekable();
    while let Some(a) = it.next() {
        if a == "--max-reroll" || a == "--input" {
            it.next(); // 跳过它的值，别当成卡名
            continue;
        }
        if a.starts_with("--") {
            continue;
        }
        pos.push(a.clone());
    }
    let queue = if pos.is_empty() {
        parse_queue_file(&std::fs::read_to_string("cards.txt").unwrap_or_default())
    } else {
        parse_queue(&pos.join(" "))
    };
    if queue.is_empty() {
        die("队列是空的");
    }

    logfile::init();
    logln!("共 {} 轮{}", queue.len(), if go { "" } else { "（空跑，不按键）" });

    let input_mode = if rest.iter().any(|a| a == "--input") {
        let v = rest
            .iter()
            .position(|a| a == "--input")
            .and_then(|i| rest.get(i + 1))
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        if v == "gamepad" || v == "pad" {
            ybb::input::InputMode::Gamepad
        } else {
            ybb::input::InputMode::Keyboard
        }
    } else {
        ybb::input::InputMode::Keyboard
    };
    let cfg = ybb::engine::Config {
        queue,
        start_index: 0,
        go,
        max_reroll,
        use_interception: use_icpt,
        input: input_mode,
    };
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    ybb::engine::run(
        cfg,
        move |m| match m {
            ybb::engine::Msg::Log(lv, s) => {
                let _ = lv;
                logln!("{s}");
            }
            ybb::engine::Msg::Progress(a, b, t) => {
                logln!("\n第 {a}/{b} 轮  目标={t:?}");
            }
            ybb::engine::Msg::Done(r) => match r {
                Ok(()) => {}
                Err(e) => logln!("停止：{e}"),
            },
        },
        stop,
    );
}


/// 连拍：抓 N 帧存到目录，同时打印相邻帧的卡名带差值（标定判稳阈值用）
fn grab_cmd(rest: &[String]) {
    let pos: Vec<String> = rest.iter().filter(|a| !a.starts_with("--")).cloned().collect();
    let dir = pos.first().cloned().unwrap_or_else(|| "frames".into());
    let count: usize = pos.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let interval: u64 = pos.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);

    let (maa, _tasker) = init_ocr();
    let Some(win) = win::find(WINDOW_KEY) else {
        die("没找到游戏窗口");
    };
    let ctrl = Controller::create(
        maa.clone(),
        win.hwnd,
        maa::screencap::DXGI_DESKTOP_DUP_WINDOW,
        maa::input::INTERCEPTION,
    )
    .unwrap_or_else(|e| die(&e));
    ctrl.connect().unwrap_or_else(|e| die(&e));
    ctrl.set_screenshot_long_side(1920);

    std::fs::create_dir_all(&dir).unwrap_or_else(|e| die(&format!("建目录失败: {e}")));

    println!("连拍 {count} 帧，间隔 {interval}ms，存到 {dir}");
    println!("（请让游戏停在选卡界面别动）\n");
    println!("{:>4} {:>8} {:>12}", "帧", "截图ms", "与上帧卡名带差");

    let mut prev: Option<ybb::maa::Image> = None;
    let mut diffs: Vec<f64> = Vec::new();
    let mut caps: Vec<f64> = Vec::new();

    for i in 0..count {
        let t0 = Instant::now();
        let Ok(img) = ctrl.screencap() else {
            println!("第 {i} 帧截图失败");
            continue;
        };
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        caps.push(ms);

        let rois = name_rois_abs(&img);
        let band = ybb::fingerprint::name_band(&rois);
        let d = prev
            .as_ref()
            .map(|p| ybb::fingerprint::band_diff(p, &img, band))
            .unwrap_or(f64::NAN);
        if d.is_finite() {
            diffs.push(d);
        }

        let _ = img.save_png(std::path::Path::new(&dir).join(format!("g{i:03}.png")));
        println!("{:>4} {:>8.1} {:>12.3}", i, ms, d);

        prev = Some(img);
        std::thread::sleep(std::time::Duration::from_millis(interval));
    }

    if !diffs.is_empty() {
        let mut v = diffs.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "\n卡名带差值: 最小 {:.3}  中位 {:.3}  最大 {:.3}",
            v[0],
            v[v.len() / 2],
            v[v.len() - 1]
        );
        println!("若最大值远小于 1.5（原版阈值），说明判稳可以更激进");
        let within = diffs.iter().filter(|d| **d < 1.5).count();
        println!(
            "有 {}/{} 帧的差值 < 1.5",
            within,
            diffs.len()
        );
    }
    if !caps.is_empty() {
        let mut v = caps.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("截图耗时: 中位 {:.1}ms", v[v.len() / 2]);
    }
}

/// 空跑：对着正在运行的游戏做一次完整判断，但**不按任何键**
fn dry_cmd(rest: &[String]) {
    // 目标卡：命令行给了就用，否则读 cards.txt 的第一张
    let cards_arg: Vec<String> = rest.iter().filter(|a| !a.starts_with("--")).cloned().collect();
    let queue = if cards_arg.is_empty() {
        let txt = std::fs::read_to_string("cards.txt").unwrap_or_default();
        parse_queue_file(&txt)
    } else {
        parse_queue(&cards_arg.join(" "))
    };
    if queue.is_empty() {
        die("没有目标卡：在命令行里给一个，或把 cards.txt 填上");
    }
    let target = &queue[0];

    let (maa, tasker) = init_ocr();
    let Some(win) = win::find(WINDOW_KEY) else {
        die("没找到游戏窗口");
    };

    let ctrl = Controller::create(
        maa.clone(),
        win.hwnd,
        maa::screencap::DXGI_DESKTOP_DUP_WINDOW,
        maa::input::INTERCEPTION,
    )
    .unwrap_or_else(|e| die(&e));
    ctrl.connect().unwrap_or_else(|e| die(&e));
    ctrl.set_screenshot_long_side(1920);

    let tpl = ybb::bubble::Bubble::load("focus_template.png");
    println!(
        "焦点判定: {}",
        if tpl.is_some() { "模板匹配" } else { "OCR（没找到 focus_template.png）" }
    );

    // 热身：截图 + 完整跑一次识别，把模型预热和缓存都喂满，之后才是稳态数字
    let _ = ctrl.screencap();
    let warm = ybb::picker::settle_and_read(&ctrl, &tasker, tpl.as_ref(), 3000);
    let seed = if std::env::var("YBB_FRAME_REUSE")
        .map(|v| v != "0")
        .unwrap_or(true)
    {
        warm.map(|l| l.frame)
    } else {
        None
    };

    let t_all = Instant::now();
    let Some(look) =
        ybb::picker::settle_and_read_seeded(&ctrl, &tasker, tpl.as_ref(), 3000, seed)
    else {
        die("等不到稳定的选卡画面");
    };
    let t_total = t_all.elapsed().as_secs_f64() * 1000.0;
    let cards = look.cards.clone();
    let focus = look.focus;

    println!(
        "判稳 {:.0}ms（{} 帧） / 整个「看」{:.0}ms\n",
        look.settle_ms, look.shots, t_total
    );

    for (i, c) in cards.iter().enumerate() {
        let mark = if Some(i) == focus { "  ← 焦点" } else { "" };
        println!("  第{}张  {:?}{}", i + 1, c.name, mark);
    }

    // 目标匹配
    let mut best: Option<(usize, f64)> = None;
    for (i, c) in cards.iter().enumerate() {
        if c.is_empty() {
            continue;
        }
        let s = ybb::score::score(target, &c.name);
        if best.map(|(_, bs)| s > bs).unwrap_or(true) {
            best = Some((i, s));
        }
    }

    println!("\n目标: {:?}", target);
    match best {
        Some((i, s)) if s >= ybb::consts::MATCH_THRESHOLD => {
            let from = focus.unwrap_or(0);
            let moves = i as i32 - from as i32;
            println!(
                "  => 第{}张 {:?} 分数 {:.2}，焦点现在第{}张，{}",
                i + 1,
                cards[i].name,
                s,
                from + 1,
                if moves == 0 {
                    "直接确认".to_string()
                } else {
                    format!("{} {} 格后确认", if moves > 0 { "推右" } else { "推左" }, moves.abs())
                }
            );
        }
        Some((i, s)) => println!(
            "  => 没有目标卡（最接近的是第{}张 {:?}，{:.2} < {:.2}）→ 应重抽",
            i + 1,
            cards[i].name,
            s,
            ybb::consts::MATCH_THRESHOLD
        ),
        None => println!("  => 三张都没认出来 → 应重抽"),
    }
    println!("\n（空跑模式：以上只是判断，没有按任何键）");
}

/// 三个卡名 ROI 换成绝对像素 [x, y, w, h]
fn name_rois_abs(img: &ybb::maa::Image) -> [[i32; 4]; 3] {
    let w = img.width as f64;
    let h = img.height as f64;
    let mut out = [[0i32; 4]; 3];
    for (i, f) in ybb::consts::NAME_ROIS.iter().enumerate() {
        let x = (f.0 * w) as i32;
        let y = (f.1 * h) as i32;
        out[i] = [x, y, (f.2 * w) as i32 - x, (f.3 * h) as i32 - y];
    }
    out
}
struct FpOpts {
    dir: String,
    store: String,
    bin: bool,
    stable: bool,
}

fn fp_opts(rest: &[String], default_store: &str) -> FpOpts {
    let pos: Vec<String> = rest.iter().filter(|a| !a.starts_with("--")).cloned().collect();
    FpOpts {
        dir: pos.first().cloned().unwrap_or_else(|| ".".into()),
        store: pos
            .get(1)
            .cloned()
            .unwrap_or_else(|| default_store.to_string()),
        bin: rest.iter().any(|a| a == "--bin"),
        stable: rest.iter().any(|a| a == "--stable"),
    }
}

fn fp_algo(img: &ybb::maa::Image, roi: [i32; 4], bin: bool) -> Option<Vec<f32>> {
    if bin {
        ybb::fingerprint::fingerprint_binary(img, roi)
    } else {
        ybb::fingerprint::fingerprint(img, roi)
    }
}

/// 从目录里的截图学习卡名指纹（用 OCR 当老师）
fn learn_cmd(rest: &[String]) {
    let o = fp_opts(rest, "templates.json");
    let (_maa, tasker) = init_ocr();
    let mut store = ybb::fingerprint::Store::load(&o.store);

    let mut files = list_png(&o.dir);
    files.sort();
    if files.is_empty() {
        die(&format!("{} 里没有 png", o.dir));
    }

    let (mut added, mut empty, mut unstable) = (0, 0, 0);
    let mut prev: Option<ybb::maa::Image> = None;

    for f in &files {
        let Ok(img) = ybb::maa::Image::load_bgr(f) else {
            continue;
        };
        let rois = name_rois_abs(&img);

        if o.stable {
            if let Some(p) = &prev {
                let d = ybb::fingerprint::band_diff(p, &img, ybb::fingerprint::name_band(&rois));
                if d > 1.5 {
                    unstable += 1;
                    prev = Some(img);
                    continue;
                }
            }
        }

        let cards = ybb::picker::read_cards(&tasker, &img, true);
        for (i, c) in cards.iter().enumerate() {
            if c.is_empty() {
                empty += 1;
                continue;
            }
            if let Some(v) = fp_algo(&img, rois[i], o.bin) {
                if store.add(&c.name, v) {
                    added += 1;
                }
            }
        }
        prev = Some(img);
    }

    store
        .save(&o.store)
        .unwrap_or_else(|e| die(&format!("保存失败: {e}")));
    println!(
        "处理 {} 张（{}）：新增 {} 条模板，空卡位 {}，不稳帧跳过 {}",
        files.len(),
        if o.bin { "二值特征" } else { "梯度特征" },
        added,
        empty,
        unstable
    );
    println!(
        "模板库 {} 条，覆盖 {} 张卡，已存 {}",
        store.items.len(),
        store.names().len(),
        o.store
    );
}

/// 用模板库认目录里的截图，与 OCR 对比
fn match_cmd(rest: &[String]) {
    let o = fp_opts(rest, "templates.json");
    let (_maa, tasker) = init_ocr();
    let store = ybb::fingerprint::Store::load(&o.store);
    if store.items.is_empty() {
        die("模板库是空的，先跑 learn");
    }

    let mut files = list_png(&o.dir);
    files.sort();
    if files.is_empty() {
        die(&format!("{} 里没有 png", o.dir));
    }

    let (mut total, mut hit, mut miss, mut none) = (0usize, 0usize, 0usize, 0usize);
    let mut confusions: Vec<(String, String)> = Vec::new();
    let mut us: Vec<f64> = Vec::new();
    let mut prev: Option<ybb::maa::Image> = None;

    for f in &files {
        let Ok(img) = ybb::maa::Image::load_bgr(f) else {
            continue;
        };
        let rois = name_rois_abs(&img);

        if o.stable {
            if let Some(p) = &prev {
                if ybb::fingerprint::band_diff(p, &img, ybb::fingerprint::name_band(&rois)) > 1.5 {
                    prev = Some(img);
                    continue;
                }
            }
        }

        let cards = ybb::picker::read_cards(&tasker, &img, true);
        for (i, c) in cards.iter().enumerate() {
            if c.is_empty() {
                continue;
            }
            let t0 = Instant::now();
            let fp = fp_algo(&img, rois[i], o.bin);
            us.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
            total += 1;

            match fp.and_then(|v| store.best(&v)) {
                Some((name, sim)) if sim >= ybb::fingerprint::MATCH_SIM => {
                    if name == c.name {
                        hit += 1;
                    } else {
                        miss += 1;
                        confusions.push((c.name.clone(), name));
                    }
                }
                Some((name, sim)) => {
                    miss += 1;
                    confusions.push((c.name.clone(), format!("{name}?{sim:.2}")));
                }
                None => none += 1,
            }
        }
        prev = Some(img);
    }

    if total == 0 {
        die("没有可比对的样本");
    }
    us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "样本 {}：命中 {} / 认错 {} / 无特征 {}  => 准确率 {:.1}%   [{}]",
        total,
        hit,
        miss,
        none,
        hit as f64 / total as f64 * 100.0,
        if o.bin { "二值特征" } else { "梯度特征" }
    );
    println!("指纹计算: 中位 {:.0}µs  最慢 {:.0}µs", us[us.len() / 2], us[us.len() - 1]);
    if !confusions.is_empty() {
        println!("\n认错（真值 -> 匹配到）:");
        confusions.sort();
        confusions.dedup();
        for (a, b) in confusions.iter().take(15) {
            println!("  {a}  ->  {b}");
        }
    }
}

fn list_png(dir: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("png")).unwrap_or(false))
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// 对比：模板匹配 vs OCR 找焦点
fn focust_cmd(rest: &[String]) {
    let pos: Vec<String> = rest.iter().filter(|a| !a.starts_with("--")).cloned().collect();
    let Some(path) = pos.first() else {
        eprintln!("用法: ybb-cli.exe focust <图片> [模板png]");
        std::process::exit(2);
    };
    let tpl_path = pos.get(1).cloned().unwrap_or_else(|| "focus_template.png".into());

    let tpl = ybb::bubble::Bubble::load(&tpl_path)
        .unwrap_or_else(|| die(&format!("读不了模板 {tpl_path}")));
    println!("模板: {tpl_path}  尺寸 {}x{}", tpl.size().0, tpl.size().1);

    let (_maa, tasker) = init_ocr();
    let img = ybb::maa::Image::load_bgr(path).unwrap_or_else(|e| die(&e));
    let cards = ybb::picker::read_cards(&tasker, &img, true);
    println!("图像 {}x{}\n", img.width, img.height);

    let t0 = Instant::now();
    let (ocr_idx, _) = ybb::picker::focus_index(&tasker, &img, &cards, true);
    let t_ocr = t0.elapsed().as_secs_f64() * 1000.0;

    let t0 = Instant::now();
    let tpl_idx = ybb::picker::focus_by_template(&img, &cards, &tpl, 0.5);
    let t_tpl = t0.elapsed().as_secs_f64() * 1000.0;

    // 再连跑 20 次看模板匹配的稳定耗时
    let mut v = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        let _ = ybb::picker::focus_by_template(&img, &cards, &tpl, 0.5);
        v.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());

    println!(
        "OCR      : {:?}  {:.0}ms",
        ocr_idx.map(|i| i + 1),
        t_ocr
    );
    println!(
        "模板匹配 : {:?}  {:.1}ms（连跑中位 {:.1}ms）",
        tpl_idx.map(|i| i + 1),
        t_tpl,
        v[v.len() / 2]
    );
    if ocr_idx == tpl_idx {
        println!("\n两者一致");
    } else {
        println!("\n不一致！需要调整模板或阈值");
    }
}

/// 调试：读卡名 + 判焦点，并把 OCR 的原始命中连同坐标全打出来
fn focus_cmd(rest: &[String]) {
    let only_rec = !rest.iter().any(|a| a == "--full");
    let Some(path) = rest.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("用法: ybb-cli.exe focus <图片路径> [--full]");
        std::process::exit(2);
    };

    let (_maa, tasker) = init_ocr();
    let img = ybb::maa::Image::load_bgr(path).unwrap_or_else(|e| die(&e));
    println!("图像: {}x{}\n", img.width, img.height);

    let t0 = Instant::now();
    let cards = ybb::picker::read_cards(&tasker, &img, only_rec);
    let t_cards = t0.elapsed().as_secs_f64() * 1000.0;

    for (i, c) in cards.iter().enumerate() {
        let b = c
            .box_
            .map(|b| format!("[{},{},{},{}]", b[0], b[1], b[2], b[3]))
            .unwrap_or_else(|| "无".into());
        println!("卡位{}: {:?}  box={}", i + 1, c.name, b);
    }
    println!("读卡耗时 {:.0}ms\n", t_cards);

    let t0 = Instant::now();
    let (idx, hits) = ybb::picker::focus_index(&tasker, &img, &cards, only_rec);
    let t_focus = t0.elapsed().as_secs_f64() * 1000.0;

    println!("焦点带 OCR 命中 {} 条（耗时 {:.0}ms）:", hits.len(), t_focus);
    for h in &hits {
        println!(
            "  {:>4},{:>4} {:>4}x{:<4}  score={:.2}  {:?}",
            h.box_[0], h.box_[1], h.box_[2], h.box_[3], h.score, h.text
        );
    }
    match idx {
        Some(i) => println!("\n=> 焦点在第 {} 张（{}）", i + 1, cards[i].name),
        None => println!("\n=> 读不到焦点气泡"),
    }
    println!("单次识别总耗时 {:.0}ms", t_cards + t_focus);
}

/// 加载 Maa + OCR 模型，返回 (maa, tasker)
fn init_ocr() -> (Arc<Maa>, ybb::ocr::Tasker) {
    let maa_dir = find_asset("_internal/maa/bin", "MaaFramework.dll")
        .unwrap_or_else(|| die("找不到 MaaFramework.dll"));
    let model_dir = find_asset("model/ocr", "rec.onnx").unwrap_or_else(|| die("找不到 model/ocr"));

    let maa = Arc::new(Maa::load(&maa_dir).expect("加载 MaaFramework 失败"));
    let res = ybb::ocr::Resource::new(maa.clone()).expect("创建 Resource 失败");
    res.post_ocr_model(&model_dir).expect("加载 OCR 模型失败");
    let tasker = ybb::ocr::Tasker::new(maa.clone()).expect("创建 Tasker 失败");
    tasker.bind_resource(&res).expect("绑定 Resource 失败");
    std::mem::forget(res); // Tasker 已持有资源引用，避免提前释放
    (maa, tasker)
}

fn ocr_cmd(rest: &[String]) {
    let only_rec = rest.iter().any(|a| a == "--only-rec");
    let Some(path) = rest.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("用法: ybb-cli.exe ocr <图片路径> [--only-rec]");
        std::process::exit(2);
    };

    let maa_dir = find_asset("_internal/maa/bin", "MaaFramework.dll")
        .unwrap_or_else(|| die("找不到 MaaFramework.dll"));
    let model_dir = find_asset("model/ocr", "rec.onnx").unwrap_or_else(|| die("找不到 model/ocr"));

    let t_load = Instant::now();
    let maa = Arc::new(Maa::load(&maa_dir).expect("加载 MaaFramework 失败"));
    let res = ybb::ocr::Resource::new(maa.clone()).expect("创建 Resource 失败");
    if let Some(ep) = eval_ep(rest) {
        let ok = res.set_inference(ep, -1);
        print!("推理后端设为 {}（{}）  ", ep_name(ep), if ok { "成功" } else { "失败" });
    }
    res.post_ocr_model(&model_dir).expect("加载 OCR 模型失败");
    let tasker = ybb::ocr::Tasker::new(maa.clone()).expect("创建 Tasker 失败");
    tasker.bind_resource(&res).expect("绑定 Resource 失败");
    println!(
        "Maa {} / 初始化 {:.0}ms / only_rec={}",
        maa.version(),
        t_load.elapsed().as_secs_f64() * 1000.0,
        only_rec
    );

    let img = ybb::maa::Image::load_bgr(path).unwrap_or_else(|e| die(&e));
    println!("图像: {}x{}x{}\n", img.width, img.height, img.channels);

    for (i, frac) in ybb::consts::NAME_ROIS.iter().enumerate() {
        let t0 = Instant::now();
        match tasker.ocr_roi_ex(&img, *frac, only_rec) {
            Ok(items) => {
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                let joined = items
                    .iter()
                    .map(|x| format!("{} ({:.2})", x.text, x.score))
                    .collect::<Vec<_>>()
                    .join(" | ");
                println!("第{}张  {:>6.1}ms  命中 {} 条: {}", i + 1, ms, items.len(), joined);
            }
            Err(e) => println!("第{}张  失败: {e}", i + 1),
        }
    }

    // 冷启动之后连跑 10 轮，看稳定耗时
    let mut times = Vec::new();
    for _ in 0..10 {
        let t0 = Instant::now();
        for frac in ybb::consts::NAME_ROIS {
            let _ = tasker.ocr_roi_ex(&img, frac, only_rec);
        }
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "\n连跑 10 轮（每轮 3 次 OCR）: 最快 {:.1}ms  中位 {:.1}ms  最慢 {:.1}ms  → 单帧卡名识别约 {:.0}ms",
        times[0],
        times[times.len() / 2],
        times[times.len() - 1],
        times[times.len() / 2]
    );
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(2);
}

/// 临时调试：对任意 ROI 做完整 OCR（带检测），看能不能自动找到卡名
fn scan_cmd(rest: &[String]) {
    let Some(path) = rest.iter().find(|a| !a.starts_with("--")) else {
        die("用法: ybb-cli.exe scan <图片> [x0 y0 x1 y1]");
    };
    let nums: Vec<f64> = rest.iter().filter_map(|s| s.parse::<f64>().ok()).collect();
    let roi = if nums.len() >= 4 {
        (nums[0], nums[1], nums[2], nums[3])
    } else {
        (0.05, 0.25, 0.95, 0.85)
    };
    let (_maa, tasker) = init_ocr();
    let img = ybb::maa::Image::load_bgr(path).unwrap_or_else(|e| die(&e));
    println!("图像 {}x{}  ROI {:?}", img.width, img.height, roi);
    match tasker.ocr_roi_ex(&img, roi, false) {
        Ok(hits) => {
            for h in hits {
                println!("box={:?}  score={:.2}  {:?}", h.box_, h.score, h.text);
            }
        }
        Err(e) => println!("失败: {e}"),
    }
}

/// 临时调试：自动布局找卡 + 全局找焦点
fn autofocus_cmd(rest: &[String]) {
    let Some(path) = rest.iter().find(|a| !a.starts_with("--")) else {
        die("用法: ybb-cli.exe autofocus <图片>");
    };
    let (_maa, tasker) = init_ocr();
    let img = ybb::maa::Image::load_bgr(path).unwrap_or_else(|e| die(&e));
    let tpl = ybb::bubble::Bubble::load("focus_template.png");

    let t0 = Instant::now();
    let cards = ybb::picker::read_cards_auto(&tasker, &img);
    let t_cards = t0.elapsed().as_secs_f64() * 1000.0;

    let t0 = Instant::now();
    let focus = ybb::picker::focus_auto_global(&tasker, &img, &cards, tpl.as_ref());
    let t_focus = t0.elapsed().as_secs_f64() * 1000.0;

    println!("图像 {}x{}  自动找卡 {:.0}ms  全局焦点 {:.0}ms", img.width, img.height, t_cards, t_focus);
    for (i, c) in cards.iter().enumerate() {
        let mark = if Some(i) == focus { "  ← 焦点" } else { "" };
        println!("  第{}张  {:?}  box={:?}{}", i + 1, c.name, c.box_, mark);
    }
    println!("焦点: {:?}", focus.map(|i| i + 1));
}

/// 解析 `--ep cpu|directml|webgpu|auto`；没给就返回 None（用默认）
fn eval_ep(args: &[String]) -> Option<i32> {
    let i = args.iter().position(|a| a == "--ep")?;
    let v = args.get(i + 1)?.to_lowercase();
    Some(match v.as_str() {
        "cpu" => 1,
        "directml" | "dml" => 2,
        "webgpu" => 5,
        "auto" => 0,
        other => die(&format!("未知的推理后端: {other}")),
    })
}

fn ep_name(ep: i32) -> &'static str {
    match ep {
        0 => "Auto",
        1 => "CPU",
        2 => "DirectML",
        5 => "WebGPU",
        _ => "?",
    }
}

fn parse_cmd(rest: &[String]) {
    let text = if rest.is_empty() {
        let mut s = String::new();
        use std::io::Read;
        std::io::stdin().read_to_string(&mut s).unwrap();
        s
    } else {
        rest.join(" ")
    };
    let cards = parse_queue(&text);
    println!("共 {} 轮", cards.len());
    for (i, c) in cards.iter().enumerate() {
        println!("  {:2}. {}", i + 1, c);
    }
}

fn list_windows(rest: &[String]) {
    // 不带参数就列出所有可见窗口，方便查游戏窗口的真实标题
    let key = rest.first().cloned().unwrap_or_default();
    let all = win::find_all(&key);
    if all.is_empty() {
        println!("没找到匹配 \"{key}\" 的可见窗口");
        return;
    }
    println!("共 {} 个可见窗口:", all.len());
    for w in all.iter().take(60) {
        println!("  {:?}  {}", w.hwnd, w.title);
    }
}

/// 找一个资源目录：环境变量优先，其次从当前目录/可执行文件目录往上找。
fn find_asset(rel: &str, marker: &str) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("YBB_ASSET_ROOT") {
        let p = PathBuf::from(p).join(rel);
        if p.join(marker).exists() {
            return Some(p);
        }
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(d) = std::env::current_dir() {
        roots.push(d);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            roots.push(d.to_path_buf());
        }
    }
    for root in roots {
        let mut cur = Some(root.as_path());
        while let Some(d) = cur {
            let cand = d.join(rel);
            if cand.join(marker).exists() {
                return Some(cand);
            }
            cur = d.parent();
        }
    }
    None
}

fn probe() {
    println!("========== 截图探针 ==========");
    println!("依次尝试 5 种截图方式：先测耗时，再用 OCR 验证截出来的画面对不对。");
    println!("全程不按键、不动游戏、不消耗游戏币。");
    println!();

    let Some(maa_dir) = find_asset("_internal/maa/bin", "MaaFramework.dll") else {
        eprintln!("找不到 MaaFramework.dll，请把 ybb-cli.exe 放在 YBB 目录下运行。");
        std::process::exit(2);
    };
    let Some(model_dir) = find_asset("model/ocr", "rec.onnx") else {
        eprintln!("找不到 model/ocr 目录，请把 ybb-cli.exe 放在 YBB 目录下运行。");
        std::process::exit(2);
    };

    let maa = Arc::new(Maa::load(&maa_dir).expect("加载 MaaFramework 失败"));
    println!("MaaFramework 版本: {}", maa.version());

    let res = ybb::ocr::Resource::new(maa.clone()).expect("创建 Resource 失败");
    res.post_ocr_model(&model_dir).expect("加载 OCR 模型失败");
    let tasker = ybb::ocr::Tasker::new(maa.clone()).expect("创建 Tasker 失败");
    tasker.bind_resource(&res).expect("绑定 Resource 失败");
    println!("OCR 模型已加载\n");

    let Some(win) = win::find(WINDOW_KEY) else {
        eprintln!("没找到标题含 \"{WINDOW_KEY}\" 的窗口。");
        eprintln!("1) 游戏先开起来，并切成窗口 / 无边框模式（独占全屏截不到）");
        eprintln!("2) 如果游戏已经开着，双击「列出窗口.bat」看看它真实的标题");
        std::process::exit(3);
    };
    println!("窗口: {}  {:?}\n", win.title, win.hwnd);

    let methods: [(&str, u64); 5] = [
        ("FramePool", maa::screencap::FRAME_POOL),
        ("PrintWindow", maa::screencap::PRINT_WINDOW),
        ("GDI", maa::screencap::GDI),
        ("DXGI_Window", maa::screencap::DXGI_DESKTOP_DUP_WINDOW),
        ("ScreenDC", maa::screencap::SCREEN_DC),
    ];

    let shots_dir = PathBuf::from("shots");
    let _ = std::fs::create_dir_all(&shots_dir);

    let mut rows: Vec<MethodRow> = Vec::new();

    for (name, method) in methods {
        let ctrl = match Controller::create(maa.clone(), win.hwnd, method, maa::input::INTERCEPTION) {
            Ok(c) => c,
            Err(e) => {
                println!("{name:<13} 创建失败: {e}");
                continue;
            }
        };
        if let Err(e) = ctrl.connect() {
            println!("{name:<13} {e}");
            continue;
        }
        ctrl.set_screenshot_long_side(1920);

        // 连拍 5 次取中位，避开首帧开销
        let mut times = Vec::new();
        let mut last: Option<ybb::maa::Image> = None;
        for _ in 0..5 {
            let t0 = Instant::now();
            if let Ok(img) = ctrl.screencap() {
                times.push(t0.elapsed().as_secs_f64() * 1000.0);
                last = Some(img);
            }
        }
        let Some(img) = last else {
            println!("{name:<13} 连接OK  截图全部失败");
            continue;
        };
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = times[times.len() / 2];

        let _ = img.save_png(shots_dir.join(format!("{}.png", name.to_lowercase())));

        // 用 OCR 验证这张图的**几何位置**对不对
        let mut names = Vec::new();
        for frac in ybb::consts::NAME_ROIS {
            let t = match tasker.ocr_roi_ex(&img, frac, true) {
                Ok(items) => items
                    .into_iter()
                    .filter(|x| !x.text.trim().is_empty())
                    .map(|x| x.text)
                    .next()
                    .unwrap_or_default(),
                Err(_) => String::new(),
            };
            names.push(t);
        }

        rows.push(MethodRow {
            label: name,
            method,
            ms: median,
            size: format!("{}x{}x{}", img.width, img.height, img.channels),
            mean: img.mean(),
            names,
        });
    }

    // 交叉投票：几何正确的方式会认出**同样**的三张卡名；
    // 位置偏移的方式通常也能凑出三条文字，但内容不同（比如被切掉半个字）。
    let mut votes: Vec<(String, usize)> = Vec::new();
    for r in &rows {
        if r.names.iter().any(|s| s.is_empty()) {
            continue;
        }
        let key = r.names.join("|");
        match votes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, c)) => *c += 1,
            None => votes.push((key, 1)),
        }
    }
    let majority = votes.iter().max_by_key(|(_, c)| *c).map(|(k, _)| k.clone());
    // 只有一种方式认出文字时，投票无意义，退回默认顺序里第一个能认出三张的
    let trusted = match &majority {
        Some(k) if votes.iter().any(|(kk, c)| kk == k && *c >= 2) => Some(k.clone()),
        _ => rows
            .iter()
            .find(|r| !r.names.iter().any(|s| s.is_empty()))
            .map(|r| r.names.join("|")),
    };

    println!(
        "{:<13} {:>9} {:>16} {:>6}  {:<12} 卡名",
        "方式", "截图(ms)", "尺寸", "亮度", "OCR"
    );
    println!("{}", "-".repeat(94));

    let mut best: Option<(u64, f64)> = None;
    for r in &rows {
        let all_found = !r.names.iter().any(|s| s.is_empty());
        let agree = trusted.as_deref() == Some(r.names.join("|").as_str());
        let state = if all_found && agree {
            "通过"
        } else if all_found {
            "位置偏移"
        } else {
            "识别失败"
        };
        println!(
            "{:<13} {:>9.1} {:>16} {:>6.1}  {:<12} {}",
            r.label,
            r.ms,
            r.size,
            r.mean,
            state,
            r.names.join(" | ")
        );
        if all_found && agree && best.map(|(_, t)| r.ms < t).unwrap_or(true) {
            best = Some((r.method, r.ms));
        }
    }

    let Some((method, median)) = best else {
        eprintln!("\n没有一种截图方式能既截到画面、又让三张卡名都识别出来。");
        eprintln!("1) 确认游戏是窗口 / 无边框模式，且没有被其他窗口遮挡");
        eprintln!("2) 停在选卡界面（三张卡）再跑一次");
        eprintln!("3) 如果刚才不在选卡界面，这条结论不成立，重跑即可");
        std::process::exit(4);
    };

    println!("\n可用且最快的截图方式: {}  {:.1}ms", method, median);

    // 用选中的方式再连拍 20 帧，看稳定性
    let ctrl = Controller::create(maa.clone(), win.hwnd, method, maa::input::INTERCEPTION)
        .expect("重建控制器失败");
    ctrl.connect().expect("重连失败");
    ctrl.set_screenshot_long_side(1920);

    let mut times = Vec::new();
    for _ in 0..20 {
        let t0 = Instant::now();
        if ctrl.screencap().is_ok() {
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
    if !times.is_empty() {
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let sum: f64 = times.iter().sum();
        println!(
            "连拍 20 帧: 最快 {:.1}ms  中位 {:.1}ms  平均 {:.1}ms  最慢 {:.1}ms",
            times[0],
            times[times.len() / 2],
            sum / times.len() as f64,
            times[times.len() - 1]
        );
    }

    println!("\nPNG 已存到: {}", shots_dir.display());
    println!("把上面这段发给我即可（PNG 我自己会看，你不用管）。");
}

struct MethodRow {
    label: &'static str,
    method: u64,
    ms: f64,
    size: String,
    mean: f64,
    names: Vec<String>,
}


/// 环境自检：把依赖、驱动、窗口一次查清楚，不加载模型也不按键
fn selftest() {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    println!("========== YBB 环境自检 ==========");
    println!("作者: 歪歪子    邮箱: 1443423104@qq.com");
    println!("程序目录: {cwd}\n");

    let mut ok = 0u32;
    let mut total = 0u32;
    let mut check = |name: &str, good: bool, detail: &str| {
        total += 1;
        if good {
            ok += 1;
        }
        println!(
            "  [{}] {:<18} {}",
            if good { "OK" } else { "--" },
            name,
            detail
        );
    };

    // 文件类
    let maa = find_asset("_internal/maa/bin", "MaaFramework.dll");
    check(
        "Maa 框架",
        maa.is_some(),
        &maa.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "缺失".into()),
    );
    let model = find_asset("model/ocr", "rec.onnx");
    check(
        "OCR 模型",
        model.is_some(),
        if model.is_some() { "rec.onnx 就位" } else { "缺失" },
    );
    check(
        "焦点模板",
        std::path::Path::new("focus_template.png").exists(),
        if std::path::Path::new("focus_template.png").exists() {
            "focus_template.png 就位（判焦点走模板，快）"
        } else {
            "缺失，会退回 OCR（慢但不影响正确性）"
        },
    );
    check(
        "键盘注入库",
        std::path::Path::new("interception.dll").exists(),
        if std::path::Path::new("interception.dll").exists() {
            "interception.dll 就位"
        } else {
            "缺失（按键会退回 SendInput，可能被游戏过滤）"
        },
    );

    // 驱动类
    let kbd = std::path::Path::new(r"C:/Windows/System32/drivers/keyboard.sys").exists();
    check(
        "键盘驱动",
        kbd,
        if kbd {
            "keyboard.sys 已安装"
        } else {
            "未安装 —— 请双击「安装驱动.bat」，装完重启"
        },
    );
    let vigem = std::path::Path::new(r"C:/Windows/System32/drivers/ViGEmBus.sys").exists();
    check(
        "虚拟手柄驱动",
        vigem,
        if vigem {
            "ViGEmBus 已安装（手柄模式可用）"
        } else {
            "未安装（不影响键盘模式）"
        },
    );

    // 游戏窗口
    match win::find(WINDOW_KEY) {
        Some(w) => {
            let (x, y, cw, ch) = win::rect(w.hwnd);
            check(
                "游戏窗口",
                true,
                &format!("{}  位置({x},{y}) 尺寸{cw}x{ch}", w.title),
            );
            if cw <= 0 || ch <= 0 || x <= -30000 {
                println!("       注意：窗口可能被最小化了，先恢复再跑");
            }
        }
        None => check("游戏窗口", false, "没找到（游戏没开，或标题不含 Feed The Cups）"),
    }

    // 实体手柄
    match ybb::xinput::describe() {
        Some(d) => println!("  [--] {:<18} {}（键盘模式不受影响）", "实体手柄", d),
        None => println!("  [--] {:<18} 未检测到", "实体手柄"),
    }

    println!("\n结果: {ok}/{total} 项就绪");
    if !kbd {
        println!("下一步：双击「安装驱动.bat」，UAC 点「是」，装完重启电脑。");
    } else if maa.is_none() || model.is_none() {
        println!("下一步：文件不全，重新解压整个压缩包。");
    } else {
        println!("环境没问题，可以双击 YBB.exe 用了。");
    }
}
