#!/usr/bin/env python3
"""Controls for rejecting misleading accounting joins, without a display."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('account', ROOT/'common/account.py')
account = importlib.util.module_from_spec(spec)
spec.loader.exec_module(account)


def snapshot(end=False):
    return {'mono_ns': 100 if not end else 200, 'clk_tck': 100,
            'pids': [dict(pid=10, comm='server', starttime=1, utime_ticks=1 if not end else 2,
                          stime_ticks=1, minflt=0, majflt=0)]}


class Accounting(unittest.TestCase):
    def test_counter_identity_window_and_tick_rate_refusals(self):
        before, after = snapshot(), snapshot(True)
        self.assertEqual(account.cpu_delta(before, after)['cpu_ns'], 10000000)
        cases = []
        for key, value in [('clk_tck', 1000), ('mono_ns', 100), ('pids', [])]:
            bad = copy.deepcopy(after); bad[key] = value; cases.append(bad)
        for key, value in [('starttime', 2), ('utime_ticks', 0), ('minflt', -1)]:
            bad = copy.deepcopy(after); bad['pids'][0][key] = value; cases.append(bad)
        bad = copy.deepcopy(after); bad['pids'] *= 2; cases.append(bad)
        for bad in cases:
            with self.assertRaises(SystemExit): account.cpu_delta(before, bad)

    def test_real_header_shape_missing_cpu_and_lifecycle_refusal(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, content in [('before', snapshot()), ('after', snapshot(True))]:
                (root/name).write_text(json.dumps(content))
            header = '# header arm=wl-panel width=1920 height=24 buffers=2 fill=shade_mod128_a255 frames=2 warmup=1 bulk=0 bulk_width=0 bulk_height=0 lifecycle=fresh clock=CLOCK_MONOTONIC_ns presented_clock=wp_presentation clock_id asserted CLOCK_MONOTONIC (id=1)\n'
            rows = '1\t10\t20\t30\t5\twp_presentation\n2\t40\t50\t60\t35\twp_presentation\n'
            (root/'a').write_text(header+rows)
            (root/'b').write_text(header+rows)
            command = [sys.executable, str(ROOT/'common/account.py'), 'report', '--arm',
                       f'a={root}/a:{root}/before:{root}/after', '--arm',
                       f'b={root}/b:{root}/before:{root}/after', '--json', str(root/'report')]
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads((root/'report').read_text())
            self.assertIn('diagnostic_only', report['qualification'])
            self.assertIsNone(report['arms'][0]['client_cpu_ns'])
            self.assertEqual(report['arms'][0]['fill_to_presented_us']['n'], 2)
            for bad in [header.replace('fresh', 'reuse')+rows,
                        header.replace(' lifecycle=fresh', '')+rows,
                        header+rows.replace('2\t40', '3\t40'),
                        header+rows.replace('20\t30', '20\t15'),
                        header+header+rows]:
                (root/'b').write_text(bad)
                result = subprocess.run(command, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0, bad)


if __name__ == '__main__': unittest.main()
