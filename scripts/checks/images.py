"""Small dependency-free RGB reader for the game's non-interlaced PNG captures."""
from __future__ import annotations
from pathlib import Path
import struct
import zlib


def png_rgb(path: Path) -> tuple[int, int, bytes]:
    """A capture's width, height and RGB bytes (alpha dropped)."""
    data = Path(path).read_bytes()
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
    if raw[::stride + 1].count(0) == height:
        # Every row unfiltered (the game's captures): the pixels are the rows as stored.
        pixels = b"".join(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)] for y in range(height))
    else:
        pixels = _unfilter(raw, stride, height, channels)
    if channels == 4:
        rgb = bytearray(width * height * 3)
        for channel in range(3):
            rgb[channel::3] = pixels[channel::4]
        pixels = bytes(rgb)
    return width, height, bytes(pixels)


def _unfilter(raw: bytes, stride: int, height: int, channels: int) -> bytearray:
    """Undoes the five PNG scanline filters, byte by byte (PNGs from other encoders)."""
    previous, pixels = bytearray(stride), bytearray()
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
        pixels.extend(row)
        previous = row
    return pixels


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
    for y in range(y0, y1):
        start, end = (y * width + x0) * 3, (y * width + x1) * 3
        row_a, row_b = a[start:end], b[start:end]
        if row_a == row_b:
            continue  # most rows of two captures of one scene match exactly
        for i in range(0, end - start, 3):
            red, green, blue = row_b[i] - row_a[i], row_b[i + 1] - row_a[i + 1], row_b[i + 2] - row_a[i + 2]
            delta = max(abs(red), abs(green), abs(blue))
            if delta > maximum:
                maximum = delta
            if delta > threshold:
                changed += 1
                x = x0 + i // 3
                bounds = [min(bounds[0], x), min(bounds[1], y), max(bounds[2], x), max(bounds[3], y)]
            darker += min(red, green, blue) < -threshold
            brighter += max(red, green, blue) > threshold
    return {"changed": changed, "darker": darker, "brighter": brighter, "maximum": maximum, "bounds": bounds if changed else []}


def pixel_mask(rgb: bytes, red: range, green: range, blue: range) -> bytes:
    """One byte per pixel: 1 where every channel lies in its range, else 0 (whole-buffer byte
    operations, no per-pixel Python loop)."""
    masks = [int.from_bytes(rgb[channel::3].translate(bytes(int(v in wanted) for v in range(256))), "big")
             for channel, wanted in enumerate((red, green, blue))]
    return (masks[0] & masks[1] & masks[2]).to_bytes(len(rgb) // 3, "big")
