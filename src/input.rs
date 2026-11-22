//! 输入注入。
//!
//! 两条路：
//! - `SendInput`（系统 API，最简单，多数窗口化游戏能用）
//! - `Interception`（驱动级，绕开模拟按键过滤；需要已安装驱动 + interception.dll）
//!
//! 都在同一个 `Injector` trait 下，主循环不关心用的是哪条。

use std::ffi::c_void;
use std::sync::Arc;
use std::os::raw::c_int;
use std::path::Path;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};

/// 游戏里用到的四个键
#[derive(Clone, Copy, Debug)]
pub enum Key {
    Left,
    Right,
    Confirm,
    Reroll,
}

impl Key {
    /// SendInput 用的虚拟键码
    fn vk(self) -> u16 {
        match self {
            Key::Left => 0x41,    // A
            Key::Right => 0x44,   // D
            Key::Confirm => 0x20, // Space
            Key::Reroll => 0x4A,  // J
        }
    }
    /// Interception 用的扫描码（Set 1）
    fn scancode(self) -> u16 {
        match self {
            Key::Left => 0x1E,    // A
            Key::Right => 0x20,   // D
            Key::Confirm => 0x39, // Space
            Key::Reroll => 0x24,  // J
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Key::Left => "A(左)",
            Key::Right => "D(右)",
            Key::Confirm => "空格(确认)",
            Key::Reroll => "J(重抽)",
        }
    }
}

pub trait Injector {
    /// 按一下（按下 + 保持 + 抬起）
    fn tap(&self, key: Key) -> Result<(), String>;
    fn name(&self) -> &'static str;
}

// ---------------- SendInput ----------------

pub struct SendInputInjector {
    hold_ms: u64,
}

impl SendInputInjector {
    pub fn new(hold_ms: u64) -> Self {
        Self { hold_ms }
    }
}

fn make_input(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk as VIRTUAL_KEY,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

impl Injector for SendInputInjector {
    fn tap(&self, key: Key) -> Result<(), String> {
        let down = make_input(key.vk(), false);
        let up = make_input(key.vk(), true);
        unsafe {
            let n = SendInput(1, &down, std::mem::size_of::<INPUT>() as i32);
            if n != 1 {
                return Err("SendInput(按下) 被系统拒绝".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(self.hold_ms));
            SendInput(1, &up, std::mem::size_of::<INPUT>() as i32);
        }
        Ok(())
    }
    fn name(&self) -> &'static str {
        "SendInput"
    }
}

// ---------------- Interception ----------------

#[repr(C)]
#[derive(Clone, Copy)]
struct KeyboardStroke {
    code: u16,
    state: u16,
}

const KEY_DOWN: u16 = 0x00;
const KEY_UP: u16 = 0x01;
const FILTER_KEY_ALL: u16 = 0xFFFF;

type CreateCtx = unsafe extern "C" fn() -> *mut c_void;
type DestroyCtx = unsafe extern "C" fn(*mut c_void);
type SetFilter = unsafe extern "C" fn(*mut c_void, *const c_void, u16);
type Send = unsafe extern "C" fn(*mut c_void, c_int, *const KeyboardStroke, u32) -> c_int;
type IsKeyboard = unsafe extern "C" fn(c_int) -> c_int;

pub struct InterceptionInjector {
    _lib: libloading::Library,
    ctx: *mut c_void,
    send: Send,
    device: c_int,
    hold_ms: u64,
    destroy: DestroyCtx,
}

impl InterceptionInjector {
    /// `dll` 是 interception.dll 的路径（放在程序目录下即可）
    pub fn new(dll: impl AsRef<Path>, hold_ms: u64) -> Result<Self, String> {
        let dll = dll.as_ref();
        if !dll.exists() {
            return Err(format!("找不到 {}", dll.display()));
        }
        unsafe {
            let lib =
                libloading::Library::new(dll).map_err(|e| format!("加载 interception.dll 失败: {e}"))?;

            // 先把函数指针全取出来，符号对象用完即弃，之后才能把 lib 移进结构体
            let (create, destroy, set_filter, send, is_kbd) = {
                let create: libloading::Symbol<CreateCtx> =
                    lib.get(b"interception_create_context\0").map_err(|e| e.to_string())?;
                let destroy: libloading::Symbol<DestroyCtx> = lib
                    .get(b"interception_destroy_context\0")
                    .map_err(|e| e.to_string())?;
                let set_filter: libloading::Symbol<SetFilter> = lib
                    .get(b"interception_set_filter\0")
                    .map_err(|e| e.to_string())?;
                let send: libloading::Symbol<Send> =
                    lib.get(b"interception_send\0").map_err(|e| e.to_string())?;
                let is_kbd: libloading::Symbol<IsKeyboard> = lib
                    .get(b"interception_is_keyboard\0")
                    .map_err(|e| e.to_string())?;
                (*create, *destroy, *set_filter, *send, *is_kbd)
            };

            let ctx = create();
            if ctx.is_null() {
                return Err("interception_create_context 返回空（驱动没装？）".into());
            }
            // 只拦键盘，且把按下/抬起都拦下来（否则我们发出的按键会被自己再读到）
            set_filter(ctx, is_kbd as *const c_void, FILTER_KEY_ALL);

            // 找第一个键盘设备（Interception 里键盘设备号从 1 开始）
            let mut device = 0;
            for d in 1..=10 {
                if is_kbd(d) != 0 {
                    device = d;
                    break;
                }
            }
            if device == 0 {
                destroy(ctx);
                return Err("没找到键盘设备（驱动未生效，可能需要重启）".into());
            }

            Ok(Self {
                _lib: lib,
                ctx,
                send,
                device,
                hold_ms,
                destroy,
            })
        }
    }
}

impl Injector for InterceptionInjector {
    fn tap(&self, key: Key) -> Result<(), String> {
        let down = KeyboardStroke {
            code: key.scancode(),
            state: KEY_DOWN,
        };
        let up = KeyboardStroke {
            code: key.scancode(),
            state: KEY_UP,
        };
        unsafe {
            let n = (self.send)(self.ctx, self.device, &down, 1);
            if n != 1 {
                return Err(format!("interception_send(按下) 返回 {n}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(self.hold_ms));
            (self.send)(self.ctx, self.device, &up, 1);
        }
        Ok(())
    }
    fn name(&self) -> &'static str {
        "Interception"
    }
}

impl Drop for InterceptionInjector {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.ctx) };
    }
}

/// 按优先级建一个可用的注入器：Interception 优先，退回 SendInput
pub fn build(prefer_keyboard_driver: bool, hold_ms: u64) -> Box<dyn Injector> {
    if prefer_keyboard_driver {
        for p in ["interception.dll", "_internal/interception.dll"] {
            if Path::new(p).exists() {
                match InterceptionInjector::new(p, hold_ms) {
                    Ok(i) => return Box::new(i),
                    Err(e) => eprintln!("Interception 不可用（{e}），退回 SendInput"),
                }
            }
        }
        eprintln!("没找到 interception.dll，退回 SendInput");
    }
    Box::new(SendInputInjector::new(hold_ms))
}

// ---------------- 手柄（直接走 ViGEm）----------------

pub struct VigemInjector {
    pad: crate::vigem::ViGEm,
    hold_ms: u64,
}

impl VigemInjector {
    pub fn new(dll: impl AsRef<std::path::Path>, hold_ms: u64) -> Result<Self, String> {
        Ok(Self {
            pad: crate::vigem::ViGEm::new(dll)?,
            hold_ms,
        })
    }
}

impl Injector for VigemInjector {
    fn tap(&self, key: Key) -> Result<(), String> {
        let b = match key {
            Key::Left => crate::vigem::btn::DPAD_LEFT,
            Key::Right => crate::vigem::btn::DPAD_RIGHT,
            Key::Confirm => crate::vigem::btn::A,
            Key::Reroll => crate::vigem::btn::RB,
        };
        self.pad.press(b, self.hold_ms)
    }
    fn name(&self) -> &'static str {
        "虚拟手柄(Xbox360)"
    }
}

/// 输入方式
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// 键盘（Interception 优先，退回 SendInput）
    Keyboard,
    /// 虚拟手柄（ViGEm，接真手柄时用这个）
    Gamepad,
}

/// 按指定方式建注入器。
pub fn build_mode(
    mode: InputMode,
    maa: Arc<crate::maa::Maa>,
    hwnd: *mut std::os::raw::c_void,
    use_interception: bool,
    hold_ms: u64,
) -> Result<Box<dyn Injector>, String> {
    match mode {
        InputMode::Gamepad => {
            let _ = (maa, hwnd);
            let dir = crate::assets::find("_internal/maa/bin", "ViGEmClient.dll")
                .ok_or("找不到 ViGEmClient.dll")?;
            let dll = dir.join("ViGEmClient.dll");
            Ok(Box::new(VigemInjector::new(dll, hold_ms)?))
        }
        InputMode::Keyboard => Ok(build(use_interception, hold_ms)),
    }
}
