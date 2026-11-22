"""从 app256.png 生成多尺寸 app.ico（每个尺寸都是 PNG 压缩，Win10 完全支持）。"""
import os
import struct
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(HERE, "app256.png")
ICO = os.path.join(HERE, "app.ico")
SIZES = [16, 24, 32, 48, 64, 128, 256]

blobs = []
for s in SIZES:
    tmp = os.path.join(HERE, f"_s{s}.png")
    subprocess.run(
        ["ffmpeg", "-v", "error", "-i", SRC,
         "-vf", f"scale={s}:{s}:flags=lanczos", "-y", tmp],
        check=True,
    )
    with open(tmp, "rb") as f:
        blobs.append((s, f.read()))
    os.remove(tmp)

# ICONDIR
header = struct.pack("<HHH", 0, 1, len(blobs))
entries = b""
offset = 6 + 16 * len(blobs)
for s, data in blobs:
    entries += struct.pack(
        "<BBBBHHII",
        0 if s >= 256 else s,   # width  (0 表示 256)
        0 if s >= 256 else s,   # height
        0, 0, 1, 32, len(data), offset,
    )
    offset += len(data)

with open(ICO, "wb") as f:
    f.write(header + entries + b"".join(d for _, d in blobs))

print(f"生成 {ICO}  尺寸 {SIZES}  共 {os.path.getsize(ICO)} 字节")
