#!/usr/bin/env python3
"""Device-free tests for bench.py. Run: python3 -I -B -W error test_bench.py"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import types
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench  # noqa: E402

MS = 1_000_000
START = 1_000 * MS                    # first presentation of the counted window
PERIOD = 8_341_666                    # 119.88 Hz
WARMUP, FRAMES = 119, 2001


def journal(path, frames=FRAMES, source='wp_presentation', missing=None):
    lines = [f'# header arm=wl-panel width=1920 height=24 buffers=2 fill=shade_mod128_a255 frames={frames} '
             f'warmup={WARMUP} bulk=0 bulk_width=0 bulk_height=0 lifecycle=reuse workload=fresh tiles=0 '
             'interval_ms=0 damage=full idle_sec=0 clock=CLOCK_MONOTONIC_ns presented_clock=as_is']
    for k in range(1, frames + 1):
        presented = START + (k - 1) * PERIOD
        submit = presented - 7 * MS
        lines.append(f'{k}\t{submit - 100}\t{submit}\t{-1 if k == missing else presented}\t-1\t{source}')
    path.write_text('\n'.join(lines + ['# pid 2']) + '\n')


def samples(path, procs, end_ns):
    """5 ms samples from 0.5 s before START to past end_ns; every process
    gains one 10 ms tick per 20 ms, so each costs 0.5 ms per ms."""
    head = dict(kind='header', version=1, clk_tck=100, interval_ns=5 * MS, boot_id='b', sampler_pid=1,
                procs={n: dict(pid=p, starttime=p * 10, comm=n) for n, p in procs.items()})
    out, seq, t = [head], 0, START - 500 * MS
    while t < end_ns + 50 * MS:
        out.append(dict(kind='sample', seq=seq, t0=t, t1=t + 20_000, self_cpu_ns=0,
                        procs={n: dict(pid=p, starttime=p * 10, comm=n, ticks=t // (20 * MS))
                               for n, p in procs.items()}))
        seq, t = seq + 1, t + 5 * MS
    out.append(dict(kind='footer', samples=seq, late_slots=0, self_cpu_ns=0, stop='signal'))
    path.write_text(''.join(json.dumps(r) + '\n' for r in out))


def run_dir(root, name, stack='niri', mode='native', **over):
    d = root / name
    d.mkdir()
    procs = {'server': 101, 'client': 102}
    end = START + (FRAMES - 1) * PERIOD
    journal(d / 'journal.tsv', source=over.pop('source', 'wp_presentation' if stack == 'niri' else 'xpresent'),
            missing=over.pop('missing', None))
    samples(d / 'cpu-window.jsonl', procs, end)
    record = dict(schema=1, stack=stack, mode=mode, evidence='native' if mode == 'native' else 'diagnostic',
                  output='DP-1', mode_size='1920x1080', rate='119.880', unbound_start={}, unbound_end={},
                  expect={n: dict(pid=p, starttime=p * 10) for n, p in procs.items()})
    record.update(over)
    (d / 'run.json').write_text(json.dumps(record))
    if stack == 'niri':
        (d / 'outputs.json').write_text(json.dumps(NIRI_OUTPUTS))
    else:
        (d / 'xrandr.txt').write_text(XRANDR)
    return d


NIRI_OUTPUTS = {
    'DP-1': dict(current_mode=1, vrr_enabled=False,
                 modes=[dict(width=2560, height=1440, refresh_rate=59951),
                        dict(width=1920, height=1080, refresh_rate=119880)]),
    'HDMI-A-1': dict(current_mode=None, vrr_enabled=False, modes=[]),
}
XRANDR = '''Screen 0: minimum 320 x 200, current 1920 x 1080, maximum 16384 x 16384
DP-1 connected primary 1920x1080+0+0 (0x4c) normal (normal left inverted right x axis y axis) 600mm x 340mm
  1920x1080 (0x4b) 148.500MHz +HSync +VSync
        h: width  1920 start 2008 end 2052 total 2200 skew    0 clock  67.50KHz
        v: height 1080 start 1084 end 1089 total 1125           clock  60.00Hz
  1920x1080 (0x4c) 285.500MHz +HSync -VSync *current
        h: width  1920 start 1968 end 2000 total 2080 skew    0 clock 137.26KHz
        v: height 1080 start 1083 end 1088 total 1144           clock 119.88Hz
HDMI-A-1 disconnected (normal left inverted right x axis y axis)
'''


class Report(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.root)

    def test_window_is_rows_one_and_last_so_generations_120_to_2120(self):
        entry, stats = bench.plan_run('a', run_dir(self.root, 'a'))
        self.assertEqual((entry['start_frame'], entry['end_frame']), (120, 2120))
        self.assertEqual((entry['anchor_start_ns'], entry['anchor_end_ns']), (START, START + 2000 * PERIOD))
        self.assertAlmostEqual(stats['refresh_hz'], 119.88, places=2)
        self.assertEqual(stats['missed_vblank_gaps'], 0)

    def test_native_rows_refuse_fallback_missing_or_unsampled_evidence(self):
        cases = {
            'recv': dict(stack='xlibre', source='xpresent_recv'),
            'missing': dict(missing=2001),
            'unbound': dict(unbound_end={'7': dict(starttime=1, comm='helper')}),
        }
        for name, over in cases.items():
            with self.assertRaises(bench.Refused, msg=name):
                bench.plan_run(name, run_dir(self.root, name, **over))
        d = run_dir(self.root, 'noreceipt')
        (d / 'outputs.json').unlink()
        with self.assertRaises(bench.Refused):
            bench.plan_run('noreceipt', d)

    def test_smoke_rows_accept_any_source_but_say_so(self):
        _, stats = bench.plan_run('s', run_dir(self.root, 's', stack='xlibre', mode='smoke', source='xpresent_recv'))
        self.assertEqual(stats['evidence'], 'diagnostic')
        self.assertIn("['xpresent_recv']", stats['notes'][0])

    def test_niri_receipt_checks_actual_mode_vrr_and_other_outputs(self):
        record = dict(output='DP-1', mode_size='1920x1080', rate='119.880')
        path = self.root / 'outputs.json'
        path.write_text(json.dumps(NIRI_OUTPUTS))
        self.assertEqual(bench.niri_output_problems(record, path), [])
        bad = json.loads(json.dumps(NIRI_OUTPUTS))
        bad['DP-1'].update(current_mode=0, vrr_enabled=True)
        bad['HDMI-A-1']['current_mode'] = 0
        path.write_text(json.dumps(bad))
        self.assertEqual(len(bench.niri_output_problems(record, path)), 3)

    def test_x_receipt_reads_the_current_mode_vertical_clock(self):
        record = dict(output='DP-1', mode_size='1920x1080', rate='119.880')
        path = self.root / 'xrandr.txt'
        path.write_text(XRANDR)
        self.assertEqual(bench.x_output_problems(record, path), [])
        path.write_text(XRANDR.replace('*current', '').replace('+HSync +VSync', '+HSync +VSync *current'))
        self.assertEqual(bench.x_output_problems(record, path), ['DP-1 vertical clock 60.0 Hz'])
        path.write_text(XRANDR.replace('HDMI-A-1 disconnected', 'HDMI-A-1 connected 1920x1080+1920+0'))
        self.assertEqual(bench.x_output_problems(record, path), ['HDMI-A-1 is active'])

    def report(self, *runs):
        out = self.root / 'report'
        args = types.SimpleNamespace(out=out, runs=list(runs), quiet=True)
        status = bench.cmd_report(args)
        return status, json.loads((out / 'report.json').read_text())

    def test_native_report_attributes_the_exact_window_per_process(self):
        status, report = self.report(run_dir(self.root, 'n', stack='niri'), run_dir(self.root, 'x', stack='xlibre'))
        self.assertEqual(status, 0, report['refused'])
        self.assertEqual(report['evidence'], 'native')
        lo, hi = report['rows'][0]['cpu_us_per_frame']
        # Each of two processes spends 0.5 ms per ms: about 8342 us per frame in all.
        self.assertTrue(lo <= 2 * 0.5 * PERIOD / 1e3 <= hi, (lo, hi))

    def test_mixed_evidence_reports_no_differences(self):
        status, report = self.report(run_dir(self.root, 'n'), run_dir(self.root, 's', mode='smoke'))
        self.assertEqual(report['evidence'], 'diagnostic')
        self.assertEqual(report['differences'], {})

    def test_refresh_mismatch_between_native_rows_is_refused(self):
        a = run_dir(self.root, 'a')
        b = run_dir(self.root, 'b')
        rows = (b / 'journal.tsv').read_text().splitlines()
        out = [rows[0]]
        for row in rows[1:]:
            if row.startswith('#'):
                out.append(row)
                continue
            f = row.split('\t')
            k = int(f[0])
            f[3] = str(START + (k - 1) * 16_683_333)  # 59.94 Hz
            out.append('\t'.join(f))
        (b / 'journal.tsv').write_text('\n'.join(out) + '\n')
        status, report = self.report(a, b)
        self.assertEqual(status, 1)
        self.assertIn('refresh', report['refused'][0]['reason'])

    def test_sophia_import_uses_kernel_completions_120_to_2120(self):
        src = self.root / 'sophia'
        src.mkdir()
        samples(src / 'cpu-window.jsonl', {'session': 201, 'peer': 202}, START + 2000 * PERIOD)
        (src / 'cpu-binding.json').write_text(json.dumps(dict(host=dict(pid=201, start_time_ticks=2010),
                                                               peer=dict(pid=202, start_time_ticks=2020))))
        gens = [dict(candidate_generation=g, completed_monotonic_usec=(START + (g - 120) * PERIOD) // 1000,
                     latency_usec=7500) for g in range(1, 2121)]
        (src / 'native-samples.json').write_text(json.dumps(dict(offered=2120, samples=gens,
                                                                  instrumentation='capture-profile')))
        refused = self.root / 'refused'
        self.assertEqual(bench.cmd_import_sophia(types.SimpleNamespace(out=refused, run=src,
                                                                       allow_instrumented=False)), 2)
        self.assertFalse(refused.exists())
        out = self.root / 'imported'
        bench.cmd_import_sophia(types.SimpleNamespace(out=out, run=src, allow_instrumented=True))
        entry, stats = bench.plan_run('s', out)
        self.assertEqual(stats['evidence'], 'diagnostic')  # instrumented Sophia is never native evidence
        self.assertEqual((entry['start_frame'], entry['end_frame']), (120, 2120))
        self.assertEqual(sorted(entry['expect']), ['peer', 'session'])
        (src / 'native-samples.json').write_text('{}')
        with self.assertRaises(bench.Refused):
            bench.plan_run('s', out)


class Hold(unittest.TestCase):
    def test_client_is_held_after_exec_and_at_exit_under_its_own_identity(self):
        root = Path(tempfile.mkdtemp())
        try:
            stack = bench.Stack(root, dict(os.environ))
            client = stack.spawn('client', ['/bin/sleep', '0.05'], hold=True)
            starttime, _, comm = bench.stat_fields(client.pid)
            self.assertEqual(comm, 'sleep')  # bound after exec, never as python
            state = Path(f'/proc/{client.pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
            self.assertEqual(state, 't')  # traced stop: nothing has run yet
            status, exit_stop = bench.run_held(client, timeout=10, hold=0.1)
            self.assertEqual(status, 0)
            self.assertIsNotNone(exit_stop)
        finally:
            shutil.rmtree(root)


if __name__ == '__main__':
    unittest.main()
