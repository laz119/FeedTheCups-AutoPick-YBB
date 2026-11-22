//! 用**真实的** MaaFramework.dll 验证整张 FFI 表的正确性。
//!
//! 这一步不需要游戏、不需要管理员权限、也不碰任何输入：只把 164 个符号逐个解析一遍。
//! 任何一处 `argtypes`/`restype` 抄错，通常不会在解析阶段暴露，但符号名或调用约定错了会立刻报出来。

use std::ffi::CStr;
use std::path::PathBuf;

use ybb::maa_sys::MaaApi;

/// 发行包里的 Maa 二进制目录（相对本 crate 的父目录）
fn maa_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("_internal")
        .join("maa")
        .join("bin")
}

const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;

#[test]
fn framework_dll_resolves_all_symbols() {
    let dir = maa_bin();
    let dll = dir.join("MaaFramework.dll");
    if !dll.exists() {
        eprintln!("跳过：找不到 {}", dll.display());
        return;
    }

    unsafe {
        // LOAD_WITH_ALTERED_SEARCH_PATH：以 DLL 自身所在目录为搜索起点，
        // 这样它依赖的 MaaUtils / opencv_world4_maa / onnxruntime_maa 才能被找到。
        let raw = libloading::os::windows::Library::load_with_flags(
            &dll,
            LOAD_WITH_ALTERED_SEARCH_PATH,
        )
        .expect("加载 MaaFramework.dll 失败");
        let lib: libloading::Library = raw.into();

        let api = MaaApi::load(&lib).expect("有符号解析失败");

        let ver = (api.MaaVersion)();
        let ver = if ver.is_null() {
            "<null>".to_string()
        } else {
            CStr::from_ptr(ver).to_string_lossy().into_owned()
        };

        println!("MaaFramework 版本: {ver}");
        assert!(!ver.is_empty(), "版本号为空，说明加载到了错的 DLL");
    }
}
