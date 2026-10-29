"""GPU-only recipe refusals happen before connecting to a display or opening a GPU."""
import os
from pathlib import Path
import subprocess
import unittest

BUILD = Path(__file__).resolve().parents[1] / "build"


class Recipe(unittest.TestCase):
    def refuse(self, binary, base, changes):
        for change in changes:
            with self.subTest(binary=binary, change=change):
                command = [str(BUILD / binary), *base, *change]
                result = subprocess.run(
                    command, capture_output=True, text=True, timeout=5,
                    env={**os.environ, "DISPLAY": "invalid-display", "WAYLAND_DISPLAY": "/absent-test-display"},
                )
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("GPU mode requires", result.stderr)

    def test_wayland(self):
        self.refuse("wl-panel-gpu", ["/dev/null", "--frames", "4", "--warmup", "0",
                    "--lifecycle", "fresh", "--buffers", "2", "--output", "DP-1"], [
            ["--buffers", "3"], ["--bulk"], ["--workload", "retained"],
            ["--lifecycle", "reuse"], ["--frames", "1000001"], ["--warmup", "1000001"],
        ])

    def test_x(self):
        self.refuse("x-panel-gpu", ["--journal", "/dev/null", "--frames", "4", "--warmup", "0",
                    "--lifecycle", "fresh", "--buffers", "2"], [
            ["--buffers", "3"], ["--bulk"], ["--workload", "retained"],
            ["--lifecycle", "reuse"], ["--frames", "1000001"], ["--warmup", "1000001"],
        ])


if __name__ == "__main__":
    unittest.main()
