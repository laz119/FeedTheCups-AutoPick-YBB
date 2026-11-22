//! 资源目录定位（Maa 二进制、OCR 模型、模板等）。
//!
//! 优先环境变量，其次从当前目录和可执行文件目录往上找，这样无论是
//! 双击 exe 还是 `cargo run` 都能找到 `_internal/maa/bin`。

use std::path::PathBuf;

pub fn find(rel: &str, marker: &str) -> Option<PathBuf> {
    if let Ok(root) = std::env::var("YBB_ROOT") {
        let p = PathBuf::from(root).join(rel);
        if p.join(marker).exists() {
            return Some(p);
        }
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(d) = std::env::current_dir() {
        roots.push(d);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            roots.push(d.to_path_buf());
        }
    }
    for root in roots {
        let mut cur = Some(root.as_path());
        while let Some(d) = cur {
            let cand = d.join(rel);
            if cand.join(marker).exists() {
                return Some(cand);
            }
            cur = d.parent();
        }
    }
    None
}

pub fn maa_bin() -> Result<PathBuf, String> {
    find("_internal/maa/bin", "MaaFramework.dll")
        .ok_or_else(|| "找不到 MaaFramework.dll（程序要在解压出来的 YBB 目录下运行）".to_string())
}

pub fn ocr_model() -> Result<PathBuf, String> {
    find("model/ocr", "rec.onnx").ok_or_else(|| "找不到 model/ocr".to_string())
}
