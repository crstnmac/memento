#!/usr/bin/env python3
"""Generate Memento menu-bar (tray) icons. Pure stdlib, no PIL.

Outputs 44x44 RGBA PNGs (22pt @2x) into src-tauri/icons/tray/:
  tray-idle.png, tray-paused.png, tray-attention.png  (template: black + alpha)
  tray-recording.png                                   (red, not a template)
  tray-preview.png                                     (contact sheet, not shipped)
Usage: python3 scripts/gen-tray-icons.py
"""
import os, struct, zlib

S = 44          # output size px
SS = 8          # supersampling factor
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "icons", "tray")

# ---- tweakable geometry (px, in 44px space) ----
BACK = (6.0, 6.0, 24.0, 7.0)     # x, y, size, corner radius (outline card)
FRONT = (14.0, 14.0, 24.0, 7.0)  # solid card
STROKE = 3.0                     # back-card outline width
GAP = 2.75                       # transparent gap around front card
BAR_W, BAR_H, BAR_GAP = 3.6, 11.0, 3.6   # pause bars
BADGE = (35.5, 8.5, 5.5, 2.5)    # cx, cy, radius, halo width


def rrect(px, py, r):
    x, y, s, rad = r
    cx, cy = x + s / 2, y + s / 2
    qx, qy = abs(px - cx) - (s / 2 - rad), abs(py - cy) - (s / 2 - rad)
    ox, oy = max(qx, 0), max(qy, 0)
    return (ox * ox + oy * oy) ** 0.5 + min(max(qx, qy), 0) - rad  # signed distance


def inflate(r, d):
    x, y, s, rad = r
    return (x - d, y - d, s + 2 * d, rad + d)


def glyph(px, py, paused=False, badge=False):
    back = rrect(px, py, BACK) <= 0 and rrect(px, py, inflate(BACK, -STROKE)) > 0
    back = back and rrect(px, py, inflate(FRONT, GAP)) > 0
    front = rrect(px, py, FRONT) <= 0
    on = back or front
    if paused and front:
        cx = FRONT[0] + FRONT[2] / 2
        cy = FRONT[1] + FRONT[3] * 0 + FRONT[2] / 2
        for sx in (-1, 1):
            bx = cx + sx * (BAR_GAP + BAR_W) / 2
            if abs(px - bx) <= BAR_W / 2 and abs(py - cy) <= BAR_H / 2:
                on = False
    if badge:
        bx, by, br, halo = BADGE
        d = ((px - bx) ** 2 + (py - by) ** 2) ** 0.5
        if d <= br:
            on = True
        elif d <= br + halo:
            on = False
    return on


def raster(fn):
    """fn(x, y) -> (r,g,b,a 0..1) at sample point; returns list of rows of RGBA bytes."""
    rows = []
    n = SS * SS
    for j in range(S):
        row = bytearray()
        for i in range(S):
            R = G = B = A = 0.0
            for sj in range(SS):
                for si in range(SS):
                    r, g, b, a = fn(i + (si + .5) / SS, j + (sj + .5) / SS)
                    R += r * a; G += g * a; B += b * a; A += a
            if A > 0:
                row += bytes((round(R / A), round(G / A), round(B / A), round(255 * A / n)))
            else:
                row += b"\0\0\0\0"
        rows.append(bytes(row))
    return rows


def tpl(**kw):
    return lambda x, y: (0, 0, 0, 1.0 if glyph(x, y, **kw) else 0.0)


def recording(x, y):
    d = ((x - 22) ** 2 + (y - 22) ** 2) ** 0.5
    if d > 11:
        return (0, 0, 0, 0)
    if 9.5 <= d <= 10.5:  # subtle inner highlight ring, white @35% over red
        return (255, round(255 * .35 + 59 * .65), round(255 * .35 + 48 * .65), 1.0)
    return (255, 59, 48, 1.0)


def png(rows, w, h, ctype=6):
    raw = b"".join(b"\0" + r for r in rows)
    def ch(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    return (b"\x89PNG\r\n\x1a\n" + ch(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, ctype, 0, 0, 0))
            + ch(b"IDAT", zlib.compress(raw, 9)) + ch(b"IEND", b""))


def write(name, data):
    os.makedirs(OUT, exist_ok=True)
    with open(os.path.join(OUT, name), "wb") as f:
        f.write(data)


# ---- preview helpers ----
def blend(canvas, W, ox, oy, rows, w, h, tint, scale=1, down=False):
    """Composite icon rows onto canvas (list of bytearray RGB rows). tint=None keeps colour."""
    def px(i, j):
        if down:  # 2x box downsample to 22px
            acc = [0, 0, 0, 0]
            for dj in (0, 1):
                for di in (0, 1):
                    p = rows[2 * j + dj][(2 * i + di) * 4:(2 * i + di) * 4 + 4]
                    a = p[3]
                    acc[0] += p[0] * a; acc[1] += p[1] * a; acc[2] += p[2] * a; acc[3] += a
            if acc[3] == 0:
                return (0, 0, 0, 0)
            return (acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3] / 4)
        p = rows[j][i * 4:i * 4 + 4]
        return tuple(p)
    ww, hh = (w // 2, h // 2) if down else (w, h)
    for j in range(hh):
        for i in range(ww):
            r, g, b, a = px(i, j)
            if tint is not None:
                r, g, b = tint
            a /= 255
            for sj in range(scale):
                for si in range(scale):
                    X, Y = ox + i * scale + si, oy + j * scale + sj
                    if 0 <= Y < len(canvas) and 0 <= X < W:
                        o = canvas[Y]
                        for k, v in enumerate((r, g, b)):
                            o[X * 3 + k] = round(v * a + o[X * 3 + k] * (1 - a))


def preview(icons):
    W, cell = 4 * 150 + 20, 150
    strips = [((0xEC,) * 3, (0, 0, 0), 1), ((0x2A,) * 3, (255, 255, 255), 1), ((0xEC,) * 3, (0, 0, 0), 3)]
    heights = [64, 64, 150]
    H = sum(heights)
    canvas, y0 = [], 0
    for (bg, fg, sc), hh in zip(strips, heights):
        for _ in range(hh):
            canvas.append(bytearray(bytes(bg) * W))
    y0 = 0
    for (bg, fg, sc), hh in zip(strips, heights):
        for k, (name, rows, is_tpl) in enumerate(icons):
            tint = fg if is_tpl else None
            x0 = 10 + k * cell
            if sc == 1:
                blend(canvas, W, x0, y0 + 10, rows, S, S, tint)                 # 44px native
                blend(canvas, W, x0 + 70, y0 + 21, rows, S, S, tint, down=True)  # 22px (1x)
            else:
                blend(canvas, W, x0, y0 + 9, rows, S, S, tint, scale=3)         # 3x enlargement
        y0 += hh
    return png([bytes(r) for r in canvas], W, H, ctype=2)


def main():
    icons = [
        ("tray-idle.png", raster(tpl()), True),
        ("tray-paused.png", raster(tpl(paused=True)), True),
        ("tray-attention.png", raster(tpl(badge=True)), True),
        ("tray-recording.png", raster(recording), False),
    ]
    for name, rows, _ in icons:
        write(name, png(rows, S, S))
    write("tray-preview.png", preview(icons))
    print("wrote", [n for n, _, _ in icons] + ["tray-preview.png"], "to", os.path.normpath(OUT))


if __name__ == "__main__":
    main()
