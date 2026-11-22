//! 杯杯倒满 地铁模式自动选卡 —— Rust 版

pub mod assets;
pub mod bubble;
pub mod engine;
pub mod fingerprint;
pub mod input;
pub mod maa;
pub mod maa_sys;
pub mod ocr;
pub mod picker;
pub mod queue;
pub mod score;
pub mod vigem;
pub mod win;
pub mod xinput;

pub mod consts {
    /// 目标窗口标题关键字
    pub const WINDOW_KEY: &str = "Feed The Cups";
    /// 卡名匹配阈值
    pub const MATCH_THRESHOLD: f64 = 0.55;

    /// 三张卡名的 ROI（相对坐标 x0, y0, x1, y1）
    pub const NAME_ROIS: [(f64, f64, f64, f64); 3] = [
        (0.19, 0.365, 0.305, 0.435),
        (0.44, 0.365, 0.57, 0.435),
        (0.7, 0.365, 0.83, 0.435),
    ];
}

/// MIT 许可证全文，内嵌进二进制，`ybb-cli --license` 时打印
pub const LICENSE_TEXT: &str = include_str!("../LICENSE");
