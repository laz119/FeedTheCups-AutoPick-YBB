//! 把 assets/app.ico 编成 Windows 资源，链进 exe。
//! 这样资源管理器 / 任务栏显示的就是 YBB 自己的猫图标。

use std::path::{Path, PathBuf};
use std::process::Command;

fn find_windres() -> Option<PathBuf> {
    // 1. PATH 里直接有
    if Command::new("windres").arg("--version").output().is_ok() {
        return Some(PathBuf::from("windres"));
    }
    // 2. 和 gcc 放在一起（Rust 的 gnu 工具链通常是这个布局）
    if let Ok(out) = Command::new("gcc").arg("-print-prog-name=windres").output() {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let pb = PathBuf::from(&p);
        if !p.is_empty() && p != "windres" && pb.exists() {
            return Some(pb);
        }
    }
    // 3. 常见安装位置
    for p in [
        r"C:\msys64\mingw64\bin\windres.exe",
        r"C:\mingw64\bin\windres.exe",
        r"C:\Program Files\mingw64\bin\windres.exe",
    ] {
        if Path::new(p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let res = out_dir.join("app.res");

    let Some(windres) = find_windres() else {
        println!("cargo:warning=没找到 windres，exe 将没有自定义图标");
        return;
    };

    let status = Command::new(&windres)
        .arg("assets/app.rc")
        .arg("-O")
        .arg("coff")
        .arg("-o")
        .arg(&res)
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg-bins={}", res.display());
        }
        _ => println!("cargo:warning=windres 执行失败，exe 将没有自定义图标"),
    }
}
