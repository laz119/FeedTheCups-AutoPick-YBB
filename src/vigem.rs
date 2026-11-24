//! 直接调用 ViGEmClient 创建虚拟 Xbox360 手柄。
//!
//! 为什么不用 Maa 自带的 `MaaGamepadControllerCreate`：实测它创建后会报
//! `Win32ControlUnitMgr::make_input Unknown input method: 0`——该 C API 只接受
//! 「手柄类型 + 截图方式」，没有 Win32 输入方式参数，内部建输入单元时拿到 0（Null），
//! 于是按键发不出去。这里直接走 ViGEm，可控性也更好。

use std::ffi::c_void;
use std::path::Path;

/// XUSB_REPORT（XInput 手柄状态），12 字节
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XusbReport {
    buttons: u16,
    left_trigger: u8,
    right_trigger: u8,
    thumb_lx: i16,
    thumb_ly: i16,
    thumb_rx: i16,
    thumb_ry: i16,
}

// XUSB 按键位
pub mod btn {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const RB: u16 = 0x0200;
    pub const A: u16 = 0x1000;
}

type Alloc = unsafe extern "C" fn() -> *mut c_void;
type Free = unsafe extern "C" fn(*mut c_void);
type Connect = unsafe extern "C" fn(*mut c_void) -> u32;
type Disconnect = unsafe extern "C" fn(*mut c_void);
type TargetX360Alloc = unsafe extern "C" fn() -> *mut c_void;
type TargetFree = unsafe extern "C" fn(*mut c_void);
type TargetAdd = unsafe extern "C" fn(*mut c_void, *mut c_void) -> u32;
type TargetRemove = unsafe extern "C" fn(*mut c_void, *mut c_void) -> u32;
type X360Update = unsafe extern "C" fn(*mut c_void, *mut c_void, XusbReport) -> u32;

pub struct ViGEm {
    _lib: libloading::Library,
    client: *mut c_void,
    target: *mut c_void,
    connect: Connect,
    disconnect: Disconnect,
    target_add: TargetAdd,
    target_remove: TargetRemove,
    update: X360Update,
    free_client: Free,
    free_target: TargetFree,
}

impl ViGEm {
    pub fn new(dll: impl AsRef<Path>) -> Result<Self, String> {
        let dll = dll.as_ref();
        if !dll.exists() {
            return Err(format!("找不到 {}", dll.display()));
        }
        unsafe {
            // 用 LOAD_WITH_ALTERED_SEARCH_PATH，和加载 MaaFramework.dll 的方式保持一致
            let raw = libloading::os::windows::Library::load_with_flags(dll, 0x0000_0008)
                .map_err(|e| format!("加载 ViGEmClient 失败: {e}"))?;
            let lib: libloading::Library = raw.into();

            let (alloc, free_client, connect, disconnect, t_alloc, t_free, t_add, t_remove, update) = {
                let f: libloading::Symbol<Alloc> = lib.get(b"vigem_alloc\0").map_err(|e| e.to_string())?;
                let a = *f;
                let f: libloading::Symbol<Free> = lib.get(b"vigem_free\0").map_err(|e| e.to_string())?;
                let b = *f;
                let f: libloading::Symbol<Connect> = lib.get(b"vigem_connect\0").map_err(|e| e.to_string())?;
                let c = *f;
                let f: libloading::Symbol<Disconnect> =
                    lib.get(b"vigem_disconnect\0").map_err(|e| e.to_string())?;
                let d = *f;
                let f: libloading::Symbol<TargetX360Alloc> =
                    lib.get(b"vigem_target_x360_alloc\0").map_err(|e| e.to_string())?;
                let e1 = *f;
                let f: libloading::Symbol<TargetFree> =
                    lib.get(b"vigem_target_free\0").map_err(|e| e.to_string())?;
                let e2 = *f;
                let f: libloading::Symbol<TargetAdd> =
                    lib.get(b"vigem_target_add\0").map_err(|e| e.to_string())?;
                let e3 = *f;
                let f: libloading::Symbol<TargetRemove> =
                    lib.get(b"vigem_target_remove\0").map_err(|e| e.to_string())?;
                let e4 = *f;
                let f: libloading::Symbol<X360Update> =
                    lib.get(b"vigem_target_x360_update\0").map_err(|e| e.to_string())?;
                let e5 = *f;
                (a, b, c, d, e1, e2, e3, e4, e5)
            };

            let client = alloc();
            if client.is_null() {
                return Err("vigem_alloc 失败".into());
            }
            let err = connect(client);
            if err != 0 {
                free_client(client);
                return Err(format!(
                    "vigem_connect 失败（错误码 0x{err:08X}）：\
                     ViGEmBus 总线打不开。常见原因两种：①没装 ViGEmBus 驱动；\
                     ②已安装但当前进程权限不足（实测非管理员打开 \\\\.\\ViGEmBus 会返回拒绝访问），\
                     这时请以管理员身份运行"
                ));
            }

            let target = t_alloc();
            if target.is_null() {
                disconnect(client);
                free_client(client);
                return Err("vigem_target_x360_alloc 失败".into());
            }
            let err = t_add(client, target);
            if err != 0 {
                t_free(target);
                disconnect(client);
                free_client(client);
                return Err(format!("插件手柄到系统失败（错误码 {err}）"));
            }

            // 给系统一点时间完成设备枚举，否则头几次输入可能丢
            std::thread::sleep(std::time::Duration::from_millis(120));

            Ok(Self {
                _lib: lib,
                client,
                target,
                connect,
                disconnect,
                target_add: t_add,
                target_remove: t_remove,
                update,
                free_client,
                free_target: t_free,
            })
        }
    }

    /// 直接设置按键状态（不做按下/抬起配对，调用方负责）
    pub fn set_buttons(&self, buttons: u16) -> Result<(), String> {
        let rep = XusbReport {
            buttons,
            ..Default::default()
        };
        let err = unsafe { (self.update)(self.client, self.target, rep) };
        if err != 0 {
            return Err(format!("vigem_target_x360_update 失败（{err}）"));
        }
        Ok(())
    }

    /// 按住 `hold_ms` 再松开
    pub fn press(&self, buttons: u16, hold_ms: u64) -> Result<(), String> {
        self.set_buttons(buttons)?;
        std::thread::sleep(std::time::Duration::from_millis(hold_ms));
        self.set_buttons(0)?;
        Ok(())
    }
}

impl Drop for ViGEm {
    fn drop(&mut self) {
        unsafe {
            let _ = self.set_buttons(0);
            (self.target_remove)(self.client, self.target);
            (self.free_target)(self.target);
            (self.disconnect)(self.client);
            (self.free_client)(self.client);
        }
        let _ = (self.connect, self.target_add, self.update);
    }
}
