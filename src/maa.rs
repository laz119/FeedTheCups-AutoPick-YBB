//! MaaFramework 的安全薄封装：加载库、建控制器、截图。
//!
//! 只包住第一阶段需要的那几个调用，每个句柄都有对应的 `Drop`，避免手写 FFI 时漏释放。

use std::ffi::CStr;
use std::os::raw::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::maa_sys::MaaApi;

// ---- 枚举值（出处：MaaDef.h / v5.14.3）----

/// 截图方式，按位组合。`Background` 就是 `FramePool | PrintWindow`。
pub mod screencap {
    pub const GDI: u64 = 1;
    pub const FRAME_POOL: u64 = 1 << 1;
    pub const DXGI_DESKTOP_DUP: u64 = 1 << 2;
    pub const DXGI_DESKTOP_DUP_WINDOW: u64 = 1 << 3;
    pub const PRINT_WINDOW: u64 = 1 << 4;
    pub const SCREEN_DC: u64 = 1 << 5;
    /// 原版用的三档
    pub const BACKGROUND: u64 = FRAME_POOL | PRINT_WINDOW;
    pub const FOREGROUND: u64 = DXGI_DESKTOP_DUP_WINDOW | SCREEN_DC;
}

/// 输入方式（不是位掩码，单选）。
pub mod input {
    pub const SEIZE: u64 = 1;
    pub const SEND_MESSAGE: u64 = 1 << 1;
    pub const POST_MESSAGE: u64 = 1 << 2;
    pub const INTERCEPTION: u64 = 1 << 9;
}

const STATUS_SUCCEEDED: i32 = 3000;
const OPTION_SCREENSHOT_TARGET_LONG_SIDE: i32 = 1;

/// MaaFramework 库本体
pub struct Maa {
    _lib: libloading::Library,
    pub api: MaaApi,
    pub dir: PathBuf,
}

const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;

impl Maa {
    /// 从 `dir` 加载 `MaaFramework.dll`（连带它的同目录依赖）。
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, String> {
        let dir = dir.as_ref().to_path_buf();
        let dll = dir.join("MaaFramework.dll");
        if !dll.exists() {
            return Err(format!("找不到 {}", dll.display()));
        }
        unsafe {
            let raw =
                libloading::os::windows::Library::load_with_flags(&dll, LOAD_WITH_ALTERED_SEARCH_PATH)
                    .map_err(|e| format!("加载 MaaFramework.dll 失败: {e}"))?;
            let lib: libloading::Library = raw.into();
            let api = MaaApi::load(&lib)?;
            Ok(Self {
                _lib: lib,
                api,
                dir,
            })
        }
    }

    pub fn version(&self) -> String {
        unsafe {
            let p = (self.api.MaaVersion)();
            if p.is_null() {
                return String::new();
            }
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// Win32 控制器（句柄随 `Drop` 释放）
pub struct Controller {
    maa: Arc<Maa>,
    handle: *mut c_void,
    pub screencap_method: u64,
}

impl Controller {
    pub fn create(
        maa: Arc<Maa>,
        hwnd: *mut c_void,
        screencap_method: u64,
        input_method: u64,
    ) -> Result<Self, String> {
        unsafe {
            let handle = (maa.api.MaaWin32ControllerCreate)(
                hwnd,
                screencap_method,
                input_method,
                input_method,
            );
            if handle.is_null() {
                return Err("MaaWin32ControllerCreate 返回空句柄".into());
            }
            Ok(Self {
                maa,
                handle,
                screencap_method,
            })
        }
    }

    /// 异步连接并等到结束
    pub fn connect(&self) -> Result<(), String> {
        unsafe {
            let id = (self.maa.api.MaaControllerPostConnection)(self.handle);
            let st = (self.maa.api.MaaControllerWait)(self.handle, id);
            if st != STATUS_SUCCEEDED {
                return Err(format!("连接失败，状态 {st}"));
            }
            Ok(())
        }
    }

    /// 把截图长边缩放到 `n`（原版用 1920）
    pub fn set_screenshot_long_side(&self, n: i32) -> bool {
        let mut v = n;
        unsafe {
            (self.maa.api.MaaControllerSetOption)(
                self.handle,
                OPTION_SCREENSHOT_TARGET_LONG_SIDE,
                &mut v as *mut i32 as *mut c_void,
                std::mem::size_of::<i32>() as u64,
            ) != 0
        }
    }

    /// 同步截一帧
    pub fn screencap(&self) -> Result<Image, String> {
        unsafe {
            let id = (self.maa.api.MaaControllerPostScreencap)(self.handle);
            let st = (self.maa.api.MaaControllerWait)(self.handle, id);
            if st != STATUS_SUCCEEDED {
                return Err(format!("截图失败，状态 {st}"));
            }

            let buf = (self.maa.api.MaaImageBufferCreate)();
            if buf.is_null() {
                return Err("MaaImageBufferCreate 返回空".into());
            }
            let ok = (self.maa.api.MaaControllerCachedImage)(self.handle, buf) != 0;
            let out = if !ok {
                Err("CachedImage 取图失败".to_string())
            } else {
                Image::from_buffer(&self.maa.api, buf)
            };
            (self.maa.api.MaaImageBufferDestroy)(buf);
            out
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        unsafe { (self.maa.api.MaaControllerDestroy)(self.handle) };
    }
}

/// 一帧 BGR(A) 图像
#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub channels: u32,
    pub data: Vec<u8>,
}

impl Image {
    unsafe fn from_buffer(api: &MaaApi, buf: *mut c_void) -> Result<Self, String> {
        let w = (api.MaaImageBufferWidth)(buf) as u32;
        let h = (api.MaaImageBufferHeight)(buf) as u32;
        let ch = (api.MaaImageBufferChannels)(buf) as u32;
        if w == 0 || h == 0 || ch == 0 {
            return Err(format!("图像尺寸异常 {w}x{h}x{ch}"));
        }
        let raw = (api.MaaImageBufferGetRawData)(buf) as *const u8;
        if raw.is_null() {
            return Err("GetRawData 返回空".into());
        }
        let len = (w as usize) * (h as usize) * (ch as usize);
        let data = std::slice::from_raw_parts(raw, len).to_vec();
        Ok(Self {
            width: w,
            height: h,
            channels: ch,
            data,
        })
    }

    /// 全通道平均亮度（等价 numpy 的 `arr.mean()`，原版拿它判断"是不是纯黑"）
    pub fn mean(&self) -> f64 {
        if self.data.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.data.iter().map(|&b| b as u64).sum();
        sum as f64 / self.data.len() as f64
    }

    /// Maa 给的是 BGR(A)，转成 RGB(A) 便于查看和后续处理
    pub fn to_rgb(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        let px = (self.width * self.height) as usize;
        if self.channels >= 3 {
            for i in 0..px {
                let o = i * self.channels as usize;
                out.swap(o, o + 2);
            }
        }
        out
    }

    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let buf = self.to_rgb();
        let color = match self.channels {
            3 => image::ColorType::Rgb8,
            4 => image::ColorType::Rgba8,
            1 => image::ColorType::L8,
            n => return Err(format!("不支持的通道数 {n}")),
        };
        image::save_buffer(path, &buf, self.width, self.height, color)
            .map_err(|e| format!("保存 PNG 失败: {e}"))
    }

    /// 从磁盘读一张图，转成 Maa 要求的 BGR 三通道。
    pub fn load_bgr(path: impl AsRef<Path>) -> Result<Self, String> {
        let img = image::open(path.as_ref()).map_err(|e| format!("读取图片失败: {e}"))?;
        let rgb = img.to_rgb8();
        let (w, h) = rgb.dimensions();
        let mut data = rgb.into_raw();
        // RGB -> BGR
        for px in data.chunks_exact_mut(3) {
            px.swap(0, 2);
        }
        Ok(Self {
            width: w,
            height: h,
            channels: 3,
            data,
        })
    }
}
