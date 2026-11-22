<div align="center">

# 🥤 Feed The Cups 自动选卡（YBB）

《杯杯倒满》(Feed The Cups) 地铁模式自动选卡工具 · Windows 桌面程序 · Rust + egui

**选一张卡不到 1 秒，比手动快 3 倍** · 绿色免安装 · 支持自动重抽

[中文](README.md) · [English](README.en.md) · [下载](#直接使用) · [License](LICENSE)

</div>

---

## 简介

按预设队列自动识别当前三张卡、移动焦点、确认，支持自动重抽。用画面侦测代替固定等待，动画一结束立刻继续，所以快。
**手柄要用管理员模式开启**

## 特点

- **快**：不用固定 sleep 死等，8ms 轮询画面像素变化，画面一稳定立刻继续
  - 目标在焦点位约 0.6–0.9 秒
  - 推 1 格约 0.85 秒
  - 推 2 格约 1.2–1.5 秒
  - 每次重抽等新卡约 0.2 秒
- **判焦点用模板匹配**：12ms 出结果，比 OCR 快约 20 倍，失败自动退回 OCR 不会错
- **绿色**：不写注册表、不装运行库，整个目录可随便挪
- **安全**：读不到焦点直接停下，不会瞎按导致选错卡

## 运行要求

- Windows 10 / 11 x64
- 游戏切成**窗口**或**无边框窗口**（独占全屏截不到画面）
- 首次使用需安装 [Interception](https://github.com/oblitum/Interception) 键盘驱动，装完重启

## 直接使用

到 [Releases](../../releases) 下载 `YBB-v1.0.zip`，解压后：

1. 双击 `安装驱动.bat`（UAC 点「是」），重启电脑
2. 双击 `检查环境.bat` 确认各项就绪
3. 双击 `YBB.exe`，贴入队列，点「开始」

## 从源码构建

除 Rust 工具链外还需要 MinGW（`windres` 用来嵌 exe 图标）。

```bash
cd rs
cargo build --release
```

产物在 `target/release/`：`YBB.exe`（图形界面）、`ybb-cli.exe`（命令行）。

> 如果源码路径含中文或空格，MinGW 的 gcc 在链接阶段会读不了路径。这时在
> `rs/.cargo/config.toml` 里设一个纯 ASCII 的 target 目录即可：
>
> ```toml
> [build]
> target-dir = "D:/rust-target"
> ```

**注意**：仓库里不含运行时依赖，构建产物需要配套以下文件才能运行：

| 路径 | 内容 |
|---|---|
| `_internal/maa/bin/` | [MaaFramework](https://github.com/MaaXYZ/MaaFramework) 的 DLL（截图、OCR 推理） |
| `model/ocr/` | PP-OCR 模型 |
| `interception.dll` | 键盘注入库，可从 Interception 发布包取 |
| `focus_template.png` | 焦点气泡模板 |

这些都可以从 Releases 里的发行包直接复制。

默认按相对路径查找：当前目录、可执行文件所在目录、逐级父目录。
如果想把资源放在别处，设环境变量 `YBB_ASSET_ROOT` 指向资源根目录即可。

## 命令行

```
ybb-cli.exe selftest                   环境自检
ybb-cli.exe run [队列] --go            按队列选卡
ybb-cli.exe dry                        只识别不按键
ybb-cli.exe probe                      截图方式体检
ybb-cli.exe parse "卡片串"              验证分隔符
ybb-cli.exe focus <png>                读卡名 + 判焦点
ybb-cli.exe grab <目录> [帧数] [间隔]   连拍
ybb-cli.exe windows                    列出可见窗口
ybb-cli.exe --version                  版本与作者
ybb-cli.exe --license                  打印 MIT 许可证全文
```

队列分隔符支持 `→` `👉` 换行 空格 逗号。

## 结构

```
rs/
├─ build.rs              嵌 exe 图标
├─ assets/               ico/png 图标与生成脚本
└─ src/
   ├─ gui.rs             图形界面（egui）
   ├─ engine.rs          选卡主流程
   ├─ picker.rs          认卡 / 判焦点 / 判稳
   ├─ bubble.rs          焦点气泡模板匹配
   ├─ fingerprint.rs     卡名指纹（实验性，当前未启用）
   ├─ input.rs           键盘注入（Interception / SendInput）+ 手柄
   ├─ vigem.rs           虚拟 Xbox360 手柄
   ├─ xinput.rs          实体手柄探测
   ├─ maa.rs / ocr.rs    MaaFramework 绑定与 OCR
   ├─ win.rs             窗口查找与前置
   ├─ assets.rs          资源目录定位
   ├─ queue.rs           队列解析
   └─ score.rs           卡名相似度
```

## 第三方组件

- [MaaFramework](https://github.com/MaaXYZ/MaaFramework) — 截图与 OCR 推理（LGPL-3.0）
- [Interception](https://github.com/oblitum/Interception) — 驱动级键盘注入（LGPL-3.0）
- [ViGEmBus / ViGEmClient](https://github.com/nefarius/ViGEmBus) — 虚拟手柄（BSD-3-Clause）
- [egui / eframe](https://github.com/emilk/egui) — 图形界面（MIT / Apache-2.0）
- PP-OCR 模型 — 文字识别

发行包里的第三方二进制遵循各自许可证，请勿单独再分发。

## 免责声明

仅供个人学习与自动化技术研究。使用本工具产生的一切后果（包括但不限于游戏内损失、账号风险）由使用者自行承担。

## 开源协议

MIT。版权所有 © 2026 歪歪子 <1443423104@qq.com>。可自由使用、复制、修改、二次分发，但须保留版权声明。详见 [LICENSE](LICENSE)。

---

**作者**：歪歪子 · 1443423104@qq.com
**English**: [README.en.md](README.en.md)
