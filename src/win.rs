//! 找游戏窗口、把窗口调到前台。
//!
//! 原版用 `MaaToolkit` 找窗口；我们直接用 Win32，少一层依赖，也方便后面自己控制焦点。

use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    GetForegroundWindow, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE,
};

/// 一个候选窗口
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub hwnd: HWND,
    pub title: String,
}

struct Ctx {
    key: String,
    found: Vec<WindowInfo>,
}

/// 按标题关键字（不区分大小写、子串匹配）找所有可见窗口。
pub fn find_all(key: &str) -> Vec<WindowInfo> {
    let mut ctx = Ctx {
        key: key.to_lowercase(),
        found: Vec::new(),
    };
    unsafe {
        EnumWindows(Some(enum_proc), &mut ctx as *mut Ctx as LPARAM);
    }
    ctx.found
}

/// 找第一个匹配的窗口。
pub fn find(key: &str) -> Option<WindowInfo> {
    find_all(key).into_iter().next()
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam as *mut Ctx);

    if IsWindowVisible(hwnd) == 0 {
        return TRUE;
    }
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return TRUE;
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        return TRUE;
    }
    let title = String::from_utf16_lossy(&buf[..n as usize]);

    if title.to_lowercase().contains(&ctx.key) {
        ctx.found.push(WindowInfo { hwnd, title });
    }
    TRUE
}

/// 把窗口切到前台。
///
/// Windows 有"前台锁定"：后台进程直接 SetForegroundWindow 会被静默拒绝。
/// 可靠做法是三步：轻敲一下 ALT 解除锁定 → AttachThreadInput 共享输入队列 → 再设置。
pub fn force_foreground(hwnd: HWND) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, KEYEVENTF_KEYUP, VK_MENU,
    };
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }

        // 轻敲 ALT 解除前台锁定（这是 Windows 的既定行为，不影响游戏）
        keybd_event(VK_MENU as u8, 0, 0, 0);
        keybd_event(VK_MENU as u8, 0, KEYEVENTF_KEYUP, 0);

        let mut pid = 0u32;
        let target_thread = GetWindowThreadProcessId(hwnd, &mut pid);
        let my_thread = GetCurrentThreadId();

        let attached = if target_thread != 0 && target_thread != my_thread {
            AttachThreadInput(my_thread, target_thread, TRUE) != 0
        } else {
            false
        };

        BringWindowToTop(hwnd);
        let ok = SetForegroundWindow(hwnd) != 0;

        if attached {
            AttachThreadInput(my_thread, target_thread, 0);
        }
        ok
    }
}

/// 当前前台窗口是不是我们想要的那个
pub fn is_foreground(hwnd: HWND) -> bool {
    unsafe { GetForegroundWindow() == hwnd }
}

/// 窗口在屏幕上的矩形 (left, top, width, height)
pub fn rect(hwnd: HWND) -> (i32, i32, i32, i32) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe {
        if GetWindowRect(hwnd, &mut r) == 0 {
            return (0, 0, 0, 0);
        }
    }
    (r.left, r.top, r.right - r.left, r.bottom - r.top)
}
