//! 用模板匹配找「选择」气泡，替代那条 1728×270 的整带 OCR（实测 299ms）。
//!
//! 气泡是固定 UI 元素：一个白色圆角框 + 手柄图标 + 「选择」两个字，位置随焦点移动。
//! 做法是灰度归一化互相关（NCC），先 1/4 降采样粗搜，再在原图小窗口里精修。

use crate::maa::Image;

pub struct Bubble {
    w: usize,
    h: usize,
    /// 零均值归一化后的灰度
    v: Vec<f32>,
    #[allow(dead_code)]
    norm: f32,
}

fn gray_at(img: &Image, x: usize, y: usize) -> f32 {
    let iw = img.width as usize;
    let ch = img.channels as usize;
    let o = (y * iw + x) * ch;
    let (b, g, r) = (
        img.data[o] as f32,
        img.data[o + 1] as f32,
        img.data[o + 2] as f32,
    );
    0.114 * b + 0.587 * g + 0.299 * r
}

/// 把 ROI 灰度化并按 f 倍降采样（块平均）
fn gray_block(img: &Image, x0: usize, y0: usize, w: usize, h: usize, f: usize) -> Vec<f32> {
    let ow = w / f;
    let oh = h / f;
    let mut out = vec![0f32; ow * oh];
    for oy in 0..oh {
        for ox in 0..ow {
            let mut s = 0f32;
            for dy in 0..f {
                for dx in 0..f {
                    s += gray_at(img, x0 + ox * f + dx, y0 + oy * f + dy);
                }
            }
            out[oy * ow + ox] = s / (f * f) as f32;
        }
    }
    out
}

fn normalize(v: &mut [f32]) -> f32 {
    let n = v.len() as f32;
    let mean = v.iter().sum::<f32>() / n;
    for x in v.iter_mut() {
        *x -= mean;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    norm
}

impl Bubble {
    pub fn load(path: impl AsRef<std::path::Path>) -> Option<Self> {
        let img = Image::load_bgr(path).ok()?;
        Self::from_image(&img)
    }

    pub fn from_image(img: &Image) -> Option<Self> {
        let (w, h) = (img.width as usize, img.height as usize);
        if w < 8 || h < 8 {
            return None;
        }
        let mut v: Vec<f32> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| gray_at(img, x, y))
            .collect();
        let norm = normalize(&mut v);
        Some(Self { w, h, v, norm })
    }

    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// 在 `img` 里找气泡。`y0..y1` 是要搜的纵向范围，横向全图。
    /// 返回（左上角 x, 左上角 y, 相似度）。
    pub fn find(&self, img: &Image, y0: usize, y1: usize) -> Option<(usize, usize, f32)> {
        const F: usize = 4;
        let iw = img.width as usize;
        let ih = img.height as usize;
        if y1 <= y0 || y1 > ih || self.w > iw || self.h > y1 - y0 {
            return None;
        }

        // 粗搜：模板和搜索区都降采样 F 倍
        let tw = self.w / F;
        let th = self.h / F;
        if tw < 4 || th < 4 {
            return None;
        }
        let mut tv: Vec<f32> = Vec::with_capacity(tw * th);
        for oy in 0..th {
            for ox in 0..tw {
                let mut s = 0f32;
                for dy in 0..F {
                    for dx in 0..F {
                        s += self.v[(oy * F + dy) * self.w + ox * F + dx];
                    }
                }
                tv.push(s / (F * F) as f32);
            }
        }
        normalize(&mut tv);

        let band_h = y1 - y0;
        let sw = iw / F;
        let sh = band_h / F;
        if sw < tw || sh < th {
            return None;
        }
        let small = gray_block(img, 0, y0, iw, band_h, F);

        let n = (tw * th) as f32;
        let mut best = (0usize, 0usize, -2.0f32);
        for oy in 0..=(sh - th) {
            for ox in 0..=(sw - tw) {
                let mut dot = 0f32;
                let mut acc = 0f32;
                let mut acc2 = 0f32;
                for ty in 0..th {
                    let base = (oy + ty) * sw + ox;
                    for tx in 0..tw {
                        let i = small[base + tx];
                        dot += tv[ty * tw + tx] * i;
                        acc += i;
                        acc2 += i * i;
                    }
                }
                let var = acc2 - acc * acc / n;
                if var <= 1e-6 {
                    continue;
                }
                let score = dot / var.sqrt();
                if score > best.2 {
                    best = (ox, oy, score);
                }
            }
        }
        if best.2 < 0.0 {
            return None;
        }

        // 精修：回到原图，在粗搜结果的 ±2F 窗口里找
        let cx = best.0 * F;
        let cy = y0 + best.1 * F;
        let mut fbest = (cx, cy, -2.0f32);
        let lo_x = cx.saturating_sub(F * 2);
        let hi_x = (cx + F * 2).min(iw.saturating_sub(self.w));
        let lo_y = (cy.saturating_sub(F * 2)).max(y0);
        let hi_y = (cy + F * 2).min(y1.saturating_sub(self.h));

        for oy in lo_y..=hi_y {
            for ox in lo_x..=hi_x {
                let mut dot = 0f32;
                let mut acc = 0f32;
                let mut acc2 = 0f32;
                for ty in 0..self.h {
                    for tx in 0..self.w {
                        let i = gray_at(img, ox + tx, oy + ty);
                        dot += self.v[ty * self.w + tx] * i;
                        acc += i;
                        acc2 += i * i;
                    }
                }
                let var = acc2 - acc * acc / (self.w * self.h) as f32;
                if var <= 1e-6 {
                    continue;
                }
                // v 已经是单位向量，且零均值 -> dot 就是相关系数分子，
                // 只需再除以被搜区域的范数即可（分母别再除模板范数）
                let score = dot / var.sqrt();
                if score > fbest.2 {
                    fbest = (ox, oy, score);
                }
            }
        }
        Some(fbest)
    }
}
