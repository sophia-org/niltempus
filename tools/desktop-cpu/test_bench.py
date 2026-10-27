#!/usr/bin/env python3
"""Device-free tests for bench.py. Run: python3 -I -B -W error test_bench.py"""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import types
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench  # noqa: E402

MS = 1_000_000
START = 1_000 * MS                    # first presentation of the counted window
PERIOD = 8_341_666                    # 119.88 Hz
WARMUP, FRAMES = 119, 2001


def journal(path, arm='wl-panel', source='wp_presentation', missing=None, header=None):
    fields = dict(bench.RECIPE, **(header or {}))
    lines = [f'# header arm={arm} ' + ' '.join(f'{k}={v}' for k, v in fields.items()) + ' presented_clock=as_is']
    for k in range(1, FRAMES + 1):
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
    journal(d / 'journal.tsv', arm=bench.CLIENT_ARM[stack],
            source=over.pop('source', 'wp_presentation' if stack == 'niri' else 'xpresent'),
            missing=over.pop('missing', None), header=over.pop('header', None))
    samples(d / 'cpu-window.jsonl', procs, end)
    record = dict(schema=1, stack=stack, mode=mode, evidence='native' if mode == 'native' else 'diagnostic',
                  recipe=bench.RECIPE, output='DP-1', mode_size='1920x1080', rate='119.880',
                  unbound_start={}, unbound_end={}, finished=True, client_status=0, sampler_status=0,
                  client_exit_stop_ns=START + 2001 * PERIOD, teardown_errors=[],
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


def sophia_run(root, instrumentation='capture-profile'):
    """A shell-timing run directory as sophia_window reads it (shapes from
    continuous-thin-lto-01 r120b1)."""
    src = root / 'sophia'
    src.mkdir()
    samples(src / 'cpu-window.jsonl', {'session': 201, 'peer': 202}, START + 2000 * PERIOD)
    gens = []
    for g in range(1, 2121):
        done = (START + (g - 120) * PERIOD) // 1000
        gens.append(dict(candidate_generation=g, completed_monotonic_usec=done, submitted_monotonic_usec=done - 7500,
                         latency_usec=7500, output=1, heads=1))
    files = {
        'cpu-binding.json': dict(host=dict(pid=201, start_time_ticks=2010), peer=dict(pid=202, start_time_ticks=2020)),
        'native-samples.json': dict(schema=2, verdict='joined_only', offered=2120, samples=gens,
                                    instrumentation=instrumentation,
                                    journal=dict(offers=[dict(outcome='presented')] * 2120)),
        'postflight.json': dict(schema=1, status='passed', failures=[],
                                observed=dict(connector='DP-1', actual_refresh_millihz=119880)),
        'result.json': dict(schema=1, status='captured_only', error=None),
        'plan.json': dict(request=dict(source='owned_ring', frames=2120, instrumentation=instrumentation,
                                       artifacts={})),
        'panel-diagnostic.json': dict(bar_height=24, warmup=120, counted=2000,
                                      target=dict(connector='DP-1', width=1920, height=1080, vrr=False)),
    }
    for name, value in files.items():
        (src / name).write_text(json.dumps(value))
    return src


def edit_json(path, change):
    value = json.loads(path.read_text())
    change(value)
    path.write_text(json.dumps(value))
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

    def test_a_full_journal_needs_a_cleanly_finished_run(self):
        cases = {
            'failedclient': dict(client_status=1),
            'timeout': dict(client_status=None),
            'sampler': dict(sampler_status=1),
            'noexitstop': dict(client_exit_stop_ns=None),
            'unfinished': dict(finished=None, error="Refused('x')"),
            'teardown': dict(teardown_errors=['client: Refused()']),
        }
        for name, over in cases.items():
            for mode in ('native', 'smoke'):
                with self.assertRaises(bench.Refused, msg=(name, mode)):
                    bench.plan_run(name, run_dir(self.root, f'{name}-{mode}', mode=mode, **over))

    def test_only_the_pinned_recipe_is_accepted(self):
        cases = {
            'lifecycle': dict(header=dict(lifecycle='fresh')),
            'warmup': dict(header=dict(warmup='20')),
            'buffers': dict(header=dict(buffers='3')),
            'fill': dict(header=dict(fill='solid')),
            'damage': dict(header=dict(damage='tile')),
            'bulk': dict(header=dict(bulk='1')),
            'record': dict(recipe=dict(bench.RECIPE, lifecycle='fresh')),
        }
        for name, over in cases.items():
            for mode in ('native', 'smoke'):
                with self.assertRaises(bench.Refused, msg=(name, mode)):
                    bench.plan_run(name, run_dir(self.root, f'{name}-{mode}', mode=mode, **over))
        d = run_dir(self.root, 'arm', stack='xlibre')
        (d / 'journal.tsv').write_text((d / 'journal.tsv').read_text().replace('arm=x-panel', 'arm=wl-panel'))
        with self.assertRaises(bench.Refused):
            bench.plan_run('arm', d)

    def test_native_cadence_is_one_refresh_of_the_observed_mode_per_interval(self):
        def retime(d, at):
            rows = (d / 'journal.tsv').read_text().splitlines()
            for i, row in enumerate(rows):
                if not row.startswith('#'):
                    f = row.split('\t')
                    f[3] = str(at(int(f[0])))
                    rows[i] = '\t'.join(f)
            (d / 'journal.tsv').write_text('\n'.join(rows) + '\n')
        half = run_dir(self.root, 'half')  # the receipt says 119.88; every frame took two refreshes
        retime(half, lambda k: START + (k - 1) * 2 * PERIOD)
        one = run_dir(self.root, 'one')  # one doubled interval at the right median rate
        retime(one, lambda k: START + (k - 1) * PERIOD + (PERIOD if k > 1000 else 0))
        same = run_dir(self.root, 'same', mode='smoke')  # duplicate time, even in a smoke row
        retime(same, lambda k: START + (k - 1 if k != 7 else k - 2) * PERIOD)
        for name, d in (('half', half), ('one', one), ('same', same)):
            with self.assertRaises(bench.Refused, msg=name):
                bench.plan_run(name, d)
        _, stats = bench.plan_run('s', run_dir(self.root, 'slow', mode='smoke', rate='60'))
        self.assertIsNone(stats['nominal_hz'])
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

    def test_x_outputs_are_the_servers_own_names(self):
        rig = XRANDR.replace('HDMI-A-1 disconnected', 'HDMI-1 connected') + 'DP-2 disconnected (normal)\n'
        self.assertEqual(bench.x_outputs(rig), ['DP-1', 'HDMI-1', 'DP-2'])

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

    def test_any_refused_row_withholds_differences(self):
        status, report = self.report(run_dir(self.root, 'n'), run_dir(self.root, 'x', stack='xlibre'),
                                     run_dir(self.root, 'bad', mode='smoke', client_status=1))
        self.assertEqual(status, 1)
        self.assertEqual((report['evidence'], report['differences']), ('diagnostic', {}))

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

    def import_(self, src, out, allow=False):
        return bench.cmd_import_sophia(types.SimpleNamespace(out=self.root / out, run=src, allow_instrumented=allow))

    def test_sophia_import_uses_kernel_completions_120_to_2120(self):
        src = sophia_run(self.root)
        self.assertEqual(self.import_(src, 'refused'), 2)
        self.assertFalse((self.root / 'refused').exists())
        self.assertEqual(self.import_(src, 'imported', allow=True), 0)
        entry, stats = bench.plan_run('s', self.root / 'imported')
        self.assertEqual(stats['evidence'], 'diagnostic')  # instrumented Sophia is never native evidence
        self.assertEqual((entry['start_frame'], entry['end_frame']), (120, 2120))
        self.assertEqual((entry['anchor_start_ns'], entry['anchor_end_ns']), (START, START + 2000 * PERIOD))
        self.assertEqual(sorted(entry['expect']), ['peer', 'session'])
        (src / 'native-samples.json').write_text((src / 'native-samples.json').read_text() + ' ')
        with self.assertRaises(bench.Refused):  # rewritten since import
            bench.plan_run('s', self.root / 'imported')

    def test_uninstrumented_sophia_is_native(self):
        self.assertEqual(self.import_(sophia_run(self.root, 'none'), 'imported'), 0)
        _, stats = bench.plan_run('s', self.root / 'imported')
        self.assertEqual((stats['evidence'], stats['missed_vblank_gaps'], stats['nominal_hz']), ('native', 0, 119.88))

    def test_sophia_import_refuses_a_failed_or_malformed_run(self):
        def dup(v):
            v['samples'][1] = dict(v['samples'][0])
        def late(v):
            v['samples'][500]['completed_monotonic_usec'] = v['samples'][499]['completed_monotonic_usec']
        def join(v):
            v['samples'][700]['latency_usec'] += 1
        mutations = {
            'duplicate generation': ('native-samples.json', dup),
            'flat completion': ('native-samples.json', late),
            'latency not a join': ('native-samples.json', join),
            'second head': ('native-samples.json', lambda v: v['samples'][9].update(heads=2)),
            'unpresented offer': ('native-samples.json', lambda v: v['journal']['offers'].__setitem__(5, {})),
            'postflight failed': ('postflight.json', lambda v: v.update(status='failed')),
            'wrong refresh': ('postflight.json', lambda v: v['observed'].update(actual_refresh_millihz=59940)),
            'capture failed': ('result.json', lambda v: v.update(status='failed', error='x')),
            'dma_buf source': ('plan.json', lambda v: v['request'].update(source='dma_buf')),
            'other strip': ('panel-diagnostic.json', lambda v: v.update(bar_height=48)),
            'VRR on': ('panel-diagnostic.json', lambda v: v['target'].update(vrr=True)),
            'other peer': ('cpu-binding.json', lambda v: v['peer'].update(start_time_ticks=1)),
        }
        for index, (name, (file, change)) in enumerate(mutations.items()):
            (self.root / str(index)).mkdir()
            src = sophia_run(self.root / str(index), 'none')
            edit_json(src / file, change)
            self.assertEqual(self.import_(src, f'out{index}'), 2, name)
            self.assertFalse((self.root / f'out{index}').exists(), name)


class Hold(unittest.TestCase):
    def test_client_is_held_after_exec_and_at_exit_under_its_own_identity(self):
        root = Path(tempfile.mkdtemp())
        try:
            stack = bench.Stack(root, dict(os.environ))
            client = stack.spawn('client', ['/bin/sleep', '0.05'], hold=True)
            starttime, _, comm, _ = bench.stat_fields(client.pid)
            self.assertEqual(comm, 'sleep')  # bound after exec, never as python
            state = Path(f'/proc/{client.pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
            self.assertEqual(state, 't')  # traced stop: nothing has run yet
            status, exit_stop = bench.run_held(client, timeout=10, hold=0.1)
            self.assertEqual(status, 0)
            self.assertIsNotNone(exit_stop)
            self.assertEqual(stack.teardown(), [])
        finally:
            shutil.rmtree(root)

    def assertGone(self, process):
        self.assertFalse(Path(f'/proc/{process.pid}').exists())  # reaped: not even a zombie

    def test_teardown_reaps_a_traced_client_on_timeout_error_or_signal(self):
        root = Path(tempfile.mkdtemp())
        try:
            stack = bench.Stack(root, dict(os.environ))
            never_resumed = stack.spawn('a', ['/bin/sleep', '30'], hold=True)  # error before run_held
            timed_out = stack.spawn('b', ['/bin/sleep', '30'], hold=True)
            self.assertEqual(bench.run_held(timed_out, timeout=0.2, hold=0)[0], None)
            exiting = stack.spawn('c', ['/bin/sleep', '30'], hold=True)
            bench.ptrace(bench.PTRACE_CONT, exiting.pid)
            os.kill(exiting.pid, signal.SIGTERM)  # a signal-delivery stop, which Popen.poll would take as exit
            time.sleep(0.1)
            sampler = stack.spawn('sampler', ['/bin/sleep', '30'], bind=False)
            self.assertEqual(stack.teardown(), [])
            for process in (never_resumed, timed_out, exiting, sampler):
                self.assertGone(process)
            self.assertEqual([p.returncode for p in (never_resumed, timed_out, exiting)], [-9, -9, -9])
        finally:
            shutil.rmtree(root)

    def test_native_x_server_keeps_the_vt_session_in_its_own_group(self):
        root = Path(tempfile.mkdtemp())
        try:
            stack = bench.Stack(root, dict(os.environ))
            server = stack.spawn('server', ['/bin/sleep', '30'], keep_session=True)
            time.sleep(0.1)
            self.assertEqual(os.getsid(server.pid), os.getsid(0))  # same controlling tty
            self.assertEqual(os.getpgid(server.pid), server.pid)  # teardown's killpg reaches only it
            ignored = int(Path(f'/proc/{server.pid}/status').read_text().split('SigIgn:')[1].split()[0], 16)
            self.assertTrue(ignored & 1 << signal.SIGTTOU - 1)
            self.assertIn(server.pid, bench.stack_processes({server.pid}))  # found by its group
            self.assertNotIn(os.getpid(), bench.stack_processes({server.pid}))
            self.assertEqual(stack.teardown(), [])
            self.assertEqual(server.returncode, -signal.SIGTERM)
        finally:
            shutil.rmtree(root)

    def test_signals_reach_the_client_through_run_held(self):
        root = Path(tempfile.mkdtemp())
        try:
            stack = bench.Stack(root, dict(os.environ))
            client = stack.spawn('client', ['/bin/sleep', '30'], hold=True)
            killer = subprocess.Popen(['/bin/sh', '-c', f'sleep 0.2; kill -TERM {client.pid}'])
            status, exit_stop = bench.run_held(client, timeout=10, hold=0)
            killer.wait()
            self.assertEqual(status, -signal.SIGTERM)
            self.assertIsNotNone(exit_stop)
            self.assertEqual(stack.teardown(), [])
            self.assertGone(client)
        finally:
            shutil.rmtree(root)


if __name__ == '__main__':
    unittest.main()
