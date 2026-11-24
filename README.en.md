<div align="center">

# 🥤 Feed The Cups Auto Card Picker (YBB)

Automatic card picker for the Metro mode of *Feed The Cups* · Windows desktop app · Rust + egui

**Under 1 second per pick, 3× faster than manual** · Portable · Auto reroll

[中文](README.md) · [English](README.en.md) · [Download](#download) · [License](LICENSE)

</div>

---

## What it is

YBB reads the three cards on screen, compares them with a preset queue, moves the focus and confirms the pick. If the target card is not there, it rerolls automatically. Instead of sleeping for a fixed time, it watches the screen and continues as soon as the animation settles.

## Highlights

- **Fast**: no fixed sleeps; the screen is polled every 8 ms and the next step starts as soon as it settles
  - Target already focused: about 0.6–0.9 s
  - One slot away: about 0.85 s
  - Two slots away: about 1.2–1.5 s
  - Each reroll: about 0.2 s for the new cards
- **Template-based focus detection**: takes about 12 ms, roughly 20× faster than OCR; if the template fails it falls back to OCR instead of guessing
- **Portable**: no registry writes, no runtime install, no background service — move the folder anywhere
- **Safe by default**: if the focus cannot be read, it stops instead of pressing keys blindly

## Requirements

- Windows 10 / 11 x64
- The game must run in **Windowed** or **Borderless** mode (exclusive fullscreen cannot be captured)
- Install the [Interception](https://github.com/oblitum/Interception) keyboard driver once, then reboot. The driver is already bundled in Releases — just follow the next section, no separate download needed.

<a id="download"></a>

## Getting started (three steps)

Download `YBB-v1.1.zip` from [Releases](../../releases), extract it, then:

1. Run `安装驱动.bat` (click "Yes" on the UAC prompt) and reboot.
2. Run `检查环境.bat` and confirm everything is ready.
3. Run `YBB.exe`, paste your card queue and press **开始** (Start).


<img width="225" height="225" alt="image" src="https://github.com/user-attachments/assets/b649fc03-936c-4d15-bca0-f7358c1ef6e5" />


## Build from source (not needed for normal use)

Besides the Rust toolchain you need MinGW (`windres` is used to embed the exe icon).

```bash
cd rs
cargo build --release
```

Outputs in `target/release/`: `YBB.exe` (GUI) and `ybb-cli.exe` (CLI).

> If the source path contains non-ASCII characters or spaces, MinGW's gcc may fail at link time.
> Point Cargo at an ASCII-only target directory in `rs/.cargo/config.toml`:
>
> ```toml
> [build]
> target-dir = "D:/rust-target"
> ```

**Note**: the repository does not include runtime dependencies. A built binary needs:

| Path | Contents |
|---|---|
| `_internal/maa/bin/` | [MaaFramework](https://github.com/MaaXYZ/MaaFramework) DLLs (capture and OCR) |
| `model/ocr/` | PP-OCR model |
| `interception.dll` | Keyboard injection library, from the Interception release |
| `focus_template.png` | Focus bubble template |

All of these are included in the release package.

Resources are looked up relative to the current directory, the executable and its parents.
Set `YBB_ROOT` to override the resource root.

## Command line

```
ybb-cli.exe selftest                   Environment check
ybb-cli.exe run [queue] --go           Pick cards from a queue
ybb-cli.exe dry                        Recognise only, no input
ybb-cli.exe probe                      Test screenshot methods
ybb-cli.exe parse "cards"              Validate queue separators
ybb-cli.exe focus <png>                Read cards and focus from an image
ybb-cli.exe grab <dir> [frames] [gap]  Burst-capture screenshots
ybb-cli.exe windows                    List visible windows
ybb-cli.exe --version                  Version and author
ybb-cli.exe --license                  Print the full MIT licence
```

Queue separators can be `→` `👉` newlines spaces or commas.

## Project layout

```
rs/
├─ build.rs              Embeds the exe icon
├─ assets/               ico/png icons and build scripts
└─ src/
   ├─ gui.rs             GUI (egui)
   ├─ engine.rs          Main picking loop
   ├─ picker.rs          Card reading / focus / stability
   ├─ bubble.rs          Focus bubble template matching
   ├─ fingerprint.rs     Card fingerprints (experimental, currently unused)
   ├─ input.rs           Keyboard injection (Interception / SendInput) + gamepad
   ├─ vigem.rs           Virtual Xbox 360 controller
   ├─ xinput.rs          Physical controller detection
   ├─ maa.rs / ocr.rs    MaaFramework bindings and OCR
   ├─ win.rs             Window lookup and foreground handling
   ├─ assets.rs          Resource directory lookup
   ├─ queue.rs           Queue parsing
   └─ score.rs           Card-name similarity
```

## Third-party components

- [MaaFramework](https://github.com/MaaXYZ/MaaFramework) — capture and OCR inference (LGPL-3.0)
- [Interception](https://github.com/oblitum/Interception) — driver-level keyboard injection (LGPL-3.0)
- [ViGEmBus / ViGEmClient](https://github.com/nefarius/ViGEmBus) — virtual gamepad (BSD-3-Clause)
- [egui / eframe](https://github.com/emilk/egui) — GUI (MIT / Apache-2.0)
- PP-OCR model — text recognition

Third-party binaries in the release keep their own licences; do not redistribute them separately.

## Disclaimer

For personal learning and automation research only. You are responsible for any consequences of using this tool, including in-game losses or account risk.

## License

MIT. Copyright © 2026 歪歪子 <1443423104@qq.com>. You may use, copy, modify and redistribute it freely, provided the copyright notice is retained. See [LICENSE](LICENSE).

---

**Author**: 歪歪子 · 1443423104@qq.com
**中文**: [README.md](README.md)
