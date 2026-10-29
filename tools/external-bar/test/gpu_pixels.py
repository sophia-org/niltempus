"""Synthetic consumer-capture controls, never a hardware pixel receipt."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("pixels", ROOT / "gpu/pixels.py")
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


def image(pattern):
    # Deliberately independent literal oracle, not pixels.PATTERNS.
    left, right = ((255, 0, 0, 255), (0, 0, 255, 255)) if pattern == 1 else (
        (0, 255, 0, 255), (255, 255, 255, 255))
    row = bytes(left) * 960 + bytes(right) * 960
    return p.HEADER + row * 24 + bytes((12, 34, 56, 255)) * 1920 * (1080 - 24)


class Pixels(unittest.TestCase):
    def test_both_patterns_and_narrow_claim(self):
        for pattern in (1, 2):
            result = p.check(image(pattern), pattern)
            self.assertEqual(result["status"], "pixels_match")
            self.assertEqual(result["checked_pixels"], 1918 * 24)
            self.assertIn("No capture provenance", result["limits"])

    def test_bad_shape_truncation_trailer_and_declared_pattern(self):
        data = image(1)
        for changed in (data[:-1], data + b"\0", data.replace(b"1920", b"1919", 1),
                        data.replace(b"DEPTH 4", b"DEPTH 3", 1),
                        data.replace(b"MAXVAL 255", b"MAXVAL 254", 1),
                        data.replace(b"RGB_ALPHA", b"RGB", 1), data.replace(b"P7", b"P6", 1)):
            with self.subTest(header=changed[:100]), self.assertRaises(ValueError):
                p.check(changed, 1)
        for pattern in (0, 3, True, 1.0, "1", 2):
            with self.subTest(pattern=pattern), self.assertRaises(ValueError):
                p.check(data, pattern)

    def test_swap_missing_channel_grey_alpha_and_occlusion(self):
        data = image(1)
        for x, y, rgba in ((0, 0, (0, 0, 255, 255)), (1919, 23, (0, 0, 0, 255)),
                           (50, 12, (128, 128, 128, 255)), (10, 10, (255, 0, 0, 0)),
                           (900, 5, (255, 255, 255, 255))):
            changed = bytearray(data)
            at = len(p.HEADER) + (y * 1920 + x) * 4
            changed[at:at + 4] = bytes(rgba)
            with self.subTest(x=x, y=y), self.assertRaisesRegex(ValueError, f"x={x} y={y}"):
                p.check(changed, 1)
        with self.assertRaises(ValueError):
            p.check(image(2), 1)  # A later/stale pattern is not interchangeable.

    def test_scope_excludes_split_edge_and_nonbar_pixels(self):
        changed = bytearray(image(1))
        for x, y in ((959, 0), (960, 23), (0, 24), (1919, 1079)):
            at = len(p.HEADER) + (y * 1920 + x) * 4
            changed[at:at + 4] = bytes(4)
        self.assertEqual(p.check(changed, 1)["checked_pixels"], 46032)

    def test_cli_success_and_failure_are_distinct(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "capture.pam"
            path.write_bytes(image(1))
            command = [sys.executable, "-I", "-B", str(ROOT / "gpu/pixels.py"), str(path), "--pattern"]
            good = subprocess.run([*command, "1"], capture_output=True, text=True, timeout=5)
            self.assertEqual(good.returncode, 0, good.stderr)
            self.assertEqual(json.loads(good.stdout)["status"], "pixels_match")
            bad = subprocess.run([*command, "2"], capture_output=True, text=True, timeout=5)
            self.assertEqual(bad.returncode, 1)
            self.assertEqual(bad.stdout, "")


if __name__ == "__main__":
    unittest.main()
