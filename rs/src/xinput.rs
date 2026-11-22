//! XInput 手柄探测。
//!
//! 用途有两个：
//! 1. 告诉用户「有没有实体手柄连着」以及「它是不是正在发输入」
//! 2. 虚拟手柄建好之后，确认它有没有排到 1P——杯杯倒满只认 1P，
//!    如果实体手柄占着 1P，虚拟手柄按什么都没用（原版 MBB 就是卡在这里）。

use std::ffi::c_void;
use std::sync::OnceLock;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XGamepad {
    buttons: u16,
    lt: u8,
    rt: u8,
    lx: i16,
    ly: i16,
    rx: i16,
    ry: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XState {
    packet: u32,
    gamepad: XGamepad,
}

type GetState = unsafe extern "C" fn(u32, *mut XState) -> u32;

const ERROR_SUCCESS: u32 = 0;
const STICK_DEADZONE: i16 = 8000;

fn get_state_fn() -> Option<GetState> {
    static FN: OnceLock<Option<usize>> = OnceLock::new();
    let p = FN.get_or_init(|| unsafe {
        // 依次尝试常见版本，哪个能加载上用哪个
        for name in ["xinput1_4.dll", "xinput1_3.dll", "xinput9_1_0.dll"] {
            if let Ok(lib) = libloading::Library::new(name) {
                if let Ok(sym) = lib.get::<GetState>(b"XInputGetState\0") {
                    let f = *sym;
                    // 故意泄漏：进程生命周期内一直用得上
                    std::mem::forget(lib);
                    return Some(f as usize);
                }
            }
        }
        None
    });
    p.map(|x| unsafe { std::mem::transmute::<usize, GetState>(x) })
}

/// 一个手柄槽位
pub struct Pad {
    /// 0 = 1P，1 = 2P ...
    pub slot: u32,
    /// 是否正在发输入（摇杆偏离 / 按键按下 / 扳机按下）
    pub active: bool,
}

/// 扫描 1P~4P
pub fn scan() -> Vec<Pad> {
    let Some(get) = get_state_fn() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for slot in 0..4u32 {
        let mut st = XState::default();
        let ret = unsafe { get(slot, &mut st) };
        if ret != ERROR_SUCCESS {
            continue;
        }
        let g = st.gamepad;
        let active = g.buttons != 0
            || g.lt > 30
            || g.rt > 30
            || g.lx.abs() > STICK_DEADZONE
            || g.ly.abs() > STICK_DEADZONE
            || g.rx.abs() > STICK_DEADZONE
            || g.ry.abs() > STICK_DEADZONE;
        out.push(Pad { slot, active });
    }
    out
}

/// 一句话描述当前手柄状况，没有手柄返回 None
pub fn describe() -> Option<String> {
    let pads = scan();
    if pads.is_empty() {
        return None;
    }
    let mut s = String::new();
    for p in &pads {
        if !s.is_empty() {
            s.push('、');
        }
        s.push_str(&format!(
            "{}P{}",
            p.slot + 1,
            if p.active { "(正在发输入)" } else { "" }
        ));
    }
    Some(s)
}

// 避免未使用告警：给外部保留一个显式探测入口
pub fn has_any() -> bool {
    !scan().is_empty()
}

#[allow(dead_code)]
fn _unused(_: *mut c_void) {}
