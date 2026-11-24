//! 选卡引擎：GUI 和命令行共用同一套流程。
//!
//! 与 MBB 原版的行为对齐，但把所有固定 `sleep` 换成"变化侦测 + 稳定帧直接读"。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::assets;
use crate::bubble::Bubble;
use crate::consts::{MATCH_THRESHOLD, WINDOW_KEY};
use crate::input::{self, Injector};
use crate::maa::{self, Controller, Image, Maa};
use crate::ocr::{Resource, Tasker};
use crate::picker;
use crate::score::score;
use crate::tuning::Tuning;
use crate::win;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Step,
    Ok,
    Warn,
    Err,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    WaitingScreen,
    Reading,
    MovingFocus,
    Confirming,
    WaitingNext,
    Rerolling,
    Done,
}

pub enum Msg {
    Log(Level, String),
    /// 当前轮次 / 总轮数 / 目标卡
    Progress(usize, usize, String),
    /// 当前阶段，用于 GUI 状态条
    Phase(Phase),
    Done(Result<(), String>),
}

pub struct Config {
    pub queue: Vec<String>,
    /// 从队列的第几轮开始（0 起）。出错后「继续」用它从当前轮接着跑。
    pub start_index: usize,
    /// false = 空跑，只识别不按键
    pub go: bool,
    pub max_reroll: u32,
    pub use_interception: bool,
    /// 键盘 还是 虚拟手柄
    pub input: input::InputMode,
}

/// 跑完整个队列。`emit` 会在工作线程里被调用，GUI 用它更新界面。
pub fn run(cfg: Config, emit: impl Fn(Msg) + Send + 'static, stop: Arc<AtomicBool>) {
    let result = run_inner(&cfg, &emit, &stop);
    emit(Msg::Done(result));
}

fn run_inner(
    cfg: &Config,
    emit: &impl Fn(Msg),
    stop: &Arc<AtomicBool>,
) -> Result<(), String> {
    let log = |lv: Level, s: String| emit(Msg::Log(lv, s));
    let stopped = || stop.load(Ordering::SeqCst);
    let tune = Tuning::from_env();

    let maa_dir = assets::maa_bin()?;
    let model_dir = assets::ocr_model()?;

    log(Level::Info, "加载 OCR 模型 ...".into());
    let maa = Arc::new(Maa::load(&maa_dir).map_err(|e| e.to_string())?);
    let res = Resource::new(maa.clone()).map_err(|e| e.to_string())?;
    res.post_ocr_model(&model_dir).map_err(|e| e.to_string())?;
    let tasker = Tasker::new(maa.clone()).map_err(|e| e.to_string())?;
    tasker.bind_resource(&res).map_err(|e| e.to_string())?;

    let win = win::find(WINDOW_KEY).ok_or("没找到游戏窗口（标题含 Feed The Cups）")?;
    let ctrl = Controller::create(
        maa.clone(),
        win.hwnd,
        maa::screencap::DXGI_DESKTOP_DUP_WINDOW,
        maa::input::INTERCEPTION,
    )
    .map_err(|e| e.to_string())?;
    ctrl.connect().map_err(|e| e.to_string())?;
    ctrl.set_screenshot_long_side(tune.screenshot_long_side as i32);

    let tpl = Bubble::load("focus_template.png");
    if tpl.is_none() {
        log(Level::Warn, "没找到 focus_template.png，焦点判定退回 OCR（会慢一些）".into());
    }

    // 实体手柄会改变游戏的输入模式，也决定虚拟手柄能不能拿到 1P，先探一下
    match crate::xinput::describe() {
        None => log(Level::Info, "未检测到实体手柄".into()),
        Some(d) => log(
            Level::Info,
            format!("检测到实体手柄: {d}（杯杯倒满只认 1P）"),
        ),
    }

    // 决定实际用哪种输入。用户选「手柄」时，如果条件不满足就自动回退到键盘，
    // 而不是硬报错让程序停在那——手柄方案在多数机器上本来就跑不起来。
    let mut mode = cfg.input;
    if mode == input::InputMode::Gamepad {
        let mut reason: Option<String> = None;
        // 1. 实体手柄占着 1P → 虚拟手柄只能拿到 2P+，游戏只认 1P，白搭
        if let Some(p) = crate::xinput::scan().into_iter().find(|p| p.slot == 0) {
            reason = Some(format!(
                "实体手柄正占着 1P{}，虚拟手柄会被排到 2P 以后，游戏只认 1P",
                if p.active { "（而且它正在发输入）" } else { "" }
            ));
        }
        // 2. 没有管理员权限 → ViGEmBus 打不开。先试试能不能连上，连不上就回退。
        if reason.is_none() {
            if let Err(e) = input::try_gamepad(maa.clone(), win.hwnd, 60) {
                reason = Some(e);
            }
        }
        if let Some(r) = reason {
            log(
                Level::Warn,
                format!("手柄模式不可用（{r}），已自动改用键盘"),
            );
            mode = input::InputMode::Keyboard;
        }
    }
    if mode == input::InputMode::Keyboard {
        if let Some(p) = crate::xinput::scan().into_iter().find(|p| p.active) {
            log(
                Level::Warn,
                format!(
                    "{}P 上有手柄正在发输入，游戏可能锁在手柄模式导致键盘无效；\
                     按不动就关掉手柄再试",
                    p.slot + 1
                ),
            );
        }
    }

    if cfg.go {
        win::force_foreground(win.hwnd);
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    let inj: Option<Box<dyn Injector>> = if cfg.go {
        let i = input::build_mode(
            mode,
            maa.clone(),
            win.hwnd,
            cfg.use_interception,
            tune.hold_ms,
        )?;
        log(Level::Info, format!("输入方式: {}", i.name()));
        Some(i)
    } else {
        None
    };

    // 卡名完整但焦点气泡被介绍面板挡住时，按 S 两次清掉介绍。
    let clear_cards = || {
        if let Some(inj) = &inj {
            log(Level::Warn, "  焦点气泡被挡，按 S 两次清介绍".into());
            let _ = inj.tap(input::Key::Down);
            std::thread::sleep(std::time::Duration::from_millis(45));
            let _ = inj.tap(input::Key::Down);
            std::thread::sleep(std::time::Duration::from_millis(160));
        }
    };

    // ORT 批量识别；失败/手动关闭时回退 MAA
    let use_maa = std::env::var("YBB_OCR")
        .map(|v| v.eq_ignore_ascii_case("maa"))
        .unwrap_or(false);
    let ort = if use_maa {
        log(Level::Info, "OCR: MAA（YBB_OCR=maa）".into());
        None
    } else {
        match crate::ort_ocr::OrtOcr::from_assets(tune.ort_threads) {
            Ok(o) => {
                log(Level::Info, "OCR: ONNX Runtime 三张批量（4 线程）".into());
                Some(o)
            }
            Err(e) => {
                log(Level::Warn, format!("ORT 不可用，回退 MAA：{e}"));
                None
            }
        }
    };

    // 预热一次，避免首轮把模型初始化的时间算进去
    let _ = picker::settle_and_read(&ctrl, &tasker, tpl.as_ref(), ort.as_ref(), Some(&clear_cards), tune.settle_timeout_ms);

    let total = cfg.queue.len();
    if cfg.start_index >= total {
        return Err("队列里没有可继续的轮次".into());
    }
    let frame_reuse = tune.frame_reuse;
    let batch_taps = tune.batch_taps;
    let mut seed: Option<Image> = None;
    let mut prev_names: Vec<String> = Vec::new();
    let t_all = Instant::now();

    for (i, target) in cfg.queue.iter().enumerate().skip(cfg.start_index) {
        if stopped() {
            return Err("已停止".into());
        }
        emit(Msg::Progress(i + 1, total, target.clone()));
        emit(Msg::Phase(Phase::WaitingScreen));

        let t_round = Instant::now();
        let mut t_look = 0.0f64;
        let mut t_move = 0.0f64;
        let mut rerolls = 0u32;

        let mut pending: Option<picker::Look> = None;
        let picked = loop {
            if stopped() {
                return Err("已停止".into());
            }
            let look = match pending.take() {
                Some(l) => {
                    // 重抽后已经重新识别过，直接复用，别再读一遍
                    t_look += l.settle_ms;
                    l
                }
                None => {
                    let t_l = Instant::now();
                    let mut fails = 0u32;
                    let mut pressed_s = false;
                    let l = loop {
                        let attempt = picker::settle_and_read_seeded(
                            &ctrl,
                            &tasker,
                            tpl.as_ref(),
                            ort.as_ref(),
                            Some(&clear_cards),
                            tune.settle_timeout_ms,
                            seed.take(),
                        );
                        if let Some(l) = attempt {
                            if picker::cards_ok(&l.cards) {
                                // 卡名和上一轮完全一样 = 可能是翻牌动画旧帧，丢弃重读
                                let cur: Vec<&str> = l.cards.iter().map(|c| c.name.as_str()).collect();
                                let prev: Vec<&str> = prev_names.iter().map(|s| s.as_str()).collect();
                                if cur != prev {
                                    break l;
                                }
                                log(Level::Warn, "  卡名和上一轮相同，重新截图识别".into());
                                continue;
                            }
                        }
                        fails += 1;
                        if !pressed_s && fails >= 2 {
                            if let Some(inj) = &inj {
                                log(
                                    Level::Warn,
                                    "  卡名不完整，重试后按 S 两次再读".into(),
                                );
                                inj.tap(input::Key::Down).map_err(|e| e.to_string())?;
                                std::thread::sleep(std::time::Duration::from_millis(45));
                                inj.tap(input::Key::Down).map_err(|e| e.to_string())?;
                                std::thread::sleep(std::time::Duration::from_millis(160));
                                pressed_s = true;
                                fails = 0;
                                continue;
                            }
                        }
                        let limit = if pressed_s { 2 } else { 3 };
                        if fails >= limit {
                            return Err("卡名识别异常，重试后仍读不到三张卡".into());
                        }
                    };
                    t_look += t_l.elapsed().as_secs_f64() * 1000.0;
                    emit(Msg::Phase(Phase::Reading));
                    l
                }
            };

            let shown = look
                .cards
                .iter()
                .map(|c| format!("[{}]", c.name))
                .collect::<Vec<_>>()
                .join(" ");
            if look.auto_layout {
                log(Level::Info, "  使用自动布局识别（特殊界面）".into());
            }

            let mut best: Option<(usize, f64)> = None;
            for (k, c) in look.cards.iter().enumerate() {
                if c.is_empty() {
                    continue;
                }
                let s = score(target, &c.name);
                if best.map(|(_, bs)| s > bs).unwrap_or(true) {
                    best = Some((k, s));
                }
            }

            match best {
                Some((idx, s)) if s >= MATCH_THRESHOLD => {
                    log(
                        Level::Step,
                        format!("  {shown} → 第{}张 分数 {:.2}（重抽 {} 次）", idx + 1, s, rerolls),
                    );
                    // 读不到焦点就停下：绝不能假设"焦点在第1张"，
                    // 那会按错格数、把卡选错。宁可报错让人看一眼。
                    let Some(f) = look.focus else {
                        return Err(
                            "读不到焦点：卡上既没有「选择」气泡，也没认出焦点位置。\
                             游戏可能在键盘 UI 模式下不显示气泡 —— 用手柄按一下方向键让它切回手柄 UI，或告诉我加边框识别"
                                .into(),
                        );
                    };
                    break (idx, f, look.cards.clone(), look.frame, look.auto_layout);
                }
                _ => {
                    log(Level::Step, format!("  当前没有 {target:?}  {shown}"));
                    // 特殊界面（如恶魔能力）：队列里没有同名目标时，
                    // 不按重抽（可能花杯币），暂停等用户手选，画面变了自动继续。
                    if look.auto_layout {
                        log(
                            Level::Warn,
                            "  检测到特殊界面，队列里没有同名目标；请在游戏里手动选择，选完自动继续"
                                .into(),
                        );
                        let baseline = look.frame;
                        if !picker::wait_screen_change(&ctrl, &baseline, 3.0, 300_000) {
                            return Err("等待手动选择超时（5 分钟）".into());
                        }
                        continue;
                    }
                    if rerolls >= cfg.max_reroll {
                        return Err(format!("「{target}」重抽 {} 次仍未出现，停止", rerolls));
                    }
                    let Some(inj) = &inj else {
                        log(Level::Warn, "  （空跑：本应重抽）".into());
                        break (0, 0, look.cards.clone(), look.frame, look.auto_layout);
                    };
                    let t_rr = Instant::now();
                    emit(Msg::Phase(Phase::Rerolling));
                    inj.tap(input::Key::Reroll).map_err(|e| e.to_string())?;
                    // 重抽后直接进入「稳定 + 必须不同于重抽前」的识别，
                    // 不再单独 wait_changed：动画没开始时会一直等，不会把旧画面当新画面。
                    let baseline = look.frame;
                    match picker::settle_after_change(
                        &ctrl,
                        &tasker,
                        tpl.as_ref(),
                        ort.as_ref(),
                        Some(&clear_cards),
                        &baseline,
                        tune.reroll_timeout_ms,
                    ) {
                        Ok(l) => {
                            let same = l
                                .cards
                                .iter()
                                .map(|c| &c.name)
                                .eq(look.cards.iter().map(|c| &c.name));
                            if same {
                                return Err(
                                    "重抽后卡名没有变化，可能杯币不足或重抽没生效".into()
                                );
                            }
                            log(
                                Level::Info,
                                format!(
                                    "  重抽生效，重新识别用了 {:.0}ms",
                                    t_rr.elapsed().as_secs_f64() * 1000.0
                                ),
                            );
                            rerolls += 1;
                            pending = Some(l);
                        }
                        Err(picker::SettleFail::NoChange) => {
                            return Err("重抽后画面没变化（可能杯币不足）".into());
                        }
                        Err(picker::SettleFail::Timeout) => {
                            return Err("重抽后等不到稳定画面".into());
                        }
                    }
                }
            }
        };

        let (target_idx, mut focus, snapshot, frame, auto_layout) = picked;

        if let Some(inj) = &inj {
            emit(Msg::Phase(Phase::MovingFocus));
            let mut guard = 0;
            while focus != target_idx && guard < 5 {
                guard += 1;
                let key = if target_idx > focus {
                    input::Key::Right
                } else {
                    input::Key::Left
                };
                if !win::is_foreground(win.hwnd) {
                    win::force_foreground(win.hwnd);
                    std::thread::sleep(std::time::Duration::from_millis(120));
                }
                let prev = focus;
                let dist = target_idx.abs_diff(focus);
                if batch_taps && dist > 1 {
                    for _ in 0..dist {
                        inj.tap(key).map_err(|e| e.to_string())?;
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    let t_bt = Instant::now();
                    if let Some(f) = picker::wait_focus_target(
                        &ctrl,
                        &tasker,
                        &snapshot,
                        tpl.as_ref(),
                        target_idx,
                        tune.focus_timeout_ms,
                    ) {
                        focus = f;
                        t_move += t_bt.elapsed().as_secs_f64() * 1000.0;
                        log(Level::Info, format!("  连按 {dist} 格到第{}张，用了 {:.0}ms", focus + 1, t_bt.elapsed().as_secs_f64() * 1000.0));
                        continue;
                    }
                    log(Level::Warn, "  连按没到目标，改回逐格".into());
                    if let Ok(img) = ctrl.screencap() {
                        if let Some(f) = picker::focus_auto(&tasker, &img, &snapshot, tpl.as_ref()) {
                            focus = f;
                        }
                    }
                }
                let t_mv = Instant::now();
                inj.tap(key).map_err(|e| e.to_string())?;
                let next = if auto_layout {
                    picker::wait_focus_change_global(
                        &ctrl,
                        &tasker,
                        &snapshot,
                        tpl.as_ref(),
                        prev,
                        tune.focus_timeout_ms,
                    )
                } else {
                    picker::wait_focus_change(
                        &ctrl,
                        &tasker,
                        &snapshot,
                        tpl.as_ref(),
                        prev,
                        tune.focus_timeout_ms,
                    )
                };
                match next {
                    Some(f) => {
                        focus = f;
                        t_move += t_mv.elapsed().as_secs_f64() * 1000.0;
                        log(
                            Level::Info,
                            format!("  焦点到第{}张，用了 {:.0}ms", focus + 1, t_mv.elapsed().as_secs_f64() * 1000.0),
                        );
                    }
                    None => {
                        log(Level::Warn, "  焦点没动，再试一次".into());
                        if guard >= 5 {
                            return Err(format!("焦点没能移到第{}张", target_idx + 1));
                        }
                    }
                }
            }
            if focus != target_idx {
                return Err(format!("焦点没能移到第{}张", target_idx + 1));
            }
            if !win::is_foreground(win.hwnd) {
                win::force_foreground(win.hwnd);
            }
            emit(Msg::Phase(Phase::Confirming));
            inj.tap(input::Key::Confirm).map_err(|e| e.to_string())?;
        }

        if i + 1 < total {
            emit(Msg::Phase(Phase::WaitingNext));
            // band 只取决于截图尺寸；直接用当前这帧算，别再为它截一次图。
            let band = crate::fingerprint::name_band(&picker::name_rois_abs(&frame));
            let mut changed = picker::wait_changed(&ctrl, band, &frame, 1.0, tune.settle_timeout_ms);
            if changed.is_none() {
                if let Some(inj) = &inj {
                    log(Level::Warn, "  确认后画面没变，重试一次确认".into());
                    inj.tap(input::Key::Confirm).map_err(|e| e.to_string())?;
                    changed = picker::wait_changed(&ctrl, band, &frame, 1.0, tune.focus_timeout_ms.max(1000));
                }
            }
            if changed.is_none() && inj.is_some() {
                return Err("确认后画面没有变化，可能没选中或游戏卡住".into());
            }
            let changed_ok = changed.is_some();
            if frame_reuse {
                // 把变化帧留给下一轮判稳，省一次截图
                seed = changed;
            } else {
                seed = None;
            }
            prev_names = snapshot.iter().map(|c| c.name.clone()).collect();
            log(
                Level::Ok,
                format!(
                    "  {} 本轮 {:.2}s（看 {:.0}ms / 推 {:.0}ms）",
                    if changed_ok { "已选中" } else { "画面未变" },
                    t_round.elapsed().as_secs_f64(),
                    t_look,
                    t_move
                ),
            );
        } else {
            log(
                Level::Ok,
                format!(
                    "  最后一张，收工  本轮 {:.2}s（看 {:.0}ms / 推 {:.0}ms）",
                    t_round.elapsed().as_secs_f64(),
                    t_look,
                    t_move
                ),
            );
        }
    }

    emit(Msg::Phase(Phase::Done));
    log(
        Level::Ok,
        format!("全部 {total} 轮完成，总耗时 {:.1}s", t_all.elapsed().as_secs_f64()),
    );
    Ok(())
}
