//! 卡名读取 + 焦点判定。
//!
//! 与原版 `Picker.read_cards` / `Picker.focus_index` 等价：
//! - 卡名：三个固定 ROI 各 OCR 一次，取**文字最长**的那条
//! - 焦点：在卡名下方的一条窄带里找 `选择` 气泡，取**最靠上**的那个，
//!   再和三个卡名的横向中心比距离，最近的即焦点所在。

use crate::consts::NAME_ROIS;
use crate::maa::Image;
use crate::ocr::{OcrItem, Tasker};

/// 焦点气泡所在的窄带（相对卡名垂直中心的偏移）
const BAND_TOP: f64 = 0.33;
const BAND_BOTTOM: f64 = 0.42;

/// 一个卡位
#[derive(Debug, Clone)]
pub struct CardSlot {
    /// 归一化后的卡名（用于匹配队列）
    pub name: String,
    /// OCR 原文
    pub raw: String,
    /// 卡名文字框 [x, y, w, h]，绝对像素
    pub box_: Option<[i32; 4]>,
}

impl CardSlot {
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
    }
}

/// 读三个卡位
pub fn read_cards(t: &Tasker, img: &Image, only_rec: bool) -> Vec<CardSlot> {
    let mut out = Vec::with_capacity(3);
    for frac in NAME_ROIS {
        let rs = t.ocr_roi_ex(img, frac, only_rec).unwrap_or_default();
        // 原版用 max(key=len(text))：取文字最长的那条
        match rs.into_iter().max_by_key(|x| x.text.chars().count()) {
            Some(r) => out.push(CardSlot {
                name: crate::score::norm(&r.text),
                raw: r.text,
                box_: Some(r.box_),
            }),
            None => out.push(CardSlot {
                name: String::new(),
                raw: String::new(),
                box_: None,
            }),
        }
    }
    out
}

/// 自动布局时判断一段文字像不像卡名：2–8 个字、至少两个汉字、排除标题/按钮/描述。
fn auto_name_ok(s: &str) -> bool {
    let t = crate::score::norm(s);
    let n = t.chars().count();
    if !(2..=8).contains(&n) {
        return false;
    }
    if t.chars().filter(|c| is_cjk(*c)).count() < 2 {
        return false;
    }
    const BAD: [&str; 10] = [
        "选择", "恶魔", "重抽", "放弃", "余额", "礼物", "运营", "店铺", "上限", "概率",
    ];
    !BAD.iter().any(|b| t.contains(b))
}

/// 自动布局兜底：不依赖固定 ROI，在整块卡区做一次带检测的 OCR，
/// 按左/中/右三列各取最靠上的「像卡名」的文字。
pub fn read_cards_auto(t: &Tasker, img: &Image) -> Vec<CardSlot> {
    let hits = t
        .ocr_roi_ex(img, (0.02, 0.28, 0.98, 0.82), false)
        .unwrap_or_default();
    let w = img.width as f64;
    let h = img.height as f64;
    let cols = [(0.06, 0.42), (0.42, 0.70), (0.70, 0.99)];

    let mut out = Vec::with_capacity(3);
    for (lo, hi) in cols {
        let mut best: Option<&OcrItem> = None;
        for it in &hits {
            let cx = (it.box_[0] as f64 + it.box_[2] as f64 / 2.0) / w;
            let cy = (it.box_[1] as f64 + it.box_[3] as f64 / 2.0) / h;
            if cx < lo || cx >= hi || cy < 0.28 || cy > 0.82 {
                continue;
            }
            if !auto_name_ok(&it.text) {
                continue;
            }
            if best.map(|b| it.box_[1] < b.box_[1]).unwrap_or(true) {
                best = Some(it);
            }
        }
        match best {
            Some(b) => out.push(CardSlot {
                name: crate::score::norm(&b.text),
                raw: b.text.clone(),
                box_: Some(b.box_),
            }),
            None => out.push(CardSlot {
                name: String::new(),
                raw: String::new(),
                box_: None,
            }),
        }
    }
    out
}
pub fn focus_index(
    t: &Tasker,
    img: &Image,
    cards: &[CardSlot],
    _only_rec: bool,
) -> (Option<usize>, Vec<OcrItem>) {
    let w = img.width as f64;
    let h = img.height as f64;

    let Some(b0) = cards.first().and_then(|c| c.box_) else {
        return (None, Vec::new());
    };

    // 用第一个卡名框的垂直中心，往下推出一条窄带。
    // 原版取的是 [ny+0.30, ny+0.55]（占整幅高度 25%），实测这一条要 735ms；
    // 气泡实际落在 ny+0.35 附近，收窄到 [ny+0.33, ny+0.42] 后区域面积只剩三分之一。
    let ny = (b0[1] as f64 + b0[3] as f64 / 2.0) / h;
    let y0 = (ny + BAND_TOP).max(0.0);
    let y1 = (ny + BAND_BOTTOM).min(1.0);

    // 注意：这里**必须**用完整检测（only_rec = false）。
    // only_rec 会把整个 ROI 当成一行文字送给识别模型，而这条带里散落着多段文字
    // （气泡、杯币余额、重抽提示…），跳检测只会得到一条空结果。
    let mut hits = match t.ocr_roi_ex(img, (0.05, y0, 0.95, y1), false) {
        Ok(h) => h,
        Err(_) => return (None, Vec::new()),
    };
    let all_hits = hits.clone();
    if hits.is_empty() {
        return (None, all_hits);
    }

    // 下方还有"放弃"按钮，丢掉它以及更靠下的所有命中
    if let Some(ab) = hits.iter().find(|r| r.text.contains("放弃")) {
        let ay = (ab.box_[1] as f64 + ab.box_[3] as f64 / 2.0) / h;
        hits.retain(|r| (r.box_[1] as f64 + r.box_[3] as f64 / 2.0) / h < ay);
    }

    // 只要含"选择"的，取最靠上的那个（原版 key=box[1]）
    let mut choose: Vec<&OcrItem> = hits.iter().filter(|r| r.text.contains("选择")).collect();
    if choose.is_empty() {
        return (None, all_hits);
    }
    choose.sort_by_key(|r| r.box_[1]);
    let bubble = choose[0];

    // 与三个卡名的横向中心比距离。
    // 注意单位：两边都要用**归一化**坐标（原版这里是 float 除法，不要拿绝对像素去比）。
    let centers: Vec<f64> = cards
        .iter()
        .map(|c| match c.box_ {
            Some(b) => (b[0] as f64 + b[2] as f64 / 2.0) / w,
            None => -999.0,
        })
        .collect();
    let bubble_cx = (bubble.box_[0] as f64 + bubble.box_[2] as f64 / 2.0) / w;

    let mut best = 0usize;
    let mut best_d = f64::MAX;
    for (i, c) in centers.iter().enumerate() {
        let d = (bubble_cx - c).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    (Some(best), all_hits)
}

/// 判稳阈值：静止画面实测差值 0.000，留一点余量给抗锯齿/编码噪声
pub const STABLE_DIFF: f64 = 0.5;
/// 最短稳定时间：快机器上帧间隔可能只有十几毫秒，必须连续不变够久才算稳定
pub const STABLE_MS: u64 = 50;
/// 重抽后判定「画面真的变了」的阈值
pub const CHANGE_DIFF: f64 = 0.5;

/// 一次「看」的结果
pub struct Look {
    pub cards: Vec<CardSlot>,
    pub focus: Option<usize>,
    /// 从开始等到判定稳定花了多久
    pub settle_ms: f64,
    /// 一共截了几帧
    pub shots: u32,
    /// 判稳时的那一帧，重抽后用来判断画面有没有真的刷新
    pub frame: Image,
    /// 是否是自动布局兜底识别出来的（特殊界面）
    pub auto_layout: bool,
}

/// 重抽后等画面失败的原因
#[derive(Debug, Clone, Copy)]
pub enum SettleFail {
    /// 画面一直没变（多半是杯币不足）
    NoChange,
    /// 画面变了，但一直没稳定下来
    Timeout,
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

/// 翻牌动画中间会闪出「一E / 一a / 田」这种占位乱码，别把它当结果。
/// 正常卡名至少有两个汉字；允许三张里有一张 OCR 抽风（比如把「气球」读成「球」）。
fn plausible(cards: &[CardSlot]) -> bool {
    cards
        .iter()
        .filter(|c| c.name.chars().filter(|ch| is_cjk(*ch)).count() >= 2)
        .count()
        >= 2
}

/// 判稳 + 读卡 + 判焦点。
///
/// `baseline` 有值时表示这是「重抽之后」：除了要求连续两帧稳定，
/// 还要求这一帧和重抽前明显不同，避免动画还没开始就把旧画面当成新画面。
fn settle_core(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    tpl: Option<&crate::bubble::Bubble>,
    timeout_ms: u64,
    baseline: Option<&Image>,
    seed: Option<Image>,
    stable_needed: u32,
) -> Result<Look, SettleFail> {
    let start = std::time::Instant::now();
    let seeded = seed.is_some();
    let mut prev: Option<Image> = seed;
    let mut prev_t: Option<std::time::Instant> = if seeded { Some(start) } else { None };
    let mut stable_since: Option<std::time::Instant> = None;
    let mut shots = 0u32;
    let mut saw_change = false;
    let mut stable_run = 0u32;
    // 第一次识别成功就直接过，不等 50ms；只有失败重试才走最短稳定时间
    let mut first_try = true;
    // 重抽和「复用上一帧」都可能碰到动画中间帧，卡名要像正常卡名才接受
    let require_plausible = baseline.is_some() || seeded;

    loop {
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return Err(if baseline.is_some() && !saw_change {
                SettleFail::NoChange
            } else {
                SettleFail::Timeout
            });
        }
        let Ok(img) = ctrl.screencap() else {
            std::thread::sleep(std::time::Duration::from_millis(8));
            continue;
        };
        shots += 1;
        let now = std::time::Instant::now();

        let rois = name_rois_abs(&img);
        let band = crate::fingerprint::name_band(&rois);

        // 和重抽前那一帧比：必须真的变了
        let differs = match baseline {
            None => true,
            Some(b) => {
                let d = crate::fingerprint::band_diff(b, &img, band);
                if d > CHANGE_DIFF {
                    saw_change = true;
                }
                d > CHANGE_DIFF
            }
        };

        let stable = match &prev {
            None => false,
            Some(p) => crate::fingerprint::band_diff(p, &img, band) < STABLE_DIFF,
        };
        if stable {
            if stable_since.is_none() {
                stable_since = prev_t.or(Some(now));
            }
            stable_run += 1;
        } else {
            stable_run = 0;
            stable_since = None;
        }
        let stable_long_enough = stable_since
            .map(|t| t.elapsed().as_millis() as u64 >= STABLE_MS)
            .unwrap_or(false);

        let may_read = stable_run >= stable_needed
            && differs
            && (first_try || stable_long_enough);

        if may_read {
            first_try = false;
            let cards = read_cards(t, &img, true);
            let normal_ok = cards.iter().all(|c| !c.is_empty()) && plausible(&cards);
            let normal_focus = if normal_ok {
                focus_auto(t, &img, &cards, tpl)
            } else {
                None
            };
            if normal_ok && normal_focus.is_some() {
                return Ok(Look {
                    cards,
                    focus: normal_focus,
                    settle_ms: start.elapsed().as_secs_f64() * 1000.0,
                    shots,
                    frame: img,
                    auto_layout: false,
                });
            }

            // 固定 ROI 读不到卡名或焦点：自动布局兜底（恶魔能力等特殊界面）
            let auto_cards = read_cards_auto(t, &img);
            if auto_cards.iter().all(|c| !c.is_empty()) && plausible(&auto_cards) {
                let focus = focus_auto_global(t, &img, &auto_cards, tpl);
                return Ok(Look {
                    cards: auto_cards,
                    focus,
                    settle_ms: start.elapsed().as_secs_f64() * 1000.0,
                    shots,
                    frame: img,
                    auto_layout: true,
                });
            }

            // 普通卡名读到了、但焦点没有：保持原行为，让引擎报明确错误
            if normal_ok {
                return Ok(Look {
                    cards,
                    focus: None,
                    settle_ms: start.elapsed().as_secs_f64() * 1000.0,
                    shots,
                    frame: img,
                    auto_layout: false,
                });
            }

            // 动画中间的占位帧：重新攒稳定窗口，别急着读
            if require_plausible {
                stable_run = 0;
                stable_since = Some(now);
            }
        }

        prev = Some(img);
        prev_t = Some(now);
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
}

/// 普通的一次「看」：只要求画面稳定。
pub fn settle_and_read(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    tpl: Option<&crate::bubble::Bubble>,
    timeout_ms: u64,
) -> Option<Look> {
    settle_core(ctrl, t, tpl, timeout_ms, None, None, 1).ok()
}

/// 带「上一帧」的一次「看」：`seed` 是确认后 wait_changed 返回的变化帧，
/// 直接拿它当第一帧比较，省掉一次截图。有 seed 时多要一次稳定比较，防止动画中间误判。
pub fn settle_and_read_seeded(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    tpl: Option<&crate::bubble::Bubble>,
    timeout_ms: u64,
    seed: Option<Image>,
) -> Option<Look> {
    let need = 1;
    settle_core(ctrl, t, tpl, timeout_ms, None, seed, need).ok()
}

/// 重抽之后的一次「看」：稳定 **且** 必须不同于 `baseline`。
/// 稳定窗口取 3 次比较，避开翻牌动画中间的假稳定帧。
pub fn settle_after_change(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    tpl: Option<&crate::bubble::Bubble>,
    baseline: &Image,
    timeout_ms: u64,
) -> Result<Look, SettleFail> {
    settle_core(ctrl, t, tpl, timeout_ms, Some(baseline), None, 3)
}

/// 三个卡名 ROI 换成绝对像素
pub fn name_rois_abs(img: &Image) -> [[i32; 4]; 3] {
    let w = img.width as f64;
    let h = img.height as f64;
    let mut out = [[0i32; 4]; 3];
    for (i, f) in NAME_ROIS.iter().enumerate() {
        let x = (f.0 * w) as i32;
        let y = (f.1 * h) as i32;
        out[i] = [x, y, (f.2 * w) as i32 - x, (f.3 * h) as i32 - y];
    }
    out
}

/// 等画面变化：8ms 轮询卡名带差，超过阈值就返回变化后的那一帧。
/// 返回帧可以喂给下一轮 `settle_and_read_seeded`，省掉一次截图。
pub fn wait_changed(
    ctrl: &crate::maa::Controller,
    band: [i32; 4],
    threshold: f64,
    timeout_ms: u64,
) -> Option<Image> {
    let start = std::time::Instant::now();
    let mut prev = match ctrl.screencap() {
        Ok(i) => i,
        Err(_) => return None,
    };
    loop {
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
        let Ok(img) = ctrl.screencap() else {
            continue;
        };
        if crate::fingerprint::band_diff(&prev, &img, band) > threshold {
            return Some(img);
        }
        prev = img;
    }
}

/// 等整屏明显变化，用于「特殊界面暂停、等用户手选」。
/// 每 300ms 看一次，不抢 CPU；默认给 5 分钟。
pub fn wait_screen_change(
    ctrl: &crate::maa::Controller,
    baseline: &Image,
    threshold: f64,
    timeout_ms: u64,
) -> bool {
    let start = std::time::Instant::now();
    let w = baseline.width as f64;
    let h = baseline.height as f64;
    let roi = [
        (0.05 * w) as i32,
        (0.25 * h) as i32,
        (0.90 * w) as i32,
        (0.65 * h) as i32,
    ];
    loop {
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        let Ok(img) = ctrl.screencap() else {
            continue;
        };
        if crate::fingerprint::band_diff(baseline, &img, roi) > threshold {
            return true;
        }
    }
}

/// 只重判焦点（卡名不变时用）。省掉三次卡名 OCR。
pub fn settle_and_focus(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    tpl: Option<&crate::bubble::Bubble>,
    cards: &[CardSlot],
    timeout_ms: u64,
) -> Option<usize> {
    let start = std::time::Instant::now();
    let mut prev: Option<Image> = None;
    loop {
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return None;
        }
        let Ok(img) = ctrl.screencap() else {
            std::thread::sleep(std::time::Duration::from_millis(8));
            continue;
        };
        let rois = name_rois_abs(&img);
        let band = crate::fingerprint::name_band(&rois);
        if let Some(p) = &prev {
            if crate::fingerprint::band_diff(p, &img, band) < STABLE_DIFF {
                let f = focus_auto(t, &img, cards, tpl);
                if f.is_some() {
                    return f;
                }
            }
        }
        prev = Some(img);
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
}

/// 用模板匹配找焦点（快路径）。`tpl` 是气泡模板。
pub fn focus_by_template(
    img: &Image,
    cards: &[CardSlot],
    tpl: &crate::bubble::Bubble,
    min_score: f32,
) -> Option<usize> {
    let h = img.height as f64;
    let w = img.width as f64;
    let b0 = cards.first().and_then(|c| c.box_)?;
    let ny = (b0[1] as f64 + b0[3] as f64 / 2.0) / h;
    let y0 = ((ny + BAND_TOP) * h) as usize;
    let y1 = ((ny + BAND_BOTTOM) * h) as usize;

    let (bx, _by, score) = tpl.find(img, y0, y1)?;
    if score < min_score {
        return None;
    }
    let bw = tpl.size().0 as f64;
    let cx = (bx as f64 + bw / 2.0) / w;

    let mut best = 0usize;
    let mut best_d = f64::MAX;
    for (i, c) in cards.iter().enumerate() {
        let Some(b) = c.box_ else { continue };
        let cc = (b[0] as f64 + b[2] as f64 / 2.0) / w;
        let d = (cx - cc).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    Some(best)
}

/// 优先模板匹配找焦点，失败/分数不够就退回 OCR
pub fn focus_auto(
    t: &Tasker,
    img: &Image,
    cards: &[CardSlot],
    tpl: Option<&crate::bubble::Bubble>,
) -> Option<usize> {
    if let Some(tpl) = tpl {
        if let Some(i) = focus_by_template(img, cards, tpl, 0.5) {
            return Some(i);
        }
    }
    focus_index(t, img, cards, true).0
}

/// 自动布局兜底：在整块卡区里找「选择」气泡，不限定窄带。
/// 模板匹配失败时退回一次整块区域的 OCR，找含「选择」的最靠上的文字。
pub fn focus_auto_global(
    t: &Tasker,
    img: &Image,
    cards: &[CardSlot],
    tpl: Option<&crate::bubble::Bubble>,
) -> Option<usize> {
    let h = img.height as f64;
    let w = img.width as f64;
    let b0 = cards.first().and_then(|c| c.box_)?;
    let ny = (b0[1] as f64 + b0[3] as f64 / 2.0) / h;
    let y0 = ((ny + 0.08) * h).max(0.45 * h) as usize;
    let y1 = (0.98 * h) as usize;

    let cx = if let Some(tpl) = tpl {
        // 先在卡片区域内找，避开底部「礼物重抽 / 放弃」那排同样的「选择」提示；
        // 找不到再放宽到整屏下半部分（普通界面的气泡位置更低）。
        let y_mid = ((0.68 * h) as usize).max(y0 + 1);
        let mut hit = tpl.find(img, y0, y_mid);
        if hit.map(|(_, _, s)| s < 0.6).unwrap_or(true) {
            hit = tpl.find(img, y0, y1);
        }
        let (bx, _by, score) = hit?;
        if score < 0.5 {
            return None;
        }
        (bx as f64 + tpl.size().0 as f64 / 2.0) / w
    } else {
        let hits = t
            .ocr_roi_ex(img, (0.02, y0 as f64 / h, 0.98, 0.98), false)
            .ok()?;
        let b = hits
            .iter()
            .filter(|x| x.text.contains("选择"))
            .min_by_key(|x| x.box_[1])?;
        (b.box_[0] as f64 + b.box_[2] as f64 / 2.0) / w
    };

    let mut best = 0usize;
    let mut best_d = f64::MAX;
    for (i, c) in cards.iter().enumerate() {
        let Some(b) = c.box_ else { continue };
        let cc = (b[0] as f64 + b[2] as f64 / 2.0) / w;
        let d = (cx - cc).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    Some(best)
}

/// 特殊界面用：在整块卡区轮询焦点，不限定窄带。
pub fn wait_focus_change_global(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    cards: &[CardSlot],
    tpl: Option<&crate::bubble::Bubble>,
    from: usize,
    timeout_ms: u64,
) -> Option<usize> {
    std::thread::sleep(std::time::Duration::from_millis(FOCUS_PRE_WAIT_MS));
    let start = std::time::Instant::now();
    loop {
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
        let Ok(img) = ctrl.screencap() else {
            continue;
        };
        if let Some(i) = focus_auto_global(t, &img, cards, tpl) {
            if i != from {
                return Some(i);
            }
        }
    }
}

/// 推格后先等动画走一段，避免前几帧还停在旧焦点上白抓
pub const FOCUS_PRE_WAIT_MS: u64 = 50;

/// 等焦点离开当前卡。
///
/// 原来的 `wait_changed` 盯的是**卡名带**，而气泡在卡名带下方，推格根本改不到那里，
/// 每次都要白等满超时。这里直接用模板匹配轮询焦点索引，语义精确、单次 12ms。
pub fn wait_focus_change(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    cards: &[CardSlot],
    tpl: Option<&crate::bubble::Bubble>,
    from: usize,
    timeout_ms: u64,
) -> Option<usize> {
    std::thread::sleep(std::time::Duration::from_millis(FOCUS_PRE_WAIT_MS));
    let start = std::time::Instant::now();
    loop {
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(6));
        let Ok(img) = ctrl.screencap() else {
            continue;
        };
        if let Some(i) = focus_auto(t, &img, cards, tpl) {
            if i != from {
                return Some(i);
            }
        }
    }
}
