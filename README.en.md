# Feed The Cups Auto-Picker (YBB)

An automatic card picker for the "Metro" mode of *Feed The Cups* (杯杯倒满). Windows desktop app built with Rust + egui.

It reads a preset queue, recognizes the three cards on screen, moves the focus, and confirms — with automatic reroll. Picks a card in under 1 second, roughly 3× faster than doing it by hand.

**[中文](README.md) · [English](README.en.md)**

## Features

- **Fast**: no fixed sleep. Polls screen pixels every 8 ms and proceeds the moment the picture settles
  - target already focused: ~0.6–0.9 s
  - move 1 slot: ~0.85 s
  - move 2 slots: ~1.2–1.5 s
  - wait for new cards after reroll: ~0.2 s
- **Template-matched focus detection**: 12 ms per result, ~20× faster than OCR, falls back to OCR automatically so it never picks wrong
- **Portable**: no registry writes, no runtime install, move the folder anywhere
- **Safe**: stops instead of guessing when focus can't be read, so it never misclicks

## Requirements

- Windows 10 / 11 x64
- Game in **windowed** or **borderless windowed** mode (exclusive fullscreen can't be captured)
- First-time setup needs the [Interception](https://github.com/oblitum/Interception) keyboard driver, then reboot

## Quick Start

Download `YBB-v1.0.zip` from [Releases](../../releases), extract, then:

1. Double-click `安装驱动.bat` (accept UAC), reboot
2. Double-click `检查环境.bat` to verify everything is ready
3. Double-click `YBB.exe`, paste your queue, click "开始" (Start)

Full guide: `使用说明.txt` inside the archive.

## Build from Source

Requires the Rust toolchain plus MinGW (`windres` is used to embed the exe icon).

```bash
cd rs
cargo build --release
```

Output in `target/release/`: `YBB.exe` (GUI), `ybb-cli.exe` (CLI).

> If the source path contains non-ASCII characters or spaces, MinGW's gcc can't read
> the path at link time. Set a pure-ASCII target dir in `rs/.cargo/config.toml`:
>
> ```toml
> [build]
> target-dir = "D:/rust-target"
> ```

**Note**: runtime dependencies are not in the repo. A build needs these alongside it:

| Path | Contents |
|---|---|
| `_internal/maa/bin/` | [MaaFramework](https://github.com/MaaXYZ/MaaFramework) DLLs (screencap, OCR inference) |
| `model/ocr/` | PP-OCR model |
| `interception.dll` | keyboard injection library (from the Interception release) |
| `focus_template.png` | focus bubble template |

Copy these straight from the release archive.

Resource lookup is relative by default: current dir, exe dir, then parent dirs.
To put resources elsewhere, set `YBB_ASSET_ROOT` to the resource root.

## CLI

```
ybb-cli.exe selftest                   environment check
ybb-cli.exe run [queue] --go           pick cards per queue
ybb-cli.exe dry                        recognize only, no input
ybb-cli.exe probe                      screencap method check
ybb-cli.exe parse "cards"              validate separators
ybb-cli.exe focus <png>                read cards + detect focus
ybb-cli.exe grab <dir> [frames] [ms]   burst capture
ybb-cli.exe windows                    list visible windows
ybb-cli.exe --version                  version & author
ybb-cli.exe --license                  print MIT license
```

Queue separators: `→` `👉` newline space comma.

## Layout

```
rs/
├─ build.rs              embeds exe icon
├─ assets/               ico/png icons and generation script
└─ src/
   ├─ gui.rs             GUI (egui)
   ├─ engine.rs          main picking flow
   ├─ picker.rs          card recognition / focus detection / settle
   ├─ bubble.rs          focus bubble template matching
   ├─ fingerprint.rs     card-name fingerprint (experimental, unused)
   ├─ input.rs           keyboard injection (Interception / SendInput) + gamepad
   ├─ vigem.rs           virtual Xbox 360 gamepad
   ├─ xinput.rs          physical gamepad detection
   ├─ maa.rs / ocr.rs    MaaFramework bindings and OCR
   ├─ win.rs             window find & bring-to-front
   ├─ assets.rs          resource directory lookup
   ├─ queue.rs           queue parsing
   └─ score.rs           card-name similarity
```

## Third-party Components

- [MaaFramework](https://github.com/MaaXYZ/MaaFramework) — screencap & OCR inference (LGPL-3.0)
- [Interception](https://github.com/oblitum/Interception) — driver-level keyboard injection (LGPL-3.0)
- [ViGEmBus / ViGEmClient](https://github.com/nefarius/ViGEmBus) — virtual gamepad (BSD-3-Clause)
- [egui / eframe](https://github.com/emilk/egui) — GUI (MIT / Apache-2.0)
- PP-OCR model — text recognition

Third-party binaries in the release archive follow their own licenses; do not redistribute them separately.

## Disclaimer

For personal learning and automation research only. You assume all consequences (including in-game loss and account risk) of using this tool.

## License

MIT. Copyright (c) 2026 歪歪子 <1443423104@qq.com>. You may use, copy, modify and redistribute freely, provided the copyright notice is retained. See [LICENSE](LICENSE).

---

**作者 / Author**: 歪歪子 · 1443423104@qq.com
