"""Gera assets/taskbeat.ico (16..256 px) sem dependências externas.

Arte: "squircle" em gradiente (roxo -> rosa -> laranja) com barras brancas de
equalizador sobre uma linha que lembra a barra de tarefas. Anti-aliasing por
supersampling 4x4. Cada tamanho vai como PNG dentro do .ico.

Uso: python assets/make_icon.py
"""
import os
import struct
import zlib

SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
SS = 4  # supersampling


def lerp(a, b, t):
    return tuple(a[i] + (b[i] - a[i]) * t for i in range(3))


def gradient(t):
    purple, pink, orange = (0.42, 0.25, 0.95), (0.93, 0.27, 0.62), (1.0, 0.62, 0.28)
    return lerp(purple, pink, t * 2) if t < 0.5 else lerp(pink, orange, (t - 0.5) * 2)


def in_rrect(x, y, x0, y0, x1, y1, r):
    if x < x0 or x > x1 or y < y0 or y > y1:
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def shapes(n):
    """Lista de (retângulo arredondado) brancos em coordenadas 0..1."""
    small = n <= 24
    out = []
    if small:
        heights, bw, gap, base = [0.40, 0.62, 0.30], 0.16, 0.08, 0.72
    else:
        heights, bw, gap, base = [0.34, 0.58, 0.42, 0.24], 0.11, 0.06, 0.70
    total = len(heights) * bw + (len(heights) - 1) * gap
    x = 0.5 - total / 2
    for h in heights:
        out.append((x, base - h, x + bw, base, bw / 2))
        x += bw + gap
    if not small:
        out.append((0.2, 0.77, 0.8, 0.83, 0.03))
    return out


def render(n):
    px = bytearray(n * n * 4)
    bars = shapes(n)
    radius = 0.26 if n > 24 else 0.22
    for y in range(n):
        for x in range(n):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(SS):
                for sx in range(SS):
                    u = (x + (sx + 0.5) / SS) / n
                    v = (y + (sy + 0.5) / SS) / n
                    if not in_rrect(u, v, 0.0, 0.0, 1.0, 1.0, radius):
                        continue
                    c = gradient(min(max((u + v) / 2, 0.0), 1.0))
                    for b in bars:
                        if in_rrect(u, v, *b[:4], b[4]):
                            c = (1.0, 1.0, 1.0)
                            break
                    acc[0] += c[0]
                    acc[1] += c[1]
                    acc[2] += c[2]
                    acc[3] += 1.0
            k = SS * SS
            a = acc[3] / k
            i = (y * n + x) * 4
            if acc[3] > 0:
                px[i] = int(acc[0] / acc[3] * 255 + 0.5)
                px[i + 1] = int(acc[1] / acc[3] * 255 + 0.5)
                px[i + 2] = int(acc[2] / acc[3] * 255 + 0.5)
            px[i + 3] = int(a * 255 + 0.5)
    return bytes(px)


def png(n, rgba):
    raw = b"".join(b"\x00" + rgba[y * n * 4:(y + 1) * n * 4] for y in range(n))

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    images = [(n, png(n, render(n))) for n in SIZES]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, blobs = b"", b""
    for n, data in images:
        dim = 0 if n >= 256 else n
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
        blobs += data
    with open(os.path.join(here, "taskbeat.ico"), "wb") as f:
        f.write(header + entries + blobs)
    with open(os.path.join(here, "taskbeat-256.png"), "wb") as f:
        f.write(images[-1][1])
    print("ok:", [n for n, _ in images])


if __name__ == "__main__":
    main()
