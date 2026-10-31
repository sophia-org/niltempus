"""Delayed application of matched release events; mocks only, no display or GPU."""
import importlib.util
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('receipts_fixture', ROOT/'test/gpu_receipts.py')
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
q = fixture.q


class ReleaseDelay(unittest.TestCase):
    def test_real_adapter_release_boundaries(self):
        for transport, name in (('wayland', 'gpu-wl-delay-test'), ('x11', 'gpu-x-delay-test')):
            with self.subTest(transport=transport):
                run = subprocess.run([str(ROOT/'build'/name)], capture_output=True,
                                     text=True, timeout=5)
                self.assertEqual(run.returncode, 0, run.stderr)
                self.assertIn('control passed', run.stdout)
                self.assertEqual(q.qualify_release_delay(run.stderr, transport)['status'],
                                 'release_application_delayed')

    def test_trace_requires_identity_order_and_delay(self):
        for transport in ('wayland', 'x11', 'sophia'):
            sophia = transport == 'sophia'
            clock = 'monotonic_usec' if sophia else 'monotonic_ns'
            units = 1_000_000 if sophia else 1_000_000_000
            identity = ('offer=7 connection_epoch=2 content_grant_epoch=3 resource=1 generation=1'
                        if sophia else 'token=7')
            lines = [f'gpu_release_delay schema=1 phase={phase} {identity} {clock}={now}\n'
                     for phase, now in zip(('held', 'resume', 'released'),
                                           (units, units+units//10, units+units//10+1))]
            text = ''.join(lines)
            q.qualify_release_delay(text, transport)
            for bad in (
                text[:-1], text+lines[-1], ''.join(lines[1:]), ''.join(reversed(lines)),
                'prefix '+text, text.replace('schema=1', 'schema=01'),
                text.replace(identity, identity+' extra=1'),
                lines[0]+lines[1].replace('=7 ', '=8 ')+lines[2],
                text.replace(f'{clock}={units+units//10}\n', f'{clock}={units+1}\n'),
                text.replace(f'{clock}={units+units//10+1}\n', f'{clock}={units}\n'),
                text.replace(f'{clock}={units+units//10}\n', f'{clock}={units*7}\n'),
                text.replace('=7 ', '=0 '), text.replace('=7 ', '=07 '),
                text.replace('schema=1', 'schema=1 schema=1'),
            ):
                with self.subTest(transport=transport, bad=bad), self.assertRaises(ValueError):
                    q.qualify_release_delay(bad, transport)

    def test_delay_journal_cannot_qualify_as_timed_or_colour(self):
        for transport in q.TRANSPORTS:
            source = q.TRANSPORTS[transport][2]
            text = (fixture.fixture(transport)
                    .replace('vello_shade_mod128_opaque_rgb', 'diagnostic_release_delay_100ms')
                    .replace('frames=2 warmup=1', 'frames=1 warmup=0')
                    .replace('rendered=3 released=3', 'rendered=1 released=1')
                    .replace('ok=3 violations=0', 'ok=1 violations=0')
                    .replace(f'2\t40\t50\t60\t30\t{source}\n', ''))
            self.assertEqual(q.qualify_delay(text, transport)['frames'], 1)
            with self.assertRaises(ValueError):
                q.qualify(text, transport, 1, 0)
            with self.assertRaises(ValueError):
                q.qualify_colour(text, transport)

    def test_recipe_refuses_before_connect(self):
        env = dict(os.environ, DISPLAY='invalid-display', WAYLAND_DISPLAY='/absent-test-display')
        for name, base in (('wl-panel-gpu-delay', ['/dev/null', '--output', 'DP-1']),
                           ('x-panel-gpu-delay', ['--journal', '/dev/null'])):
            for frames, warmup in ((2, 0), (1, 1)):
                run = subprocess.run([str(ROOT/'build'/name), *base, '--frames', str(frames),
                                      '--warmup', str(warmup), '--buffers', '2', '--lifecycle',
                                      'fresh'], env=env, capture_output=True, text=True, timeout=5)
                self.assertEqual(run.returncode, 2, run.stderr)
                self.assertIn('requires one frame and no warmup', run.stderr)
                self.assertNotIn('gpu_release_delay schema=', run.stderr)


if __name__ == '__main__':
    unittest.main()
