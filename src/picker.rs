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

/// 用 ORT 对固定三个卡名 ROI 做一次 batch 识别。
pub fn read_cards_ort(o: &crate::ort_ocr::OrtOcr, img: &Image) -> Vec<CardSlot> {
    let abs = name_rois_abs(img);
    let texts = o.read_rois(img, &NAME_ROIS).unwrap_or_default();
    texts
        .into_iter()
        .enumerate()
        .map(|(i, text)| CardSlot {
            name: crate::score::norm(&text),
            raw: text,
            box_: Some(abs[i]),
        })
        .collect()
}

#[derive(Clone)]
struct CachedLayout {
    w: u32,
    h: u32,
    boxes: [[i32; 4]; 3],
}

static LAYOUT_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<CachedLayout>>> =
    std::sync::OnceLock::new();

fn layout_cache() -> &'static std::sync::Mutex<Option<CachedLayout>> {
    LAYOUT_CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

fn cached_boxes(img: &Image) -> Option<[[i32; 4]; 3]> {
    let g = layout_cache().lock().ok()?;
    g.as_ref()
        .filter(|c| c.w == img.width && c.h == img.height)
        .map(|c| c.boxes)
}

fn store_boxes(img: &Image, boxes: [[i32; 4]; 3]) {
    if let Ok(mut g) = layout_cache().lock() {
        *g = Some(CachedLayout {
            w: img.width,
            h: img.height,
            boxes,
        });
    }
}

fn clear_boxes() {
    if let Ok(mut g) = layout_cache().lock() {
        *g = None;
    }
}

/// 用一组绝对框读三张卡名；优先 ORT，失败回退 MAA。
fn read_cards_boxes(
    ort: Option<&crate::ort_ocr::OrtOcr>,
    t: &Tasker,
    img: &Image,
    boxes: [[i32; 4]; 3],
) -> Vec<CardSlot> {
    if let Some(o) = ort {
        let w = img.width as f64;
        let h = img.height as f64;
        let rois = [
            (
                boxes[0][0] as f64 / w,
                boxes[0][1] as f64 / h,
                (boxes[0][0] + boxes[0][2]) as f64 / w,
                (boxes[0][1] + boxes[0][3]) as f64 / h,
            ),
            (
                boxes[1][0] as f64 / w,
                boxes[1][1] as f64 / h,
                (boxes[1][0] + boxes[1][2]) as f64 / w,
                (boxes[1][1] + boxes[1][3]) as f64 / h,
            ),
            (
                boxes[2][0] as f64 / w,
                boxes[2][1] as f64 / h,
                (boxes[2][0] + boxes[2][2]) as f64 / w,
                (boxes[2][1] + boxes[2][3]) as f64 / h,
            ),
        ];
        if let Ok(texts) = o.read_rois(img, &rois) {
            if texts.iter().all(|s| !s.is_empty()) {
                return texts
                    .into_iter()
                    .enumerate()
                    .map(|(i, text)| CardSlot {
                        name: crate::score::norm(&text),
                        raw: text,
                        box_: Some(boxes[i]),
                    })
                    .collect();
            }
        }
    }

    let w = img.width as f64;
    let h = img.height as f64;
    boxes
        .iter()
        .map(|b| {
            let frac = (
                b[0] as f64 / w,
                b[1] as f64 / h,
                (b[0] + b[2]) as f64 / w,
                (b[1] + b[3]) as f64 / h,
            );
            let hit = t
                .ocr_roi_ex(img, frac, true)
                .unwrap_or_default()
                .into_iter()
                .max_by_key(|x| x.text.chars().count());
            match hit {
                Some(r) => CardSlot {
                    name: crate::score::norm(&r.text),
                    raw: r.text,
                    box_: Some(*b),
                },
                None => CardSlot {
                    name: String::new(),
                    raw: String::new(),
                    box_: None,
                },
            }
        })
        .collect()
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
pub const STABLE_MS: u64 = 20;
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
    // 每张卡名至少两个汉字；少认字的帧不能进入决策，否则「球」会被当成「气球」。
    cards.iter().all(|c| looks_like_card_name(&c.name))
}

/// 卡名不该有百分号、冒号这类说明文字；数字/字母允许（例如「2倍速」）。
fn looks_like_card_name(s: &str) -> bool {
    let n = s.chars().count();
    let cjk = s.chars().filter(|c| is_cjk(*c)).count();
    cjk >= 2
        && n <= 8
        && !s.contains('%')
        && !s.contains('：')
        && !s.contains(':')
        && !s.contains('。')
        && !s.contains('，')
}

/// 三张卡名是否都完整可决策。
pub fn cards_ok(cards: &[CardSlot]) -> bool {
    cards.len() >= 3 && cards.iter().all(|c| !c.is_empty()) && plausible(cards)
}

/// 判稳 + 读卡 + 判焦点。
///
/// `baseline` 有值时表示这是「重抽之后」：除了要求连续两帧稳定，
/// 还要求这一帧和重抽前明显不同，避免动画还没开始就把旧画面当成新画面。
fn settle_core(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    tpl: Option<&crate::bubble::Bubble>,
    ort: Option<&crate::ort_ocr::OrtOcr>,
    clear: Option<&dyn Fn()>,
    timeout_ms: u64,
    baseline: Option<&Image>,
    seed: Option<Image>,
    stable_needed: u32,
) -> Result<Look, SettleFail> {
    let start = std::time::Instant::now();
    let seeded = seed.is_some();
    let mut prev_t: Option<std::time::Instant> = if seeded { Some(start) } else { None };
    let mut stable_since: Option<std::time::Instant> = None;
    let mut shots = 0u32;
    let mut saw_change = false;
    let mut stable_run = 0u32;
    // 第一次识别成功就直接过，不等 50ms；只有失败重试才走最短稳定时间
    let mut first_try = true;
    // 重抽和「复用上一帧」都可能碰到动画中间帧，卡名要像正常卡名才接受
    let require_plausible = baseline.is_some() || seeded;

    // 不再从 seed 直接读卡：实测 seed 是翻牌动画帧，焦点气泡还没出现，
    // 快路径几乎每轮都失败，白浪费一次 OCR，然后还是走正常判稳。
    // seed 只作为判稳的上一帧基准（下面 prev = seed），不单独读卡。
    let mut tried_clear = false;
    let mut prev: Option<Image> = seed;
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
            let cards = match ort {
                Some(o) => read_cards_ort(o, &img),
                None => read_cards(t, &img, true),
            };
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

            // 卡名完整但焦点没读到（介绍面板挡住气泡）：按 S 两次再重读焦点。
            // 卡名不完整则不按 S，交给外层重试/重抽处理。
            if normal_ok && !tried_clear {
                tried_clear = true;
                if let Some(clear) = clear {
                    clear();
                    std::thread::sleep(std::time::Duration::from_millis(140));
                    prev = None;
                    prev_t = None;
                    stable_since = None;
                    stable_run = 0;
                    first_try = true;
                    shots = 0;
                    continue;
                }
            }

            // 固定 ROI 读不到卡名或焦点：先试上次自动布局缓存的框
            if let Some(boxes) = cached_boxes(&img) {
                let cards = read_cards_boxes(ort, t, &img, boxes);
                if cards.iter().all(|c| !c.is_empty()) && plausible(&cards) {
                    if let Some(focus) = focus_auto_global(t, &img, &cards, tpl) {
                        return Ok(Look {
                            cards,
                            focus: Some(focus),
                            settle_ms: start.elapsed().as_secs_f64() * 1000.0,
                            shots,
                            frame: img,
                            auto_layout: true,
                        });
                    }
                }
                // 缓存框已经不适配当前画面，丢掉重找
                clear_boxes();
            }

            // 固定 ROI 读不到卡名或焦点：自动布局兜底（恶魔能力等特殊界面）
            let auto_cards = read_cards_auto(t, &img);
            if auto_cards.iter().all(|c| !c.is_empty()) && plausible(&auto_cards) {
                let focus = focus_auto_global(t, &img, &auto_cards, tpl);
                if let [Some(a), Some(b), Some(c)] = [
                    auto_cards[0].box_,
                    auto_cards[1].box_,
                    auto_cards[2].box_,
                ] {
                    store_boxes(&img, [a, b, c]);
                }
                return Ok(Look {
                    cards: auto_cards,
                    focus,
                    settle_ms: start.elapsed().as_secs_f64() * 1000.0,
                    shots,
                    frame: img,
                    auto_layout: true,
                });
            }

            // 卡名读到了但焦点缺失：
            // - 重抽场景（baseline 有值）：立即返回，焦点交给引擎按 S 处理，
            //   不能在这里反复 continue 把 timeout 耗光导致误报 NoChange。
            // - 普通场景：继续重试，让 clear 分支有机会按 S 清介绍。
            if normal_ok {
                if baseline.is_some() {
                    return Ok(Look {
                        cards,
                        focus: normal_focus,
                        settle_ms: start.elapsed().as_secs_f64() * 1000.0,
                        shots,
                        frame: img,
                        auto_layout: false,
                    });
                }
                stable_run = 0;
                stable_since = Some(now);
                continue;
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
    ort: Option<&crate::ort_ocr::OrtOcr>,
    clear: Option<&dyn Fn()>,
    timeout_ms: u64,
) -> Option<Look> {
    settle_core(ctrl, t, tpl, ort, clear, timeout_ms, None, None, 1).ok()
}

/// 带「上一帧」的一次「看」：`seed` 是确认后 wait_changed 返回的变化帧，
/// 直接拿它当第一帧比较，省掉一次截图。有 seed 时多要一次稳定比较，防止动画中间误判。
pub fn settle_and_read_seeded(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    tpl: Option<&crate::bubble::Bubble>,
    ort: Option<&crate::ort_ocr::OrtOcr>,
    clear: Option<&dyn Fn()>,
    timeout_ms: u64,
    seed: Option<Image>,
) -> Option<Look> {
    let need = 1;
    settle_core(ctrl, t, tpl, ort, clear, timeout_ms, None, seed, need).ok()
}

/// 重抽之后的一次「看」：稳定 **且** 必须不同于 `baseline`。
/// 稳定窗口取 3 次比较，避开翻牌动画中间的假稳定帧。
pub fn settle_after_change(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    tpl: Option<&crate::bubble::Bubble>,
    ort: Option<&crate::ort_ocr::OrtOcr>,
    clear: Option<&dyn Fn()>,
    baseline: &Image,
    timeout_ms: u64,
) -> Result<Look, SettleFail> {
    settle_core(ctrl, t, tpl, ort, clear, timeout_ms, Some(baseline), None, 1)
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
/// 等画面相对 `prev` 发生变化，并且**稳定下来**再返回。
///
/// 翻牌动画第一帧就和 `prev` 不同了，但卡名可能还是旧的。如果直接返回这一帧，
/// 下一轮的快路径会读到旧卡名（触发「卡名和上一轮相同」重读，白白多一次截图+OCR）。
/// 所以这里检测到变化后继续等，直到连续两帧卡名带几乎不变（STABLE_DIFF）才返回。
pub fn wait_changed(
    ctrl: &crate::maa::Controller,
    band: [i32; 4],
    prev: &Image,
    threshold: f64,
    timeout_ms: u64,
) -> Option<Image> {
    let start = std::time::Instant::now();
    let mut changed_img: Option<Image> = None;
    let mut last: Option<Image> = None;
    let mut stable_run = 0u32;
    loop {
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return changed_img;
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
        let Ok(img) = ctrl.screencap() else {
            continue;
        };
        // 阶段1：等画面相对 prev 发生明显变化
        if changed_img.is_none() {
            if crate::fingerprint::band_diff(prev, &img, band) > threshold {
                changed_img = Some(img);
                stable_run = 0;
            }
            continue;
        }
        // 阶段2：变化后等画面真正稳定（wait_changed 用更严格的阈值，
        // 避免动画慢速阶段两帧差 <0.5 被误判为稳定、提前返回）
        const WAIT_STABLE_DIFF: f64 = 0.2;
        if let Some(l) = &last {
            if crate::fingerprint::band_diff(l, &img, band) < WAIT_STABLE_DIFF {
                stable_run += 1;
                if stable_run >= 2 {
                    // 稳定了，但还要确认和 prev 不同（防动画视觉效果导致的假变化）
                    if crate::fingerprint::band_diff(prev, &img, band) > threshold {
                        return Some(img);
                    }
                    // 稳定帧和 prev 一样 = 假变化，重置继续等
                    stable_run = 0;
                }
            } else {
                stable_run = 0;
            }
        }
        last = Some(img);
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
    focus_by_template_scored(img, cards, tpl, min_score).map(|(i, _)| i)
}

fn focus_by_template_scored(
    img: &Image,
    cards: &[CardSlot],
    tpl: &crate::bubble::Bubble,
    min_score: f32,
) -> Option<(usize, f32)> {
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
    Some((best, score))
}

/// 焦点判定：**模板优先**（~25ms），失败退回 OCR（~200ms）。
///
/// 模板 `focus_template.png` 是从实际游戏画面裁的白底黑字「选择」气泡，
/// 与真实 UI 一致，多帧实测模板和 OCR 判定结果完全一致。
/// 模板阈值 0.7：足够拦住误匹配，又不会因为轻微抗锯齿/缩放漏判。
pub fn focus_auto(
    t: &Tasker,
    img: &Image,
    cards: &[CardSlot],
    tpl: Option<&crate::bubble::Bubble>,
) -> Option<usize> {
    if let Some(tpl) = tpl {
        if let Some(i) = focus_by_template(img, cards, tpl, 0.7) {
            return Some(i);
        }
        if let Some((i, _)) = focus_global_scored(img, cards, tpl, 0.7) {
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
    if let Some(tpl) = tpl {
        return focus_global_scored(img, cards, tpl, 0.5).map(|(i, _)| i);
    }

    let h = img.height as f64;
    let w = img.width as f64;
    let b0 = cards.first().and_then(|c| c.box_)?;
    let ny = (b0[1] as f64 + b0[3] as f64 / 2.0) / h;
    let y0 = ((ny + 0.08) * h).max(0.45 * h) as usize;
    let hits = t
        .ocr_roi_ex(img, (0.02, y0 as f64 / h, 0.98, 0.98), false)
        .ok()?;
    let b = hits
        .iter()
        .filter(|x| x.text.contains("选择"))
        .min_by_key(|x| x.box_[1])?;
    let cx = (b.box_[0] as f64 + b.box_[2] as f64 / 2.0) / w;

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

/// 特殊界面的模板焦点搜索，返回索引和分数；不做 OCR 回退。
fn focus_global_scored(
    img: &Image,
    cards: &[CardSlot],
    tpl: &crate::bubble::Bubble,
    min_score: f32,
) -> Option<(usize, f32)> {
    let h = img.height as f64;
    let w = img.width as f64;
    let b0 = cards.first().and_then(|c| c.box_)?;
    let ny = (b0[1] as f64 + b0[3] as f64 / 2.0) / h;
    let y0 = ((ny + 0.08) * h).max(0.45 * h) as usize;
    let y1 = (0.98 * h) as usize;
    let y_mid = ((0.68 * h) as usize).max(y0 + 1);
    let mut hit = tpl.find(img, y0, y_mid);
    if hit.map(|(_, _, s)| s < 0.6).unwrap_or(true) {
        hit = tpl.find(img, y0, y1);
    }
    let (bx, _by, score) = hit?;
    if score < min_score {
        return None;
    }
    let cx = (bx as f64 + tpl.size().0 as f64 / 2.0) / w;

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
    Some((best, score))
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
        let detected = match tpl {
            Some(tpl) => focus_global_scored(&img, cards, tpl, 0.5),
            None => focus_auto_global(t, &img, cards, None).map(|i| (i, 1.0)),
        };
        if let Some((i, score)) = detected {
            if i != from {
                if score >= 0.75 {
                    return Some(i);
                }
                // 低分才连续两帧确认，避免动画中间帧误判
                std::thread::sleep(std::time::Duration::from_millis(6));
                if let Ok(img2) = ctrl.screencap() {
                    let again = match tpl {
                        Some(tpl) => focus_global_scored(&img2, cards, tpl, 0.5)
                            .map(|(i, _)| i),
                        None => focus_auto_global(t, &img2, cards, None),
                    };
                    if again == Some(i) {
                        return Some(i);
                    }
                }
            }
        }
    }
}

/// 推格后先等动画走一段，避免前几帧还停在旧焦点上白抓
pub const FOCUS_PRE_WAIT_MS: u64 = 16;

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
            // 超时前再用一次全局搜索兜底（气泡不在窄带里时也能找到）
            if let Ok(img) = ctrl.screencap() {
                if let Some(i) = focus_auto_global(t, &img, cards, tpl) {
                    if i != from {
                        return Some(i);
                    }
                }
            }
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(6));
        let Ok(img) = ctrl.screencap() else {
            continue;
        };
        let detected = match tpl {
            Some(tpl) => focus_by_template_scored(&img, cards, tpl, 0.5)
                .or_else(|| focus_global_scored(&img, cards, tpl, 0.5)),
            None => focus_auto(t, &img, cards, None).map(|i| (i, 1.0)),
        };
        if let Some((i, score)) = detected {
            if i != from {
                if std::env::var("YBB_FOCUS_DEBUG").map(|v| v == "1").unwrap_or(false) {
                    eprintln!("[focus] i={} score={:.3}", i, score);
                }
                // 分数高直接认；低分才连续两帧确认，避免动画中间帧误判
                if score >= 0.75 {
                    return Some(i);
                }
                std::thread::sleep(std::time::Duration::from_millis(6));
                if let Ok(img2) = ctrl.screencap() {
                    let again = match tpl {
                        Some(tpl) => focus_by_template(&img2, cards, tpl, 0.5),
                        None => focus_auto(t, &img2, cards, None),
                    };
                    if again == Some(i) {
                        return Some(i);
                    }
                }
            }
        }
    }
}

/// 连按多格后，等焦点最终落到 target。
pub fn wait_focus_target(
    ctrl: &crate::maa::Controller,
    t: &Tasker,
    cards: &[CardSlot],
    tpl: Option<&crate::bubble::Bubble>,
    target: usize,
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
        let detected = match tpl {
            Some(tpl) => focus_by_template_scored(&img, cards, tpl, 0.5)
                .or_else(|| focus_global_scored(&img, cards, tpl, 0.5)),
            None => focus_auto(t, &img, cards, None).map(|i| (i, 1.0)),
        };
        if let Some((i, score)) = detected {
            if i == target {
                if std::env::var("YBB_FOCUS_DEBUG").map(|v| v == "1").unwrap_or(false) {
                    eprintln!("[focus] target i={} score={:.3}", i, score);
                }
                if score >= 0.75 {
                    return Some(target);
                }
                std::thread::sleep(std::time::Duration::from_millis(6));
                if let Ok(img2) = ctrl.screencap() {
                    let again = match tpl {
                        Some(tpl) => focus_by_template(&img2, cards, tpl, 0.5),
                        None => focus_auto(t, &img2, cards, None),
                    };
                    if again == Some(target) {
                        return Some(target);
                    }
                }
            }
        }
    }
}
