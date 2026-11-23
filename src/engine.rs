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
use crate::maa::{self, Controller, Maa};
use crate::ocr::{Resource, Tasker};
use crate::picker;
use crate::score::score;
use crate::win;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Step,
    Ok,
    Warn,
    Err,
}

pub enum Msg {
    Log(Level, String),
    /// 当前轮次 / 总轮数 / 目标卡
    Progress(usize, usize, String),
    Done(Result<(), String>),
}

pub struct Config {
    pub queue: Vec<String>,
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
    ctrl.set_screenshot_long_side(1920);

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
            60,
        )?;
        log(Level::Info, format!("输入方式: {}", i.name()));
        Some(i)
    } else {
        None
    };

    // 预热一次，避免首轮把模型初始化的时间算进去
    let _ = picker::settle_and_read(&ctrl, &tasker, tpl.as_ref(), 3000);

    let total = cfg.queue.len();
    let t_all = Instant::now();

    for (i, target) in cfg.queue.iter().enumerate() {
        if stopped() {
            return Err("已停止".into());
        }
        emit(Msg::Progress(i + 1, total, target.clone()));
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
                    let Some(l) = picker::settle_and_read(&ctrl, &tasker, tpl.as_ref(), 3000)
                    else {
                        return Err("等不到稳定的选卡画面".into());
                    };
                    t_look += t_l.elapsed().as_secs_f64() * 1000.0;
                    l
                }
            };

            let shown = look
                .cards
                .iter()
                .map(|c| format!("[{}]", c.name))
                .collect::<Vec<_>>()
                .join(" ");

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
                    break (idx, f, look.cards.clone());
                }
                _ => {
                    log(Level::Step, format!("  当前没有 {target:?}  {shown}"));
                    if rerolls >= cfg.max_reroll {
                        return Err(format!("「{target}」重抽 {} 次仍未出现，停止", rerolls));
                    }
                    let Some(inj) = &inj else {
                        log(Level::Warn, "  （空跑：本应重抽）".into());
                        break (0, 0, look.cards.clone());
                    };
                    let t_rr = Instant::now();
                    inj.tap(input::Key::Reroll).map_err(|e| e.to_string())?;
                    // 重抽后直接进入「稳定 + 必须不同于重抽前」的识别，
                    // 不再单独 wait_changed：动画没开始时会一直等，不会把旧画面当新画面。
                    let baseline = look.frame;
                    match picker::settle_after_change(
                        &ctrl,
                        &tasker,
                        tpl.as_ref(),
                        &baseline,
                        3500,
                    ) {
                        Ok(l) => {
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

        let (target_idx, mut focus, snapshot) = picked;

        if let Some(inj) = &inj {
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
                let t_mv = Instant::now();
                inj.tap(key).map_err(|e| e.to_string())?;
                match picker::wait_focus_change(
                    &ctrl,
                    &tasker,
                    &snapshot,
                    tpl.as_ref(),
                    prev,
                    1500,
                ) {
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
            inj.tap(input::Key::Confirm).map_err(|e| e.to_string())?;
        }

        if i + 1 < total {
            let band = crate::fingerprint::name_band(&picker::name_rois_abs(
                &ctrl.screencap().map_err(|e| e.to_string())?,
            ));
            let changed = picker::wait_changed(&ctrl, band, 1.0, 3500);
            log(
                Level::Ok,
                format!(
                    "  {} 本轮 {:.2}s（看 {:.0}ms / 推 {:.0}ms）",
                    if changed { "已选中" } else { "画面未变" },
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

    log(
        Level::Ok,
        format!("全部 {total} 轮完成，总耗时 {:.1}s", t_all.elapsed().as_secs_f64()),
    );
    Ok(())
}
