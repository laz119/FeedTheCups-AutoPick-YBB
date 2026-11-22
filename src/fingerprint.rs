//! 卡名指纹：用「梯度网格 + 余弦相似度」代替逐字 OCR。
//!
//! 为什么不直接用灰度：卡面底色随卡的类型变化（黄/红/绿/紫），卡上还有持续流动的高光，
//! 同一张卡在不同时刻的灰度差很多。而**梯度幅值**只反映"哪里有笔画边"，对底色和整体亮度不敏感，
//! 所以先把 ROI 转成梯度幅值图，再压成固定网格，最后做零均值归一化。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::maa::Image;

/// 指纹网格尺寸。240 维足够区分「气球 / 气球+」这类近似名，又短到可以毫秒级比对。
pub const GRID_W: usize = 24;
pub const GRID_H: usize = 10;

/// 余弦相似度阈值：高于它认为是同一张卡
pub const MATCH_SIM: f32 = 0.92;

/// 从一帧图的绝对像素 ROI 生成指纹
pub fn fingerprint(img: &Image, roi: [i32; 4]) -> Option<Vec<f32>> {
    let (x0, y0) = (roi[0].max(0) as usize, roi[1].max(0) as usize);
    let w = roi[2].max(0) as usize;
    let h = roi[3].max(0) as usize;
    if w < GRID_W || h < GRID_H {
        return None;
    }
    let iw = img.width as usize;
    let ih = img.height as usize;
    let ch = img.channels as usize;
    if x0 + w > iw || y0 + h > ih {
        return None;
    }

    // 1) 灰度
    let mut gray = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let o = ((y0 + y) * iw + (x0 + x)) * ch;
            let (b, g, r) = (img.data[o] as f32, img.data[o + 1] as f32, img.data[o + 2] as f32);
            gray[y * w + x] = 0.114 * b + 0.587 * g + 0.299 * r;
        }
    }

    // 2) 梯度幅值（中心差分）
    let mut grad = vec![0f32; w * h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let gx = gray[i + 1] - gray[i - 1];
            let gy = gray[i + w] - gray[i - w];
            grad[i] = (gx * gx + gy * gy).sqrt();
        }
    }

    // 3) 网格平均
    let mut v = vec![0f32; GRID_W * GRID_H];
    let mut cnt = vec![0f32; GRID_W * GRID_H];
    for y in 0..h {
        let gy = y * GRID_H / h;
        for x in 0..w {
            let gx = x * GRID_W / w;
            let cell = gy * GRID_W + gx;
            v[cell] += grad[y * w + x];
            cnt[cell] += 1.0;
        }
    }
    for i in 0..v.len() {
        if cnt[i] > 0.0 {
            v[i] /= cnt[i];
        }
    }

    // 4) 零均值 + 单位长度（之后点积即余弦）
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    for x in v.iter_mut() {
        *x -= mean;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-6 {
        return None; // 纯色块（没认到字）
    }
    for x in v.iter_mut() {
        *x /= norm;
    }
    Some(v)
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return -1.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// 另一种指纹：先按 ROI 自身的亮度范围做归一化，再用高阈值取出**白色文字**的掩膜，
/// 然后压成网格。相比梯度版，它对卡面底色（黄/红/绿/紫）和高光的鲁棒性更好。
pub fn fingerprint_binary(img: &Image, roi: [i32; 4]) -> Option<Vec<f32>> {
    let (x0, y0) = (roi[0].max(0) as usize, roi[1].max(0) as usize);
    let w = roi[2].max(0) as usize;
    let h = roi[3].max(0) as usize;
    if w < GRID_W || h < GRID_H {
        return None;
    }
    let iw = img.width as usize;
    let ih = img.height as usize;
    let ch = img.channels as usize;
    if x0 + w > iw || y0 + h > ih {
        return None;
    }

    // 灰度 + 极值
    let mut gray = vec![0f32; w * h];
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for y in 0..h {
        for x in 0..w {
            let o = ((y0 + y) * iw + (x0 + x)) * ch;
            let (b, g, r) = (img.data[o] as f32, img.data[o + 1] as f32, img.data[o + 2] as f32);
            let v = 0.114 * b + 0.587 * g + 0.299 * r;
            gray[y * w + x] = v;
            if v < lo {
                lo = v;
            }
            if v > hi {
                hi = v;
            }
        }
    }
    let span = hi - lo;
    if span < 25.0 {
        return None; // 几乎纯色，没有文字
    }

    // 取最亮的那一小撮作为"文字"（白字带深色描边）
    let mut hist = [0usize; 32];
    for &v in &gray {
        let b = (((v - lo) / span) * 31.0) as usize;
        hist[b.min(31)] += 1;
    }
    // 找出前 12% 的亮度分位点做阈值
    let target = (gray.len() as f32 * 0.12) as usize;
    let mut acc = 0usize;
    let mut cut = 31usize;
    for (i, c) in hist.iter().enumerate().rev() {
        acc += c;
        if acc >= target {
            cut = i;
            break;
        }
    }
    let thresh = lo + span * (cut as f32 / 31.0);

    // 网格平均掩膜
    let mut v = vec![0f32; GRID_W * GRID_H];
    let mut cnt = vec![0f32; GRID_W * GRID_H];
    for y in 0..h {
        let gy = y * GRID_H / h;
        for x in 0..w {
            let gx = x * GRID_W / w;
            let cell = gy * GRID_W + gx;
            if gray[y * w + x] >= thresh {
                v[cell] += 1.0;
            }
            cnt[cell] += 1.0;
        }
    }
    for i in 0..v.len() {
        if cnt[i] > 0.0 {
            v[i] /= cnt[i];
        }
    }

    let mean = v.iter().sum::<f32>() / v.len() as f32;
    for x in v.iter_mut() {
        *x -= mean;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-6 {
        return None;
    }
    for x in v.iter_mut() {
        *x /= norm;
    }
    Some(v)
}

/// 一条模板
#[derive(Serialize, Deserialize, Clone)]
pub struct Template {
    pub name: String,
    pub vec: Vec<f32>,
}

/// 模板库
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Store {
    pub items: Vec<Template>,
}

impl Store {
    pub fn load(path: impl AsRef<Path>) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let s = serde_json::to_string(self).unwrap_or_else(|_| "{\"items\":[]}".into());
        std::fs::write(path, s)
    }

    /// 加一条；如果已有高度相似的（同一张卡），不重复加。
    /// 返回 true 表示真的新增了。
    pub fn add(&mut self, name: &str, vec: Vec<f32>) -> bool {
        if name.is_empty() {
            return false;
        }
        for t in &self.items {
            if t.name == name && cosine(&t.vec, &vec) > 0.98 {
                return false;
            }
        }
        self.items.push(Template {
            name: name.to_string(),
            vec,
        });
        true
    }

    /// 最相似的模板
    pub fn best(&self, vec: &[f32]) -> Option<(String, f32)> {
        let mut b: Option<(String, f32)> = None;
        for t in &self.items {
            let s = cosine(&t.vec, vec);
            if b.as_ref().map(|(_, bs)| s > *bs).unwrap_or(true) {
                b = Some((t.name.clone(), s));
            }
        }
        b
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.items.iter().map(|t| t.name.clone()).collect();
        v.sort();
        v.dedup();
        v
    }
}

/// 两块 ROI 的平均灰度差，用来判断画面是否静止（原版用降采样帧差，这里只算卡名带）
pub fn band_diff(a: &Image, b: &Image, roi: [i32; 4]) -> f64 {
    if a.width != b.width || a.height != b.height || a.channels != b.channels {
        return f64::MAX;
    }
    let (x0, y0) = (roi[0].max(0) as usize, roi[1].max(0) as usize);
    let w = roi[2].max(0) as usize;
    let h = roi[3].max(0) as usize;
    let iw = a.width as usize;
    let ch = a.channels as usize;
    if x0 + w > iw || (y0 + h) > a.height as usize {
        return f64::MAX;
    }
    let mut sum = 0f64;
    let mut n = 0f64;
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let o = ((y0 + y) * iw + (x0 + x)) * ch;
            for c in 0..3 {
                sum += (a.data[o + c] as f64 - b.data[o + c] as f64).abs();
            }
            n += 3.0;
            x += 2;
        }
        y += 2;
    }
    if n > 0.0 {
        sum / n
    } else {
        f64::MAX
    }
}

/// 三个卡名 ROI 的并集（用于判稳）
pub fn name_band(rois: &[[i32; 4]; 3]) -> [i32; 4] {
    let x0 = rois.iter().map(|r| r[0]).min().unwrap_or(0);
    let y0 = rois.iter().map(|r| r[1]).min().unwrap_or(0);
    let x1 = rois.iter().map(|r| r[0] + r[2]).max().unwrap_or(0);
    let y1 = rois.iter().map(|r| r[1] + r[3]).max().unwrap_or(0);
    [x0, y0, x1 - x0, y1 - y0]
}
