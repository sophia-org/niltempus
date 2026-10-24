#!/usr/bin/env python3
"""Synthetic tests for cpuwin.py: no live processes beyond this interpreter."""
import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import types
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cpuwin  # noqa: E402

MS = 1_000_000
HEAD = dict(kind="header", version=1, clk_tck=100, interval_ns=10 * MS, boot_id="b",
            sampler_pid=1, procs={"session": dict(pid=11, starttime=100, comm="sophia"),
                                  "peer": dict(pid=12, starttime=200, comm="peer ring)")})
EXPECT = {"session": {"pid": 11, "starttime": 100}, "peer": {"pid": 12}}


def read(name, t0, **over):
    h = HEAD["procs"][name]
    # One tick per 10 ms for the session, half that for the peer.
    r = dict(pid=h["pid"], starttime=h["starttime"], comm=h["comm"],
             ticks=t0 // (10 * MS) if name == "session" else t0 // (20 * MS))
    r.update(over)
    return r


def samples(start_ms=0, end_ms=600, step_ms=10, skip=(), edit=None):
    out, seq = [], 0
    for t in range(start_ms, end_ms + 1, step_ms):
        if t in skip:
            continue
        t0 = t * MS
        rec = dict(kind="sample", seq=seq, t0=t0, t1=t0 + MS, self_cpu_ns=t0 // 100,
                   procs={n: read(n, t0) for n in HEAD["procs"]})
        if edit:
            edit(t, rec)
        out.append(rec)
        seq += 1
    return out


class Case(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory(dir=os.path.dirname(os.path.abspath(__file__)))

    def tearDown(self):
        self.dir.cleanup()

    def write(self, recs, name="run.jsonl", header=HEAD, footer=True, tail=""):
        path = os.path.join(self.dir.name, name)
        with open(path, "w") as f:
            for r in [header, *recs] + ([dict(kind="footer", samples=len(recs), late_slots=0,
                                              self_cpu_ns=0, stop="seconds")] if footer else []):
                f.write(json.dumps(r) + "\n")
            f.write(tail)
        return path

    def run_(self, path, a=105 * MS, b=505 * MS, expect=EXPECT, **kw):
        run = dict(name="r", arm="x", samples=path, start_frame=120, end_frame=2120,
                   anchor_start_ns=a, anchor_end_ns=b, expect=expect)
        return cpuwin.analyze_run(run, kw.get("max_gap_ms"))

    def refused(self, msg, path, **kw):
        with self.assertRaisesRegex(cpuwin.Refused, msg):
            self.run_(path, **kw)


class Bounds(Case):
    def test_exact_inner_outer_bounds(self):
        r = self.run_(self.write(samples()))
        # k=100ms (t1 101 <= 105), i=110, j=500 (t1 501 <= 505), l=510
        self.assertEqual(r["samples_used"], (10, 11, 50, 51))
        s = r["procs"]["session"]
        self.assertEqual((s["inner_ticks"], s["outer_ticks"]), (39, 41))
        self.assertEqual((s["lower_ns"], s["upper_ns"]), (370 * MS, 430 * MS))
        p = r["procs"]["peer"]  # ticks 5..25 inner, 5..25 outer
        self.assertEqual((p["inner_ticks"], p["outer_ticks"]), (20, 20))
        self.assertEqual((p["lower_ns"], p["upper_ns"]), (180 * MS, 220 * MS))
        self.assertEqual(r["intervals"], 2000)
        self.assertEqual(r["per_frame_us"], (550 * MS / 2000 / 1000, 650 * MS / 2000 / 1000))
        # the true session CPU over [A, B] (400 ms) lies inside its bounds
        self.assertLess(s["lower_ns"], 400 * MS)
        self.assertGreater(s["upper_ns"], 400 * MS)

    def test_fixed_window_accepts_idle_without_inventing_frame_counts(self):
        idle = lambda t, rec: [rec["procs"][n].update(ticks=0) for n in rec["procs"]]
        run = dict(name="idle", arm="candidate", samples=self.write(samples(edit=idle)),
                   window_mode="fixed", anchor_start_ns=105 * MS,
                   anchor_end_ns=505 * MS, expect=EXPECT)
        r = cpuwin.analyze_run(run)
        self.assertIsNone(r["intervals"])
        self.assertNotIn("per_frame_us", r)
        self.assertEqual(r["cpu_ms_per_s"], (0, 100))
        run["start_frame"] = 0
        with self.assertRaisesRegex(cpuwin.Refused, "frame denominators"):
            cpuwin.analyze_run(run)

    def test_fixed_window_preserves_identity_and_coverage_refusals(self):
        run = dict(name="idle", arm="candidate", samples=self.write(samples()),
                   window_mode="fixed", anchor_start_ns=105 * MS,
                   anchor_end_ns=505 * MS, expect=EXPECT)
        r = cpuwin.analyze_run(run)
        self.assertEqual(r["cpu_ms_per_s"], (1375, 1625))
        run["anchor_end_ns"] = 705 * MS
        with self.assertRaisesRegex(cpuwin.Refused, "after the end anchor"):
            cpuwin.analyze_run(run)
        run["anchor_end_ns"] = 505 * MS
        run["expect"] = {**EXPECT, "peer": {"pid": 13}}
        with self.assertRaisesRegex(cpuwin.Refused, "expected"):
            cpuwin.analyze_run(run)

    def test_fixed_window_cli_labels_units_and_refuses_mixed_modes(self):
        run = dict(name="idle", arm="candidate", samples=self.write(samples()),
                   window_mode="fixed", anchor_start_ns=105 * MS,
                   anchor_end_ns=505 * MS, expect=EXPECT)
        plan = os.path.join(self.dir.name, "fixed.json")
        out = os.path.join(self.dir.name, "report.json")
        with open(plan, "w") as f:
            json.dump(dict(runs=[run]), f)
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            self.assertEqual(cpuwin.cmd_analyze(types.SimpleNamespace(plan=plan, json=out)), 0)
        self.assertIn("ms/s", stdout.getvalue())
        self.assertNotIn("us/frame", stdout.getvalue())
        with open(out) as f:
            self.assertEqual(json.load(f)["arms"]["candidate"], [1375, 1625])
        with open(plan, "w") as f:
            json.dump(dict(runs=[run, dict(run, window_mode="frames")]), f)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(cpuwin.cmd_analyze(types.SimpleNamespace(plan=plan, json=None)), 1)

    def test_lower_bound_floors_at_zero(self):
        idle = lambda t, rec: [rec["procs"][n].update(ticks=0) for n in rec["procs"]]
        r = self.run_(self.write(samples(edit=idle)))
        self.assertEqual(r["lower_ns"], 0)
        self.assertEqual(r["upper_ns"], 2 * 2 * 10 * MS)

    def test_sample_straddling_an_anchor_is_used_for_neither_pair(self):
        r = self.run_(self.write(samples()), a=int(100.5 * MS), b=int(500.5 * MS))
        # the 100 ms and 500 ms samples straddle; k=90, i=110, j=490, l=510
        self.assertEqual(r["samples_used"], (9, 11, 49, 51))

    def test_reads_touching_an_anchor_count_on_their_side(self):
        r = self.run_(self.write(samples()), a=101 * MS, b=500 * MS)
        # t1 == A is outer at the start; t0 == B is outer at the end
        self.assertEqual(r["samples_used"], (10, 11, 49, 50))

    def test_all_head_anchor_uses_latest_head(self):
        r = self.run_(self.write(samples()), a=[95 * MS, 105 * MS], b=[505 * MS, 499 * MS])
        self.assertEqual(r["samples_used"], (10, 11, 50, 51))

    def test_labels_cadence_bracket_and_sampler_cost(self):
        r = self.run_(self.write(samples()))
        self.assertEqual(r["cadence_ns"]["p50"], 10 * MS)
        self.assertEqual(r["bracket_ns"]["max"], MS)
        self.assertGreater(r["sampler_cpu_ns"], 0)


class Identity(Case):
    def test_starttime_change_inside_window_refused(self):
        def edit(t, rec):
            if t >= 300:
                rec["procs"]["peer"]["starttime"] = 999
        self.refused("peer: identity changed at seq 30", self.write(samples(edit=edit)))

    def test_pid_change_inside_window_refused(self):
        def edit(t, rec):
            if t == 300:
                rec["procs"]["session"]["pid"] = 77
        self.refused("session: identity changed", self.write(samples(edit=edit)))

    def test_change_at_outer_sample_refused(self):
        def edit(t, rec):
            if t == 510:
                rec["procs"]["session"]["comm"] = "other"
        self.refused("session: identity changed at seq 51", self.write(samples(edit=edit)))

    def test_exit_inside_window_refused(self):
        def edit(t, rec):
            if t >= 400:
                rec["procs"]["peer"] = dict(pid=12, error="No such process")
        self.refused("peer: unreadable at seq 40", self.write(samples(edit=edit)))

    def test_normal_exit_after_window_keeps_run_without_footer(self):
        def edit(t, rec):
            if t >= 520:
                rec["procs"]["peer"] = dict(pid=12, error="No such process")
        r = self.run_(self.write(samples(edit=edit), footer=False))
        self.assertEqual(r["unclean_reads_outside_window"], 9)
        self.assertIsNone(r["late_slots"])

    def test_plan_binding_mismatch_refused(self):
        path = self.write(samples())
        self.refused("session: sampled pid/starttime 11/100 != expected 11/101", path,
                     expect={"session": {"pid": 11, "starttime": 101}, "peer": {"pid": 12}})
        self.refused("process names", path, expect={"session": {"pid": 11}})

    def test_counter_backwards_refused(self):
        def edit(t, rec):
            if t == 300:
                rec["procs"]["session"]["ticks"] = 0
        self.refused("session: counter went backwards at seq 30", self.write(samples(edit=edit)))


class Coverage(Case):
    def test_no_sample_before_start_anchor(self):
        self.refused("no sample completed before", self.write(samples(start_ms=110)))

    def test_no_sample_after_end_anchor(self):
        self.refused("no sample began after", self.write(samples(end_ms=500)))

    def test_gap_around_start_anchor(self):
        path = self.write(samples(skip=range(60, 150, 10)))
        self.refused("anchor coverage gap start_before", path)

    def test_gap_around_end_anchor_with_explicit_limit(self):
        path = self.write(samples(skip=(510, 520)))
        self.refused("anchor coverage gap end_after=25.000 ms > 20.000 ms", path, max_gap_ms=20)

    def test_no_pair_inside_window(self):
        path = self.write(samples(step_ms=10), name="s.jsonl")
        self.refused("no pair of samples lies inside", path, a=105 * MS, b=108 * MS, max_gap_ms=50)

    def test_sequence_gap_refused(self):
        recs = samples()
        del recs[20]
        self.refused("record sequence gap", self.write(recs))

    def test_overlapping_brackets_refused(self):
        recs = samples()
        recs[20]["t0"] = recs[19]["t0"]
        self.refused("out of order", self.write(recs))

    def test_truncated_record_refused(self):
        self.refused("unparsable record", self.write(samples(), footer=False, tail='{"kind":"sam'))


class Records(Case):
    FOOT = dict(kind="footer", samples=61, late_slots=0, self_cpu_ns=0, stop="seconds")

    def head(self, **over):
        h = json.loads(json.dumps(HEAD))
        h.update(over)
        return h

    def test_footer_present_and_missing_are_reported(self):
        self.assertEqual(self.run_(self.write(samples()))["footer"], "seconds")
        self.assertEqual(self.run_(self.write(samples(), footer=False))["footer"], "missing")

    def test_first_record_must_be_header(self):
        recs = samples()
        self.refused("first record is 'sample'", self.write([HEAD] + recs, header=recs[0]))

    def test_second_header_refused(self):
        recs = samples()
        recs.insert(5, HEAD)
        self.refused("second header", self.write(recs))

    def test_unknown_kind_refused(self):
        recs = samples()
        recs.insert(5, dict(kind="note"))
        self.refused("unknown record kind 'note'", self.write(recs))

    def test_record_after_footer_refused(self):
        tail = json.dumps(samples()[0]) + "\n"
        self.refused("record after the footer", self.write(samples(), tail=tail))

    def test_second_footer_refused(self):
        self.refused("record after the footer",
                     self.write(samples(), tail=json.dumps(self.FOOT) + "\n"))

    def test_blank_line_refused(self):
        self.refused("unparsable record", self.write(samples(), tail="\n"))

    def test_header_version_refused(self):
        for v in (2, True, None):
            self.refused("header version", self.write(samples(), header=self.head(version=v)))

    def test_nonpositive_clock_or_interval_refused(self):
        for over in (dict(clk_tck=0), dict(interval_ns=-1), dict(clk_tck=100.0)):
            self.refused("must be positive integers", self.write(samples(), header=self.head(**over)))

    def test_empty_process_set_refused(self):
        self.refused("names no processes", self.write([], header=self.head(procs={})))

    def test_one_pid_under_two_names_refused(self):
        h = self.head()
        h["procs"]["peer"]["pid"] = 11
        self.refused("one pid under two names", self.write(samples(), header=h))

    def test_sample_with_other_process_labels_refused(self):
        def edit(t, rec):
            if t == 50:
                rec["procs"]["extra"] = rec["procs"]["peer"]
        self.refused("processes differ from the header", self.write(samples(edit=edit)))

    def test_malformed_read_refused(self):
        def edit(t, rec):
            if t == 50:
                rec["procs"]["session"]["ticks"] = "5"
        self.refused("malformed read for 'session'", self.write(samples(edit=edit)))

    def test_footer_sample_count_mismatch_refused(self):
        self.refused("footer counts 60 samples, file has 61",
                     self.write(samples() + [dict(self.FOOT, samples=60)], footer=False))

    def test_footer_identity_changed_refused(self):
        self.refused("stopped on identity_changed",
                     self.write(samples() + [dict(self.FOOT, stop="identity_changed")], footer=False))

    def test_malformed_footer_refused(self):
        self.refused("malformed footer",
                     self.write(samples() + [dict(self.FOOT, stop="crashed")], footer=False))

    def test_sampler_refuses_one_pid_under_two_names(self):
        me = os.getpid()
        with open(f"/proc/{me}/stat") as fh:
            start = cpuwin.parse_stat(fh.read())[1]
        out = os.path.join(self.dir.name, "dup.jsonl")
        rc = subprocess.run([sys.executable, "-I", cpuwin.__file__, "sample", "--out", out,
                             "--seconds", "1", "--pid", f"a={me}:{start}", "--pid", f"b={me}:{start}"],
                            capture_output=True).returncode
        self.assertEqual(rc, cpuwin.EXIT_BINDING)
        self.assertFalse(os.path.exists(out))


class Plan(Case):
    def test_abba_means_and_difference(self):
        runs = []
        for n, (arm, scale) in enumerate([("sealed", 1), ("ring", 2), ("ring", 2), ("sealed", 1)]):
            def edit(t, rec, scale=scale):
                for p in rec["procs"].values():
                    p["ticks"] *= scale
            runs.append(dict(name=f"r{n}", arm=arm, samples=self.write(samples(edit=edit), name=f"r{n}.jsonl"),
                             start_frame=120, end_frame=2120, anchor_start_ns=105 * MS,
                             anchor_end_ns=505 * MS, expect=EXPECT))
        bad = dict(runs[0], name="bad", samples=self.write(samples(end_ms=500), name="bad.jsonl"))
        plan = os.path.join(self.dir.name, "plan.json")
        out = os.path.join(self.dir.name, "out.json")
        with open(plan, "w") as f:
            json.dump(dict(baseline="sealed", runs=runs + [bad]), f)
        with contextlib.redirect_stdout(io.StringIO()) as text:
            rc = cpuwin.cmd_analyze(types.SimpleNamespace(plan=plan, json=out))
        self.assertEqual(rc, 1)  # a refused run fails the plan
        self.assertIn("bad      sealed   4      REFUSED: no sample began after", text.getvalue())
        with open(out) as fh:
            res = json.load(fh)
        sl, su = res["arms"]["sealed"]
        rl, ru = res["arms"]["ring"]
        self.assertEqual(len([r for r in res["runs"] if r["arm"] == "sealed"]), 2)
        self.assertEqual(res["differences"]["ring-sealed"], [rl - su, ru - sl])
        self.assertLess(res["differences"]["ring-sealed"][0], res["differences"]["ring-sealed"][1])


class Parse(unittest.TestCase):
    def test_comm_with_parens_and_spaces(self):
        f = ["S"] + [str(n) for n in range(4, 53)]
        f[11], f[12], f[19] = "7", "5", "4242"
        self.assertEqual(cpuwin.parse_stat("12 (a) b( c) " + " ".join(f)), ("a) b( c", 4242, 7, 5))

    def test_own_stat_parses(self):
        with open(f"/proc/{os.getpid()}/stat") as fh:
            comm, start, ut, st = cpuwin.parse_stat(fh.read())
        self.assertGreater(start, 0)


if __name__ == "__main__":
    unittest.main()
