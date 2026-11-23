//! OCR：加载 Maa 的 PP-OCR 模型，对指定 ROI 做识别。
//!
//! 对应原版 `Picker.ocr()`：`MaaTaskerPostRecognition("OCR", json, image_buffer)`。
//! 注意 Maa 的通道约定是 **BGR**，喂图之前必须是三通道。

use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::path::Path;
use std::sync::Arc;

use crate::maa::{Image, Maa};

const STATUS_SUCCEEDED: i32 = 3000;

/// 一次 OCR 命中的结果
#[derive(Debug, Clone)]
pub struct OcrItem {
    pub text: String,
    pub score: f64,
    pub box_: [i32; 4],
}

struct StringBuf {
    api: Arc<Maa>,
    handle: *mut c_void,
}

impl StringBuf {
    fn new(api: Arc<Maa>) -> Option<Self> {
        let handle = unsafe { (api.api.MaaStringBufferCreate)() };
        if handle.is_null() {
            None
        } else {
            Some(Self { api, handle })
        }
    }
    fn get(&self) -> String {
        unsafe {
            let p = (self.api.api.MaaStringBufferGet)(self.handle);
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }
}

impl Drop for StringBuf {
    fn drop(&mut self) {
        unsafe { (self.api.api.MaaStringBufferDestroy)(self.handle) };
    }
}

/// 资源（OCR 模型挂在这里）
pub struct Resource {
    maa: Arc<Maa>,
    handle: *mut c_void,
}

impl Resource {
    pub fn new(maa: Arc<Maa>) -> Result<Self, String> {
        let handle = unsafe { (maa.api.MaaResourceCreate)() };
        if handle.is_null() {
            return Err("MaaResourceCreate 返回空".into());
        }
        Ok(Self { maa, handle })
    }

    /// 指定推理后端。**必须在加载模型之前调用。**
    ///
    /// - ep: 0=Auto 1=CPU 2=DirectML 5=WebGPU
    /// - device: -2=CPU -1=Auto 0..=GPU 序号
    pub fn set_inference(&self, ep: i32, device: i32) -> bool {
        // MaaResOption_InferenceExecutionProvider = 2, MaaResOption_InferenceDevice = 1
        let mut ep_v = ep;
        let mut dev_v = device;
        unsafe {
            let a = (self.maa.api.MaaResourceSetOption)(
                self.handle,
                2,
                &mut ep_v as *mut i32 as *mut c_void,
                std::mem::size_of::<i32>() as u64,
            );
            let b = (self.maa.api.MaaResourceSetOption)(
                self.handle,
                1,
                &mut dev_v as *mut i32 as *mut c_void,
                std::mem::size_of::<i32>() as u64,
            );
            a != 0 && b != 0
        }
    }

    /// 加载 PP-OCR 模型目录（原版的 `model/ocr`）
    pub fn post_ocr_model(&self, dir: impl AsRef<Path>) -> Result<(), String> {
        let dir = dir.as_ref();
        if !dir.is_dir() {
            return Err(format!("模型目录不存在: {}", dir.display()));
        }
        let c = CString::new(dir.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        unsafe {
            let id = (self.maa.api.MaaResourcePostOcrModel)(self.handle, c.as_ptr());
            if id == 0 {
                return Err("PostOcrModel 返回无效 id".into());
            }
            let st = (self.maa.api.MaaResourceWait)(self.handle, id);
            if st != STATUS_SUCCEEDED {
                return Err(format!("加载 OCR 模型失败，状态 {st}"));
            }
            if (self.maa.api.MaaResourceLoaded)(self.handle) == 0 {
                return Err("模型未就绪".into());
            }
        }
        Ok(())
    }
}

impl Drop for Resource {
    fn drop(&mut self) {
        unsafe { (self.maa.api.MaaResourceDestroy)(self.handle) };
    }
}

/// 执行器：识别都走这里
pub struct Tasker {
    maa: Arc<Maa>,
    handle: *mut c_void,
}

impl Tasker {
    pub fn new(maa: Arc<Maa>) -> Result<Self, String> {
        let handle = unsafe { (maa.api.MaaTaskerCreate)() };
        if handle.is_null() {
            return Err("MaaTaskerCreate 返回空".into());
        }
        Ok(Self { maa, handle })
    }

    pub fn bind_resource(&self, res: &Resource) -> Result<(), String> {
        unsafe {
            if (self.maa.api.MaaTaskerBindResource)(self.handle, res.handle) == 0 {
                return Err("BindResource 失败".into());
            }
        }
        Ok(())
    }

    /// 对一帧图的指定 ROI 做 OCR。`frac` 是相对坐标 (x0, y0, x1, y1)。
    pub fn ocr_roi(
        &self,
        img: &Image,
        frac: (f64, f64, f64, f64),
    ) -> Result<Vec<OcrItem>, String> {
        self.ocr_roi_ex(img, frac, false)
    }

    /// `only_rec = true` 时跳过文字检测模型，直接在给定 ROI 上跑识别。
    ///
    /// 送进 MAA 之前先把 ROI 从整帧里裁出来：`SetRawData` 是整块内存拷贝，
    /// 传 1920×1080 全图每次约 6MB，三张卡一轮就是 18MB；裁成卡名框后只剩几百 KB。
    pub fn ocr_roi_ex(
        &self,
        img: &Image,
        frac: (f64, f64, f64, f64),
        only_rec: bool,
    ) -> Result<Vec<OcrItem>, String> {
        if img.channels != 3 {
            return Err(format!("OCR 需要三通道 BGR，当前 {}", img.channels));
        }
        let (x0, y0, x1, y1) = frac;
        let w = img.width as f64;
        let h = img.height as f64;
        let mut x = (x0 * w) as i32;
        let mut y = (y0 * h) as i32;
        let mut rw = (x1 * w) as i32 - x;
        let mut rh = (y1 * h) as i32 - y;
        // 夹到图像范围内，越界直接报错而不是悄悄读脏内存
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

        // 只拷 ROI 的像素行，避免整帧大拷贝
        let ch = img.channels as usize;
        let iw = img.width as usize;
        let row_bytes = rw as usize * ch;
        let mut roi_data = Vec::with_capacity(row_bytes * rh as usize);
        for row in 0..rh as usize {
            let start = ((y as usize + row) * iw + x as usize) * ch;
            roi_data.extend_from_slice(&img.data[start..start + row_bytes]);
        }

        // 已经裁过图，roi 从 (0,0) 开始
        let json = format!(
            r#"{{"expected":[],"roi":[0,0,{rw},{rh}],"roi_offset":[0,0,0,0],"threshold":0.3,"replace":[],"order_by":"Horizontal","index":0,"only_rec":{only_rec},"model":"","color_filter":""}}"#
        );

        unsafe {
            let buf = (self.maa.api.MaaImageBufferCreate)();
            if buf.is_null() {
                return Err("MaaImageBufferCreate 返回空".into());
            }
            // 16 == OpenCV 的 CV_8UC3，与原版一致
            let ok = (self.maa.api.MaaImageBufferSetRawData)(
                buf,
                roi_data.as_ptr() as *mut c_void,
                rw,
                rh,
                16,
            );
            if ok == 0 {
                (self.maa.api.MaaImageBufferDestroy)(buf);
                return Err("SetRawData 失败".into());
            }

            let ty = CString::new("OCR").unwrap();
            let param = CString::new(json).unwrap();
            let task_id =
                (self.maa.api.MaaTaskerPostRecognition)(self.handle, ty.as_ptr(), param.as_ptr(), buf);
            let st = (self.maa.api.MaaTaskerWait)(self.handle, task_id);

            let out = if st != STATUS_SUCCEEDED {
                Err(format!("识别失败，状态 {st}"))
            } else {
                // 结果框是相对裁剪图的，补回原图偏移，焦点比较还要用
                self.read_results(task_id).map(|mut items| {
                    for it in &mut items {
                        it.box_[0] += x;
                        it.box_[1] += y;
                    }
                    items
                })
            };
            (self.maa.api.MaaImageBufferDestroy)(buf);
            out
        }
    }

    unsafe fn read_results(&self, task_id: i64) -> Result<Vec<OcrItem>, String> {
        let api = &self.maa.api;
        let entry = StringBuf::new(self.maa.clone()).ok_or("创建 StringBuffer 失败")?;

        // 第一次调用只拿数量（与原版一样，节点列表指针传 null）
        let mut size: u64 = 0;
        let mut status: i32 = 0;
        if (api.MaaTaskerGetTaskDetail)(
            self.handle,
            task_id,
            entry.handle,
            std::ptr::null_mut(),
            &mut size,
            &mut status,
        ) == 0
        {
            return Err("GetTaskDetail(探测数量) 失败".into());
        }
        if size == 0 {
            return Ok(Vec::new());
        }

        let mut node_ids = vec![0i64; size as usize];
        if (api.MaaTaskerGetTaskDetail)(
            self.handle,
            task_id,
            entry.handle,
            node_ids.as_mut_ptr(),
            &mut size,
            &mut status,
        ) == 0
        {
            return Err("GetTaskDetail(取节点) 失败".into());
        }

        // 任务 → 节点 → 识别
        let name_buf = StringBuf::new(self.maa.clone()).ok_or("创建 StringBuffer 失败")?;
        let mut reco_id: i64 = 0;
        let mut action_id: i64 = 0;
        let mut completed: u8 = 0;
        if (api.MaaTaskerGetNodeDetail)(
            self.handle,
            node_ids[0],
            name_buf.handle,
            &mut reco_id,
            &mut action_id,
            &mut completed,
        ) == 0
        {
            return Err("GetNodeDetail 失败".into());
        }
        if reco_id == 0 {
            return Ok(Vec::new());
        }

        let algo = StringBuf::new(self.maa.clone()).ok_or("创建 StringBuffer 失败")?;
        let detail = StringBuf::new(self.maa.clone()).ok_or("创建 StringBuffer 失败")?;
        let mut hit: u8 = 0;
        let rect = (api.MaaRectCreate)();
        let raw = (api.MaaImageBufferCreate)();
        let draws = (api.MaaImageListBufferCreate)();

        let ret = (api.MaaTaskerGetRecognitionDetail)(
            self.handle,
            reco_id,
            name_buf.handle,
            algo.handle,
            &mut hit,
            rect,
            detail.handle,
            raw,
            draws,
        );
        let json = if ret != 0 { detail.get() } else { String::new() };

        (api.MaaRectDestroy)(rect);
        (api.MaaImageBufferDestroy)(raw);
        (api.MaaImageListBufferDestroy)(draws);

        Ok(parse_ocr_detail(&json))
    }
}

impl Drop for Tasker {
    fn drop(&mut self) {
        unsafe { (self.maa.api.MaaTaskerDestroy)(self.handle) };
    }
}

/// 从 `MaaTaskerGetRecognitionDetail` 的 JSON 里取出全部 OCR 结果。
///
/// 结构大致为 `{"all":[{"text":"永生花","box":[x,y,w,h],"score":0.98}, ...], "filtered":[...], "best":{...}}`
fn parse_ocr_detail(json: &str) -> Vec<OcrItem> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(arr) = v.get("all").and_then(|a| a.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|it| {
            let text = it.get("text")?.as_str()?.to_string();
            let score = it
                .get("score")
                .and_then(|s| s.as_f64())
                .or_else(|| it.get("cls_score").and_then(|s| s.as_f64()))
                .unwrap_or(0.0);
            let mut box_ = [0i32; 4];
            if let Some(b) = it.get("box").and_then(|b| b.as_array()) {
                for (i, x) in b.iter().take(4).enumerate() {
                    box_[i] = x.as_f64().unwrap_or(0.0) as i32;
                }
            }
            Some(OcrItem { text, score, box_ })
        })
        .collect()
}
