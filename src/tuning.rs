//! 性能/超时参数集中在这里，默认值保持当前实测最优；可用环境变量临时覆盖。

#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    pub hold_ms: u64,
    pub focus_timeout_ms: u64,
    pub settle_timeout_ms: u64,
    pub reroll_timeout_ms: u64,
    pub ort_threads: usize,
    pub batch_taps: bool,
    pub frame_reuse: bool,
    pub screenshot_long_side: u32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            hold_ms: 30,
            focus_timeout_ms: 1500,
            settle_timeout_ms: 3000,
            reroll_timeout_ms: 3500,
            ort_threads: 4,
            batch_taps: true,
            frame_reuse: true,
            screenshot_long_side: 1920,
        }
    }
}

impl Tuning {
    pub fn from_env() -> Self {
        let mut t = Self::default();
        let num = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u64>().ok());
        if let Some(v) = num("YBB_HOLD_MS") {
            t.hold_ms = v.clamp(5, 200);
        }
        if let Some(v) = num("YBB_FOCUS_TIMEOUT_MS") {
            t.focus_timeout_ms = v.clamp(200, 10000);
        }
        if let Some(v) = num("YBB_SETTLE_TIMEOUT_MS") {
            t.settle_timeout_ms = v.clamp(500, 20000);
        }
        if let Some(v) = num("YBB_REROLL_TIMEOUT_MS") {
            t.reroll_timeout_ms = v.clamp(500, 20000);
        }
        if let Some(v) = num("YBB_ORT_THREADS") {
            t.ort_threads = (v as usize).clamp(1, 16);
        }
        if let Some(v) = num("YBB_LONG_SIDE") {
            t.screenshot_long_side = (v as u32).clamp(640, 3840);
        }
        if let Ok(v) = std::env::var("YBB_BATCH_TAPS") {
            t.batch_taps = v != "0";
        }
        if let Ok(v) = std::env::var("YBB_FRAME_REUSE") {
            t.frame_reuse = v != "0";
        }
        t
    }
}
