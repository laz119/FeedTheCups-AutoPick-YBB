//! 直接走 ONNX Runtime 的卡名识别：三张 crop 一次 batch。
//!
//! 只替换「识别」这一步，截图、找卡、焦点仍由原有逻辑负责。

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use image::{imageops::FilterType, Rgb, RgbImage};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;

use crate::maa::Image;

const REC_HEIGHT: u32 = 48;
const REC_MAX_WIDTH: u32 = 320;
const BATCH: usize = 3;

pub struct OrtOcr {
    session: Mutex<Session>,
    keys: Vec<String>,
    classes: usize,
}


/// ONNX Runtime 每个进程只能初始化一次；GUI 停止后「继续」会再次创建 OrtOcr。
static ORT_ENV: OnceLock<Arc<ort::environment::Environment>> = OnceLock::new();

fn load_env(dll: &Path) -> Result<Arc<ort::environment::Environment>, String> {
    if let Some(env) = ORT_ENV.get() {
        return Ok(env.clone());
    }
    let env = Arc::new(
        ort::init_from(dll)
            .map_err(|e| format!("加载 onnxruntime.dll 失败: {e}"))?
            .build()
            .map_err(|e| format!("初始化 ONNX Runtime 失败: {e}"))?,
    );
    let _ = ORT_ENV.set(env.clone());
    Ok(env)
}

/// 进程级 ORT 会话缓存：GUI 打开时可预加载，点开始时直接复用。
static CACHE: OnceLock<Mutex<Option<Arc<OrtOcr>>>> = OnceLock::new();

/// 取缓存会话；没有就加载一次并放进缓存。
pub fn cached(threads: usize) -> Result<Arc<OrtOcr>, String> {
    let slot = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().map_err(|e| e.to_string())?;
    if let Some(o) = guard.as_ref() {
        return Ok(o.clone());
    }
    let o = Arc::new(OrtOcr::from_assets(threads)?);
    *guard = Some(o.clone());
    Ok(o)
}

/// 后台预加载（GUI 启动时调用，不阻塞界面）。
pub fn preload(threads: usize) {
    std::thread::spawn(move || {
        let _ = cached(threads);
    });
}

/// 释放缓存会话；正在跑的任务持有的 Arc 会等任务结束后自然释放。
pub fn clear_cache() {
    if let Some(slot) = CACHE.get() {
        if let Ok(mut guard) = slot.lock() {
            *guard = None;
        }
    }
}

impl OrtOcr {
    /// 按 YBB 的资源目录布局找 dll/model/keys。
    pub fn from_assets(threads: usize) -> Result<Self, String> {
        let dll = crate::assets::find("_internal/ort", "onnxruntime.dll")
            .ok_or_else(|| "找不到 _internal/ort/onnxruntime.dll".to_string())?
            .join("onnxruntime.dll");
        let model = crate::assets::ocr_model()?.join("rec.onnx");
        let keys = crate::assets::ocr_model()?.join("keys.txt");
        Self::new(dll, model, keys, threads)
    }

    /// dll: onnxruntime.dll；model: rec.onnx；keys: keys.txt
    pub fn new(
        dll: impl AsRef<Path>,
        model: impl AsRef<Path>,
        keys: impl AsRef<Path>,
        threads: usize,
    ) -> Result<Self, String> {
        let env = load_env(dll.as_ref())?;
        let session = Session::builder(env.as_ref())
            .map_err(|e| e.to_string())?
            .with_intra_threads(threads.max(1))
            .map_err(|e| e.to_string())?
            .with_inter_threads(1)
            .map_err(|e| e.to_string())?
            .with_optimization_level(GraphOptimizationLevel::All)
            .map_err(|e| e.to_string())?
            .commit_from_file(model)
            .map_err(|e| format!("加载 rec.onnx 失败: {e}"))?;

        // 首次推理会做图优化/内存分配，提前用空白 batch 预热掉
        let mut session = session;
        let dummy = vec![0f32; BATCH * 3 * REC_HEIGHT as usize * 16];
        if let Ok(t) = Tensor::from_array(([BATCH, 3, REC_HEIGHT as usize, 16], dummy)) {
            let _ = session.run(ort::inputs!["x" => t]);
        }

        let text = std::fs::read_to_string(keys).map_err(|e| format!("读 keys.txt 失败: {e}"))?;
        let keys: Vec<String> = text
            .lines()
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect();

        Ok(Self {
            session: Mutex::new(session),
            classes: keys.len() + 2, // blank + 字典 + 空格
            keys,
        })
    }

    /// 对三张卡名 ROI 做一次 batch 识别；返回的文本按输入顺序。
    pub fn read_rois(
        &self,
        img: &Image,
        rois: &[(f64, f64, f64, f64); BATCH],
    ) -> Result<[String; BATCH], String> {
        let mut crops: Vec<RgbImage> = Vec::with_capacity(BATCH);
        for &(x0, y0, x1, y1) in rois {
            crops.push(crop_rgb(img, x0, y0, x1, y1)?);
        }

        let mut arrs: Vec<RgbImage> = Vec::with_capacity(BATCH);
        for c in crops {
            let w = c.width();
            let h = c.height();
            let nw = ((w as f64 * REC_HEIGHT as f64 / h as f64).round() as u32)
                .clamp(1, REC_MAX_WIDTH);
            arrs.push(image::imageops::resize(
                &c,
                nw,
                REC_HEIGHT,
                FilterType::Triangle,
            ));
        }

        let max_w = arrs.iter().map(|a| a.width()).max().unwrap_or(1) as usize;
        let mut data = vec![0f32; BATCH * 3 * REC_HEIGHT as usize * max_w];
        for (bi, a) in arrs.iter().enumerate() {
            let w = a.width() as usize;
            for y in 0..REC_HEIGHT as usize {
                for x in 0..w {
                    let p = a.get_pixel(x as u32, y as u32).0;
                    for c in 0..3 {
                        let v = p[c] as f32 / 255.0;
                        let v = (v - 0.5) / 0.5;
                        let idx = ((bi * 3 + c) * REC_HEIGHT as usize + y) * max_w + x;
                        data[idx] = v;
                    }
                }
            }
        }

        let shape = [BATCH, 3, REC_HEIGHT as usize, max_w];
        let input = Tensor::from_array((shape, data)).map_err(|e| e.to_string())?;
        let mut session = self.session.lock().map_err(|e| e.to_string())?;
        let outputs = session
            .run(ort::inputs!["x" => input])
            .map_err(|e| format!("ORT 推理失败: {e}"))?;

        let (_shape, out) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;

        let classes = self.classes;
        if out.len() % (BATCH * classes) != 0 {
            return Err(format!(
                "输出尺寸不对: {}，classes={classes}",
                out.len()
            ));
        }
        let steps = out.len() / (BATCH * classes);
        let mut result: [String; BATCH] = Default::default();
        for bi in 0..BATCH {
            let mut prev = 0usize;
            let mut s = String::new();
            for t in 0..steps {
                let base = (bi * steps + t) * classes;
                let mut best = 0usize;
                let mut best_v = f32::NEG_INFINITY;
                for k in 0..classes {
                    let v = out[base + k];
                    if v > best_v {
                        best_v = v;
                        best = k;
                    }
                }
                if best != 0 && best != prev {
                    if best <= self.keys.len() {
                        s.push_str(&self.keys[best - 1]);
                    } else if best == self.keys.len() + 1 {
                        s.push(' ');
                    }
                }
                prev = best;
            }
            result[bi] = s;
        }
        Ok(result)
    }
}

fn crop_rgb(
    img: &Image,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
) -> Result<RgbImage, String> {
    if img.channels != 3 {
        return Err(format!("需要三通道 BGR，当前 {}", img.channels));
    }
    let w = img.width as f64;
    let h = img.height as f64;
    let mut x = (x0 * w) as i32;
    let mut y = (y0 * h) as i32;
    let mut rw = (x1 * w) as i32 - x;
    let mut rh = (y1 * h) as i32 - y;
    if x < 0 {
        rw += x;
        x = 0;
    }
    if y < 0 {
        rh += y;
        y = 0;
    }
    if x + rw > img.width as i32 {
        rw = img.width as i32 - x;
    }
    if y + rh > img.height as i32 {
        rh = img.height as i32 - y;
    }
    if rw <= 0 || rh <= 0 {
        return Err("ROI 尺寸非法".into());
    }

    let mut out = RgbImage::new(rw as u32, rh as u32);
    let ch = img.channels as usize;
    let iw = img.width as usize;
    for row in 0..rh as usize {
        for col in 0..rw as usize {
            let src = ((y as usize + row) * iw + (x as usize + col)) * ch;
            let b = img.data[src];
            let g = img.data[src + 1];
            let r = img.data[src + 2];
            out.put_pixel(col as u32, row as u32, Rgb([r, g, b]));
        }
    }
    Ok(out)
}
