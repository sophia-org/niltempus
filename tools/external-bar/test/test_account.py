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
sys.path.insert(0, str(ROOT/'common'))
spec = importlib.util.spec_from_file_location('account', ROOT/'common/account.py')
account = importlib.util.module_from_spec(spec)
spec.loader.exec_module(account)


def snapshot(end=False):
    return {'mono_ns': 100 if not end else 200, 'clk_tck': 100,
            'pids': [dict(pid=10, comm='server', starttime=1, utime_ticks=1 if not end else 2,
                          stime_ticks=1, minflt=0, majflt=0)]}


class Accounting(unittest.TestCase):
    def test_workload_latency_and_idle_rates(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            before, after = snapshot(), snapshot(True)
            before['mono_ns'], after['mono_ns'] = 0, 1_000_000_000
            for name, value in [('before', before), ('after', after)]:
                (root/name).write_text(json.dumps(value))
            for workload in ('fresh', 'bursty', 'retained', 'idle'):
                with self.subTest(workload=workload):
                    header = (f'# header arm=test width=1920 height=24 buffers=2 fill=test '
                              f'frames=1 warmup=0 bulk=0 lifecycle=fresh workload={workload} '
                              f'tiles={8 if workload == "retained" else 0} '
                              f'interval_ms={250 if workload == "bursty" else 0} '
                              f'damage={"96x24+1824+0" if workload == "bursty" else "full"} '
                              f'idle_sec={60 if workload == "idle" else 0} '
                              'presented_clock=monotonic\n')
                    (root/'journal').write_text(header+'1\t10000\t20000\t50000\t5000\twp_presentation\n')
                    result = subprocess.run([
                        sys.executable, str(ROOT/'common/account.py'), 'report', '--arm',
                        f'test={root}/journal:{root}/before:{root}/after', '--json', str(root/'report')
                    ], capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    report = json.loads((root/'report').read_text())['arms'][0]
                    self.assertEqual(report['server_cpu_per_second_us'], 10000)
                    self.assertIsNone(report['client_window_cpu_per_second_us'])
                    if workload == 'idle':
                        self.assertIsNone(report['cross_arm_latency_us'])
                        self.assertNotIn('server CPU/frame', result.stdout)
                    else:
                        self.assertEqual(report['cross_arm_latency_us']['p50'],
                                         45 if workload == 'retained' else 40)

    def test_receipt_bounded_sources_supply_no_latency(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, value in [('before', snapshot()), ('after', snapshot(True))]:
                (root/name).write_text(json.dumps(value))
            header = ('# header arm=x-panel width=1920 height=24 buffers=2 fill=test frames=1 warmup=0 '
                      'bulk=0 lifecycle=reuse workload=fresh tiles=0 interval_ms=0 damage=full idle_sec=0 '
                      'presented_clock=monotonic\n')
            for source in ('xpresent_late', 'xpresent_recv', 'shm_completion'):
                with self.subTest(source=source):
                    (root/'journal').write_text(header+f'1\t10000\t20000\t50000\t5000\t{source}\n')
                    result = subprocess.run([
                        sys.executable, str(ROOT/'common/account.py'), 'report', '--arm',
                        f'test={root}/journal:{root}/before:{root}/after', '--json', str(root/'report')
                    ], capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIsNone(json.loads((root/'report').read_text())['arms'][0]['cross_arm_latency_us'])

    def test_thread_endpoints_do_not_claim_complete_lifetimes_or_invent_counts(self):
        from thread_stats import endpoint_delta
        before = {'7': dict(starttime=4, voluntary=2, involuntary=3)}
        after = {'7': dict(starttime=4, voluntary=5, involuntary=7)}
        self.assertEqual(endpoint_delta(before, after), dict(voluntary=3, involuntary=4))
        self.assertIsNone(endpoint_delta(None, after))
        self.assertIsNone(endpoint_delta(before, dict(after, **{'8': after['7']})))
        self.assertIsNone(endpoint_delta(before, {'7': dict(after['7'], starttime=5)}))
        with self.assertRaises(ValueError):
            endpoint_delta(before, {'7': dict(after['7'], voluntary=1)})

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
            header = '# header arm=wl-panel width=1920 height=24 buffers=2 fill=shade_mod128_a255 frames=2 warmup=1 bulk=0 bulk_width=0 bulk_height=0 lifecycle=fresh workload=fresh tiles=0 interval_ms=0 damage=full idle_sec=0 clock=CLOCK_MONOTONIC_ns presented_clock=wp_presentation clock_id asserted CLOCK_MONOTONIC (id=1)\n'
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
                        header.replace(' workload=fresh', '')+rows,
                        header.replace('workload=fresh', 'workload=bursty')+rows,
                        header.replace(' lifecycle=fresh', '')+rows,
                        header+rows.replace('2\t40', '3\t40'),
                        header+rows.replace('20\t30', '20\t15'),
                        header+header+rows]:
                (root/'b').write_text(bad)
                result = subprocess.run(command, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0, bad)


if __name__ == '__main__': unittest.main()
