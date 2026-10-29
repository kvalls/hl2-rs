"""Independent scanline fixtures for the packaged capture reader."""
import struct
import tempfile
import unittest
import zlib
from pathlib import Path
from images import png_rgb, pixel_changes


def write_png(path, rows, colour=2, width=2):
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    header = struct.pack(">IIBBBBB", width, 2, 8, colour, 0, 0, 0)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
                     + chunk(b"IDAT", zlib.compress(b"".join(rows))) + chunk(b"IEND", b""))


class CaptureImages(unittest.TestCase):
    def test_all_five_scanline_filters(self):
        encoded = (
            ([10, 20, 30, 40, 50, 60], [15, 30, 45, 90, 100, 110]),
            ([10, 20, 30, 30, 30, 30], [15, 30, 45, 75, 70, 65]),
            ([10, 20, 30, 40, 50, 60], [5, 10, 15, 50, 50, 50]),
            ([10, 20, 30, 35, 40, 45], [10, 20, 30, 63, 60, 58]),
            ([10, 20, 30, 30, 30, 30], [5, 10, 15, 50, 50, 50]),
        )
        expected = bytes([10, 20, 30, 40, 50, 60, 15, 30, 45, 90, 100, 110])
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "filter.png"
            for method, rows in enumerate(encoded):
                with self.subTest(method=method):
                    write_png(path, [bytes([method, *row]) for row in rows])
                    self.assertEqual(png_rgb(path), (2, 2, expected))

    def test_rgba_captures_ignore_alpha_in_colour_comparisons(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "rgba.png"
            write_png(path, [bytes([0, 1, 2, 3, 0, 4, 5, 6, 255])] * 2, colour=6)
            self.assertEqual(png_rgb(path)[2], bytes([1, 2, 3, 4, 5, 6] * 2))

    def test_pixel_change_direction_and_threshold(self):
        with tempfile.TemporaryDirectory() as folder:
            a, b = Path(folder) / "a.png", Path(folder) / "b.png"
            write_png(a, [bytes([0, 0, 10, 200]), bytes([0, 10, 10, 10])], width=1)
            write_png(b, [bytes([0, 0, 40, 100]), bytes([0, 11, 10, 9])], width=1)
            self.assertEqual(pixel_changes(a, b, region=(0, 1, 1, 2)),
                             {"changed": 0, "darker": 0, "brighter": 0, "maximum": 1, "bounds": []})
            with self.assertRaises(ValueError):
                pixel_changes(a, b, region=(0, 0, 2, 2))
            self.assertEqual(pixel_changes(a, b), {"changed": 1, "darker": 1, "brighter": 1, "maximum": 100, "bounds": [0, 0, 0, 0]})


if __name__ == "__main__":
    unittest.main()
