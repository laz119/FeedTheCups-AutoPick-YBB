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

/// 焦点在第几张（0 起）。读不到气泡返回 None。
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

/// 一次「看」的结果
pub struct Look {
    pub cards: Vec<CardSlot>,
    pub focus: Option<usize>,
    /// 从开始等到判定稳定花了多久
    pub settle_ms: f64,
    /// 一共截了几帧
    pub shots: u32,
}

/// 判稳 + 读卡 + 判焦点。
///
/// 和原版的区别：原版用**整屏**降采样帧差 + 0.06/0.08s 轮询，且要连续两帧都稳定；
/// 这里只算**卡名带**的差、轮询 8ms、稳定帧直接就读。实测静止画面差值为 0.000，
/// 所以正常情况下第一帧就能结束。
pub fn settle_and_read(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    tpl: Option<&crate::bubble::Bubble>,
    timeout_ms: u64,
) -> Option<Look> {
    let start = std::time::Instant::now();
    let mut prev: Option<Image> = None;
    let mut shots = 0u32;

    loop {
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return None;
        }
        let Ok(img) = ctrl.screencap() else {
            std::thread::sleep(std::time::Duration::from_millis(8));
            continue;
        };
        shots += 1;

        let rois = name_rois_abs(&img);
        let band = crate::fingerprint::name_band(&rois);

        let stable = match &prev {
            None => false,
            Some(p) => crate::fingerprint::band_diff(p, &img, band) < STABLE_DIFF,
        };

        if stable {
            let cards = read_cards(t, &img, true);
            if cards.iter().all(|c| !c.is_empty()) {
                let focus = focus_auto(t, &img, &cards, tpl);
                return Some(Look {
                    cards,
                    focus,
                    settle_ms: start.elapsed().as_secs_f64() * 1000.0,
                    shots,
                });
            }
        }

        prev = Some(img);
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
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

/// 等画面变化：8ms 轮询卡名带差，超过阈值就返回 true。
/// 这就是替代原版 `time.sleep(3.0)` 的东西——动画一结束立刻继续。
pub fn wait_changed(
    ctrl: &crate::maa::Controller,
    band: [i32; 4],
    threshold: f64,
    timeout_ms: u64,
) -> bool {
    let start = std::time::Instant::now();
    let mut prev = match ctrl.screencap() {
        Ok(i) => i,
        Err(_) => return false,
    };
    loop {
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
        let Ok(img) = ctrl.screencap() else {
            continue;
        };
        if crate::fingerprint::band_diff(&prev, &img, band) > threshold {
            return true;
        }
        prev = img;
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
