"""Import injection and response binding controls; no native display or GPU."""
import os
import importlib.util
from pathlib import Path
import subprocess
import unittest

BUILD = Path(__file__).resolve().parents[1] / 'build'
spec = importlib.util.spec_from_file_location('qualify_invalid', BUILD.parent/'gpu/qualify.py')
qualifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qualifier)


class InvalidImport(unittest.TestCase):
    def test_expected_refusal_is_not_a_local_failure_or_timeout(self):
        first = 'gpu_invalid_import schema=1 status=injected generation=1 token=9 bytes=184320\n'
        last = ('gpu_invalid_import_refused schema=1 transport=x11 token=9 '
                'code=2 major=145 minor=7 object=73 released=0\n')
        text = first + last
        self.assertEqual(qualifier.qualify_import_refusal(text, 'x11', 42)['token'], 9)
        wayland = text.replace('transport=x11', 'transport=wayland').replace('major=145 minor=7', 'major=0 minor=0')
        qualifier.qualify_import_refusal(wayland, 'wayland', 42)
        for rc in (0, 1, 2, -9, True, 42.0):
            with self.subTest(rc=rc), self.assertRaises(ValueError):
                qualifier.qualify_import_refusal(text, 'x11', rc)
        for bad in (
            first, last, last+first, text+last, text[:-1], 'prefix '+text,
            first+last.replace('token=9', 'token=8'), text.replace('released=0', 'released=1'),
            text.replace('minor=7', 'minor=1'), text.replace('major=145', 'major=1'),
            text.replace('generation=1', 'generation=2'), text.replace('bytes=184320', 'bytes=184319'),
            text.replace('code=2', 'code=0'), text.replace('object=73', 'object=0'),
            text.replace('schema=1', 'schema=01'), text.replace('code=2', 'code=2 code=2'),
        ):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                qualifier.qualify_import_refusal(bad, 'x11', 42)

    def test_fd_and_protocol_controls(self):
        for name in ('gpu-invalid-import-test', 'gpu-wl-invalid-test', 'gpu-x-invalid-test'):
            with self.subTest(name=name):
                result = subprocess.run([str(BUILD/name)], capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn('controls passed', result.stdout)
                self.assertEqual(result.stderr.count('gpu_invalid_import_refused '), 1)
                if name != 'gpu-invalid-import-test':
                    # Later child refusals in this fixture are separate attempts.
                    lines = result.stderr.splitlines(keepends=True)[:2]
                    transport = 'wayland' if name == 'gpu-wl-invalid-test' else 'x11'
                    qualifier.qualify_import_refusal(''.join(lines), transport, 42)

    def test_recipe_refuses_before_connect(self):
        env = dict(os.environ, DISPLAY='invalid-display', WAYLAND_DISPLAY='/absent-test-display')
        for name, base in (
            ('wl-panel-gpu-invalid', ['/dev/null', '--output', 'DP-1']),
            ('x-panel-gpu-invalid', ['--journal', '/dev/null']),
        ):
            for frames, warmup in ((2, 0), (1, 1)):
                with self.subTest(name=name, frames=frames, warmup=warmup):
                    argv = [str(BUILD/name), *base, '--frames', str(frames), '--warmup',
                            str(warmup), '--buffers', '2', '--lifecycle', 'fresh']
                    result = subprocess.run(argv, capture_output=True, text=True, env=env, timeout=5)
                    self.assertEqual(result.returncode, 2)
                    self.assertIn('requires one frame and no warmup', result.stderr)
                    self.assertNotIn('gpu_invalid_import_refused', result.stderr)


if __name__ == '__main__':
    unittest.main()
