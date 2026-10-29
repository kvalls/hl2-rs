"""Small dependency-free RGB reader for the game's non-interlaced PNG captures."""
from __future__ import annotations
from pathlib import Path
import struct
import zlib


def png_rgb(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"not a PNG: {path}")
    offset, compressed = 8, bytearray()
    width = height = channels = 0
    while offset < len(data):
        length = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4:offset + 8]
        chunk = data[offset + 8:offset + 8 + length]
        offset += length + 12
        if kind == b"IHDR":
            width, height, bits, colour, compression, filtering, interlace = struct.unpack(">IIBBBBB", chunk)
            if bits != 8 or colour not in (2, 6) or compression or filtering or interlace:
                raise ValueError(f"unsupported PNG capture format: {path}")
            channels = 3 if colour == 2 else 4
        elif kind == b"IDAT":
            compressed.extend(chunk)
        elif kind == b"IEND":
            break
    if not width or not height or not channels:
        raise ValueError(f"missing PNG header: {path}")
    raw = zlib.decompress(compressed)
    stride = width * channels
    if len(raw) != height * (stride + 1):
        raise ValueError(f"unexpected PNG scanline size: {path}")
    previous, rgb = bytearray(stride), bytearray()
    for y in range(height):
        start = y * (stride + 1)
        method = raw[start]
        row = bytearray(raw[start + 1:start + 1 + stride])
        if method > 4:
            raise ValueError(f"unsupported PNG filter: {method}")
        for x in range(stride):
            left = row[x - channels] if x >= channels else 0
            above = previous[x]
            upper_left = previous[x - channels] if x >= channels else 0
            if method == 1:
                prediction = left
            elif method == 2:
                prediction = above
            elif method == 3:
                prediction = (left + above) // 2
            elif method == 4:
                p = left + above - upper_left
                distances = (abs(p - left), abs(p - above), abs(p - upper_left))
                prediction = (left, above, upper_left)[distances.index(min(distances))]
            else:
                prediction = 0
            row[x] = (row[x] + prediction) & 255
        if channels == 3:
            rgb.extend(row)
        else:
            for x in range(0, stride, 4):
                rgb.extend(row[x:x + 3])
        previous = row
    return width, height, bytes(rgb)


def pixel_changes(before: Path, after: Path, threshold: int = 2,
                  region: tuple[int, int, int, int] | None = None) -> dict[str, int | list[int]]:
    """Counts RGB changes in an optional rectangle; returns their inclusive bounds."""
    width, height, a = png_rgb(before)
    w, h, b = png_rgb(after)
    if (width, height) != (w, h):
        raise ValueError("capture dimensions differ")
    changed = darker = brighter = maximum = 0
    bounds = [width, height, -1, -1]
    x0, y0, x1, y1 = region or (0, 0, width, height)
    if not (0 <= x0 < x1 <= width and 0 <= y0 < y1 <= height):
        raise ValueError("comparison region is outside the capture")
    pixels = (y * width + x for y in range(y0, y1) for x in range(x0, x1))
    for pixel in pixels:
        i = pixel * 3
        differences = [b[i + c] - a[i + c] for c in range(3)]
        delta = max(abs(d) for d in differences)
        maximum = max(maximum, delta)
        if delta > threshold:
            x, y = (i // 3) % width, (i // 3) // width
            bounds = [min(bounds[0], x), min(bounds[1], y), max(bounds[2], x), max(bounds[3], y)]
        changed += delta > threshold
        darker += min(differences) < -threshold
        brighter += max(differences) > threshold
    return {"changed": changed, "darker": darker, "brighter": brighter, "maximum": maximum, "bounds": bounds if changed else []}
