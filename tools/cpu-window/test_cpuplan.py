#!/usr/bin/env python3
"""Synthetic tests for cpuplan.py: fabricated run directories, no processes."""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cpuplan  # noqa: E402
import cpuwin  # noqa: E402

WARMUP, COUNTED = 120, 2000
END = WARMUP + COUNTED
RASTER_EXE = "/opt/sophia/bin/sophia-draw-raster"


def usec(g):
    return 1_000_000 + g * 1000  # completion of generation g


def ident(pid, start):
    return {"pid": pid, "start_time_ticks": start}


def time_proof(inode=7):
    node = {"time_namespace_inode": inode}
    return {"reference": {"inode": 7}, "host": node, "peer": node, "capturer": node,
            "monotonic_offset": [0, 0]}


def completion_line(g, **over):
    f = dict(schema=1, output=1, native_owner=1, native_frame=g + 4, heads=2,
             monotonic_usec=usec(g), timestamp_source="kernel", missing_kernel_timestamp=0,
             drm_timestamp_monotonic=1)
    f.update(over)
    f = {k: v for k, v in f.items() if v is not None}
    return "sophia_shell_native_completion " + " ".join(f"{k}={v}" for k, v in f.items())


JOURNAL_SOURCE = {"draw": "draw", "mixed": "mixed", "owned_ring": "ring"}


def write_run(root, source="draw", workload="continuous", edit=None):
    """A schema-2 run stage; edit(files) may change any file before writing."""
    stage = Path(root)
    stage.mkdir(parents=True)
    host = {"wrapper": ident(10, 100), "host": ident(11, 101), "supervisor": ident(12, 102),
            "peer": ident(13, 102), "time": time_proof()}
    files = {
        "panel-diagnostic.json": {"schema": 2, "warmup": WARMUP, "counted": COUNTED,
                                  "workload": workload, "plan": {"path": "plan.json", "sha256": "AUTO"},
                                  "target": {"card": "card0", "output": 1, "mode_millihz": 60000},
                                  "raster": {"path": RASTER_EXE, "sha256": "0" * 64}},
        "plan.json": {"schema": 1, "request": {"artifacts": {"sophia": {"sha256": "5" * 64}}}},
        "result.json": {"schema": 1, "status": "captured_only", "error": None},
        "environment.json": {"finish_error": None, "binding": dict(host)},
        "cpu-binding.json": dict(host),
        "native-samples.json": {
            "schema": 2, "verdict": "joined_only", "journal": {
                "source": source, "schema": 6,
                "draw": {"source": JOURNAL_SOURCE[source], "workload": workload}},
            "samples": [{"candidate_generation": g, "connection_epoch": 1, "content_grant_epoch": 1,
                         "output": 1, "native_owner": 1, "native_frame": g + 4, "heads": 2,
                         "completed_monotonic_usec": usec(g)} for g in range(1, END + 1)]},
        "native.log": [completion_line(g) for g in range(1, END + 1)],
    }
    if source != "owned_ring":
        files["cpu-raster-binding.json"] = {
            "wrapper": ident(10, 100), "host": ident(11, 101), "supervisor": ident(20, 300),
            "peer": ident(21, 301), "peer_executable": {"prepared_path": RASTER_EXE},
            "time": time_proof()}
        files["cpu-raster-lifetime.json"] = {"pid": 21, "starttime": 301,
                                             "first_bound_usec": usec(5), "last_bound_usec": usec(END + 30),
                                             "first_ended_usec": usec(END + 40)}
        files["cpu-raster-ended.json"] = {"monotonic_usec": usec(END + 40)}
    if edit:
        edit(files)
    for name, value in files.items():
        if value is None:
            continue
        text = "\n".join(value) + "\n" if name == "native.log" else json.dumps(value)
        (stage / name).write_text(text)
    descriptor = files["panel-diagnostic.json"]
    if descriptor["plan"]["sha256"] == "AUTO":
        descriptor["plan"]["sha256"] = hashlib.sha256((stage / "plan.json").read_bytes()).hexdigest()
        (stage / "panel-diagnostic.json").write_text(json.dumps(descriptor))
    return stage


def sampler_file(path, procs):
    """cpuwin samples covering both anchors at 5 ms; every process alive."""
    ms = 1_000_000
    with open(path, "w") as f:
        f.write(json.dumps(dict(kind="header", version=1, clk_tck=100, interval_ns=5 * ms,
                                boot_id="b", sampler_pid=1, procs=procs)) + "\n")
        for seq, t in enumerate(range(usec(100) * 1000, usec(END + 20) * 1000, 5 * ms)):
            reads = {n: dict(pid=p["pid"], starttime=p["starttime"], comm=p["comm"], ticks=seq // 2)
                     for n, p in procs.items()}
            f.write(json.dumps(dict(kind="sample", seq=seq, t0=t, t1=t + 20_000, self_cpu_ns=0,
                                    procs=reads)) + "\n")


HEADER = {"session": dict(pid=11, starttime=101, comm="sophia"),
          "peer": dict(pid=13, starttime=102, comm="peer"),
          "raster": dict(pid=21, starttime=301, comm="sophia-draw-ras")}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.count = 0

    def tearDown(self):
        self.tmp.cleanup()

    def run_dir(self, **kw):
        self.count += 1
        return write_run(self.root / f"r{self.count}", **kw)

    def refused(self, needle, **kw):
        with self.assertRaises(cpuplan.Refused) as cm:
            cpuplan.build([("r", "draw", self.run_dir(**kw))])
        self.assertIn(needle, str(cm.exception))


def write_run_gpu(root, source="sealed", sophia="5" * 64, peer="8" * 64, bar_height=24, edit=None):
    """A schema-3 GPU-producer run stage (schema-7 SDK ring, otherwise schema-2)."""
    stage = Path(root)
    stage.mkdir(parents=True)
    host = {"wrapper": ident(10, 100), "host": ident(11, 101), "supervisor": ident(12, 102),
            "peer": ident(13, 102), "time": time_proof()}
    files = {
        "panel-diagnostic.json": {"schema": 3, "warmup": WARMUP, "counted": COUNTED,
                                  "bar_height": bar_height,
                                  "plan": {"path": "plan.json", "sha256": "AUTO"},
                                  "target": {"card": "card0", "output": 1, "mode_millihz": 60000}},
        "plan.json": {"schema": 1, "request": {
            "artifacts": {"sophia": {"sha256": sophia}, "peer": {"sha256": peer}}, "producer": "gpu",
            "source": source, "wire": "ipc" if source == "bytes" else "files"}},
        "result.json": {"schema": 1, "status": "captured_only", "error": None},
        "environment.json": {"finish_error": None, "binding": dict(host)},
        "cpu-binding.json": dict(host),
        "native-samples.json": {
            "schema": 2, "verdict": "joined_only",
            "journal": {"source": source, "schema": 7 if source == "owned_ring" else 2,
                        "wire": "ipc" if source == "bytes" else "files"},
            "samples": [{"candidate_generation": g, "connection_epoch": 1, "content_grant_epoch": 1,
                         "output": 1, "native_owner": 1, "native_frame": g + 4, "heads": 2,
                         "completed_monotonic_usec": usec(g)} for g in range(1, END + 1)]},
        "native.log": [completion_line(g) for g in range(1, END + 1)],
    }
    if edit:
        edit(files)
    for name, value in files.items():
        if value is None:
            continue
        text = "\n".join(value) + "\n" if name == "native.log" else json.dumps(value)
        (stage / name).write_text(text)
    descriptor = files["panel-diagnostic.json"]
    if descriptor["plan"]["sha256"] == "AUTO":
        descriptor["plan"]["sha256"] = hashlib.sha256((stage / "plan.json").read_bytes()).hexdigest()
        (stage / "panel-diagnostic.json").write_text(json.dumps(descriptor))
    return stage


class GpuPositive(Base):
    def gpu_dir(self, **kw):
        self.count += 1
        return write_run_gpu(self.root / f"g{self.count}", **kw)

    def test_sdk_ring_and_dma_require_the_same_host_and_peer(self):
        r = self.gpu_dir(source="owned_ring")
        g = self.gpu_dir(source="dma_buf")
        plan = cpuplan.build([("r1", "r", r), ("g1", "g", g)], "r")
        self.assertEqual(len(plan["runs"]), 2)
        for kw, needle in [
            ({"sophia": "6" * 64}, "sophia_sha256"),
            ({"peer": "9" * 64}, "peer_sha256"),
            ({"peer": None}, "peer_sha256"),
        ]:
            with self.assertRaises(cpuplan.Refused) as cm:
                cpuplan.build([("r1", "r", r), ("g1", "g", self.gpu_dir(source="dma_buf", **kw))])
            self.assertIn(needle, str(cm.exception))
        for schema in (2, 5, 6):
            bad = self.gpu_dir(source="owned_ring", edit=setter(
                ["native-samples.json", "journal", "schema"], schema))
            with self.assertRaises(cpuplan.Refused):
                cpuplan.build([("r1", "r", bad)])

    def test_gpu_arms_split_hosts_per_arm_and_analyze(self):
        i = self.gpu_dir(source="bytes", sophia="5" * 64)
        s = self.gpu_dir(source="sealed", sophia="5" * 64)
        g = self.gpu_dir(source="dma_buf", sophia="6" * 64)
        plan = cpuplan.build([("i1", "i", i), ("s1", "s", s), ("g1", "g", g)], "i")
        for run in plan["runs"]:
            self.assertEqual(set(run["expect"]), {"session", "peer"})
        sampler_file(g / "cpu-window.jsonl",
                     {"session": HEADER["session"], "peer": HEADER["peer"]})
        result = cpuwin.analyze_run(plan["runs"][2], plan["max_gap_ms"])
        self.assertEqual(set(result["procs"]), {"session", "peer"})
        self.assertEqual(result["intervals"], COUNTED)

    def test_gpu_arm_mixing_hosts_or_heights_is_refused(self):
        a = self.gpu_dir(source="sealed", sophia="5" * 64)
        b = self.gpu_dir(source="sealed", sophia="6" * 64)
        with self.assertRaises(cpuplan.Refused) as cm:
            cpuplan.build([("s1", "s", a), ("s2", "s", b)])
        self.assertIn("mixes Session binaries", str(cm.exception))
        tall = self.gpu_dir(source="sealed", sophia="5" * 64, bar_height=512)
        with self.assertRaises(cpuplan.Refused) as cm:
            cpuplan.build([("s1", "s", a), ("s3", "s", tall)])
        self.assertIn("differ in bar_height", str(cm.exception))

    def test_gpu_descriptor_and_journal_shape_refusals(self):
        def refused(needle, **kw):
            with self.assertRaises(cpuplan.Refused) as cm:
                cpuplan.build([("s1", "s", self.gpu_dir(**kw))])
            self.assertIn(needle, str(cm.exception))
        refused("GPU journal", source="draw")
        refused("GPU journal", edit=setter(["native-samples.json", "journal", "schema"], 6))
        refused("GPU schema-3", edit=setter(["panel-diagnostic.json", "bar_height"], 108))
        refused("GPU schema-3", edit=lambda f: f["panel-diagnostic.json"].update(
            raster={"path": RASTER_EXE, "sha256": "0" * 64}))
        refused("non-raster arm", edit=lambda f: f.update({"cpu-raster-binding.json": {}}))

    def test_gpu_unbound_or_mismatched_request_is_refused(self):
        def refused(needle, **kw):
            with self.assertRaises(cpuplan.Refused) as cm:
                cpuplan.build([("s1", "s", self.gpu_dir(**kw))])
            self.assertIn(needle, str(cm.exception))
        # Missing producer (CPU/unbound request labeled GPU).
        refused("producer=gpu", edit=lambda f: f["plan.json"]["request"].pop("producer"))
        refused("producer=gpu", edit=setter(["plan.json", "request", "producer"], "cpu"))
        # Request source or wire disagreeing with the journal.
        refused("producer=gpu", edit=setter(["plan.json", "request", "source"], "dma_buf"))
        refused("producer=gpu", edit=setter(["plan.json", "request", "wire"], "ipc"))
        refused("producer=gpu", source="bytes",
                edit=setter(["plan.json", "request", "wire"], "files"))
        refused("producer=gpu", source="bytes",
                edit=setter(["native-samples.json", "journal", "wire"], "files"))


class Positive(Base):
    def test_plain_schema1_ring_report(self):
        def plain(files):
            descriptor = files["panel-diagnostic.json"]
            descriptor["schema"] = 1
            del descriptor["raster"], descriptor["workload"]
            files["native-samples.json"]["journal"] = {"schema": 5, "source": "owned_ring"}
        stage = self.run_dir(source="owned_ring", edit=plain)
        plan = cpuplan.build([("ring", "ring", stage)])
        self.assertEqual(set(plan["runs"][0]["expect"]), {"session", "peer"})
        descriptor = stage / "panel-diagnostic.json"
        value = json.loads(descriptor.read_text())
        value["raster"] = {"path": RASTER_EXE, "sha256": "0" * 64}
        descriptor.write_text(json.dumps(value))
        with self.assertRaises(cpuplan.Refused):
            cpuplan.build([("ring", "ring", stage)])
        del value["raster"]
        descriptor.write_text(json.dumps(value))
        native = stage / "native-samples.json"
        value = json.loads(native.read_text())
        value["journal"]["schema"] = 6
        native.write_text(json.dumps(value))
        with self.assertRaises(cpuplan.Refused):
            cpuplan.build([("ring", "ring", stage)])

    def test_draw_plan_derives_identity_and_kernel_anchors(self):
        run = cpuplan.build([("d1", "draw", self.run_dir())], "draw")["runs"][0]
        self.assertEqual(run["expect"], {"session": {"pid": 11, "starttime": 101},
                                         "peer": {"pid": 13, "starttime": 102},
                                         "raster": {"pid": 21, "starttime": 301}})
        self.assertEqual((run["start_frame"], run["end_frame"]), (WARMUP, END))
        self.assertEqual((run["anchor_start_ns"], run["anchor_end_ns"]),
                         (usec(WARMUP) * 1000, usec(END) * 1000))

    def test_unchanged_cpuwin_analyzes_the_built_plan(self):
        stage = self.run_dir()
        sampler_file(stage / "cpu-window.jsonl", HEADER)
        plan = cpuplan.build([("d1", "draw", stage)])
        result = cpuwin.analyze_run(plan["runs"][0], plan["max_gap_ms"])
        self.assertEqual(set(result["procs"]), {"session", "peer", "raster"})
        self.assertEqual(result["intervals"], COUNTED)

    def test_sampler_header_never_supplies_identity(self):
        stage = self.run_dir()
        wrong = dict(HEADER, raster=dict(pid=21, starttime=999, comm="sophia-draw-ras"))
        sampler_file(stage / "cpu-window.jsonl", wrong)
        plan = cpuplan.build([("d1", "draw", stage)])
        self.assertEqual(plan["runs"][0]["expect"]["raster"]["starttime"], 301)
        with self.assertRaises(cpuwin.Refused):
            cpuwin.analyze_run(plan["runs"][0], plan["max_gap_ms"])

    def test_ring_arm_has_no_raster_and_cli_writes_once(self):
        ring, draw = self.run_dir(source="owned_ring"), self.run_dir()
        out = self.root / "plan.json"
        tool = Path(__file__).with_name("cpuplan.py")
        args = [sys.executable, "-I", str(tool), "--out", str(out), "--baseline", "ring",
                "--run", "a1", "ring", str(ring), "--run", "b1", "draw", str(draw)]
        self.assertEqual(subprocess.run(args, capture_output=True).returncode, 0)
        plan = json.loads(out.read_text())
        self.assertEqual(set(plan["runs"][0]["expect"]), {"session", "peer"})
        self.assertEqual(plan["baseline"], "ring")
        self.assertNotEqual(subprocess.run(args, capture_output=True).returncode, 0)


def setter(path, value):
    def edit(files):
        target = files
        for key in path[:-1]:
            target = target[key]
        target[path[-1]] = value
    return edit


class Negative(Base):
    def test_profile_cannot_become_a_comparison_arm(self):
        self.refused("diagnostic attribution", edit=setter(
            ["plan.json", "request", "instrumentation"], "capture-profile"))

    def test_old_completion_without_monotonic_capability(self):
        self.refused("drm_timestamp_monotonic", edit=setter(
            ["native.log", WARMUP - 1], completion_line(WARMUP, drm_timestamp_monotonic=None)))

    def test_completion_on_a_non_monotonic_card(self):
        self.refused("not a kernel CLOCK_MONOTONIC", edit=setter(
            ["native.log", END - 1], completion_line(END, drm_timestamp_monotonic=0)))

    def test_fallback_completion(self):
        self.refused("not a kernel CLOCK_MONOTONIC", edit=setter(
            ["native.log", WARMUP - 1], completion_line(WARMUP, timestamp_source="observation_fallback")))

    def test_joined_anchor_differs_from_its_log_line(self):
        self.refused("differs from its native.log", edit=setter(
            ["native.log", END - 1], completion_line(END, monotonic_usec=usec(END) + 1)))

    def test_contradictory_log_lines(self):
        def edit(files):
            files["native.log"].append(completion_line(WARMUP, heads=1))
        self.refused("contradictory completion", edit=edit)

    def test_failed_capture(self):
        self.refused("capture failed", edit=setter(["result.json", "error"], "sampler failed"))

    def test_schema_one_descriptor(self):
        self.refused("schema-2", edit=setter(["panel-diagnostic.json", "schema"], 1))

    def test_missing_generation(self):
        self.refused("generations", edit=lambda f: f["native-samples.json"]["samples"].pop(7))

    def test_reconnect_inside_the_run(self):
        self.refused("connection", edit=setter(
            ["native-samples.json", "samples", 1500, "connection_epoch"], 2))

    def test_cpu_binding_disagrees_with_the_monitor(self):
        self.refused("environment monitor", edit=setter(["cpu-binding.json", "peer"], ident(14, 102)))

    def test_missing_starttime(self):
        self.refused("start_time_ticks", edit=lambda f: f["cpu-binding.json"]["host"].pop("start_time_ticks"))

    def test_foreign_time_namespace(self):
        self.refused("namespace", edit=setter(["cpu-raster-binding.json", "time"], time_proof(8)))

    def test_helper_of_another_session(self):
        self.refused("host", edit=setter(["cpu-raster-binding.json", "host"], ident(30, 101)))

    def test_unpinned_helper_executable(self):
        self.refused("pinned", edit=setter(
            ["cpu-raster-binding.json", "peer_executable", "prepared_path"], "/tmp/other"))

    def test_missing_helper_in_draw_arm(self):
        self.refused("cpu-raster-binding.json", edit=setter(["cpu-raster-binding.json"], None))

    def test_replaced_helper(self):
        self.refused("another helper", edit=setter(["cpu-raster-lifetime.json", "starttime"], 302))

    def test_helper_bound_after_window_start(self):
        self.refused("after the window start", edit=setter(
            ["cpu-raster-lifetime.json", "first_bound_usec"], usec(WARMUP) + 1))

    def test_helper_not_alive_at_window_end(self):
        self.refused("window end", edit=setter(
            ["cpu-raster-lifetime.json", "last_bound_usec"], usec(END) - 1))

    def test_missing_lifetime_bound(self):
        self.refused("malformed", edit=setter(["cpu-raster-lifetime.json", "last_bound_usec"], None))

    def test_ended_receipt_disagrees(self):
        self.refused("cpu-raster-ended.json", edit=setter(
            ["cpu-raster-ended.json", "monotonic_usec"], usec(END + 41)))

    def test_ring_arm_with_a_helper(self):
        def edit(files):
            files["cpu-raster-binding.json"] = {}
        self.refused("non-raster arm", source="owned_ring", edit=edit)

    def test_descriptor_workload_contradicts_the_draw_journal(self):
        self.refused("draw journal", edit=setter(["native-samples.json", "journal", "draw", "workload"], "bursty"))

    def test_journal_draw_source_contradicts_the_plan_source(self):
        self.refused("draw journal", edit=setter(["native-samples.json", "journal", "draw", "source"], "ring"))

    def test_unpinned_native_plan(self):
        self.refused("pinned plan", edit=setter(["panel-diagnostic.json", "plan", "sha256"], "f" * 64))

    def test_baseline_with_another_workload_under_another_label(self):
        with self.assertRaises(cpuplan.Refused) as cm:
            cpuplan.build([("a1", "ring", self.run_dir(source="owned_ring", workload="retained")),
                           ("b1", "draw", self.run_dir(workload="continuous"))], "ring")
        self.assertIn("differ in workload", str(cm.exception))

    def test_compared_runs_with_another_session_binary_or_target(self):
        other = self.run_dir(edit=setter(["plan.json", "request", "artifacts", "sophia", "sha256"], "6" * 64))
        moved = self.run_dir(edit=setter(["panel-diagnostic.json", "target", "output"], 2))
        with self.assertRaises(cpuplan.Refused) as cm:
            cpuplan.build([("a1", "ring", self.run_dir(source="owned_ring")), ("b1", "draw", other),
                           ("b2", "draw", moved)], "ring")
        self.assertIn("differ in sophia_sha256", str(cm.exception))
        self.assertIn("differ in target", str(cm.exception))

    def test_arm_mixing_sources_and_unknown_baseline(self):
        with self.assertRaises(cpuplan.Refused) as cm:
            cpuplan.build([("a", "x", self.run_dir(source="owned_ring")),
                           ("b", "x", self.run_dir())], "y")
        self.assertIn("mixes", str(cm.exception))
        self.assertIn("baseline", str(cm.exception))


if __name__ == "__main__":
    unittest.main()
