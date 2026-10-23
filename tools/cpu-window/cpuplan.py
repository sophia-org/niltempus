#!/usr/bin/env python3
"""Build a cpuwin analysis plan from panel diagnostic run directories.

  cpuplan.py --out PLAN.json [--baseline ARM] --run NAME ARM RUN_DIR [--run ...]

RUN_DIR is the native run stage (it holds panel-diagnostic.json,
native-samples.json and cpu-window.jsonl). Every expectation comes from the
observer's evidence, never from the sampler header:

  session, peer  cpu-binding.json host/peer, matched by environment.json's
                 independent monitor binding
  raster         cpu-raster-binding.json peer, which must share the Session's
                 host and wrapper, run the descriptor's pinned helper, and
                 match cpu-raster-lifetime.json with
                 first_bound <= A and last_bound >= B
  A, B           the joined all-head completions of generations warmup and
                 warmup + counted, each matched to its native.log line with
                 timestamp_source=kernel, missing_kernel_timestamp=0 and
                 drm_timestamp_monotonic=1; microseconds become ns (x1000)

Every starttime is required. A failed capture, a connection or grant
change, a helper in the ring arm, a missing helper elsewhere, or any
mismatch refuses the whole plan; nothing is written. Analyze the result
with the unchanged `cpuwin.py analyze PLAN.json`.

Compared runs share workload, native target, bar height and helper pin.
Historical GPU I/S/G permits a different Session per arm; GPU owned-ring
comparisons require one Session and peer binary across arms. Other comparisons
require one Session. One boot, machine state and run order remain recipe checks.
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

COMPLETION = "sophia_shell_native_completion"
COMPLETION_FIELDS = {"schema", "output", "native_owner", "native_frame", "heads",
                     "monotonic_usec", "timestamp_source", "missing_kernel_timestamp",
                     "drm_timestamp_monotonic"}
RASTER_SOURCES = {"draw", "mixed"}
SOURCES = RASTER_SOURCES | {"owned_ring"}
# Schema-3 GPU arms: fresh schema-2 or SDK ring schema-7, no raster helper.
GPU_SOURCES = {"bytes", "sealed", "owned_ring", "dma_buf"}
DRAW_SOURCES = {"draw": "draw", "mixed": "mixed", "ring": "owned_ring"}  # journal -> plan
WORKLOADS = {"retained", "continuous", "bursty"}
MAX_GAP_MS = 20


class Refused(Exception):
    pass


def is_int(v, low=0):
    return isinstance(v, int) and not isinstance(v, bool) and v >= low


def load(path):
    try:
        return json.loads(path.read_bytes())
    except (OSError, json.JSONDecodeError) as e:
        raise Refused(f"{path.name}: {e}")


def identity(binding, role, where):
    p = binding.get(role) if isinstance(binding, dict) else None
    if not (isinstance(p, dict) and is_int(p.get("pid"), 1) and is_int(p.get("start_time_ticks"), 1)):
        raise Refused(f"{where}: {role} needs pid and start_time_ticks")
    return {"pid": p["pid"], "starttime": p["start_time_ticks"]}


def same_time_namespace(binding, where):
    """Host, peer and capturer share the reference time namespace, zero offset."""
    t = binding.get("time") or {}
    ref = (t.get("reference") or {}).get("inode")
    if not is_int(ref, 1) or t.get("monotonic_offset") != [0, 0] or any(
            (t.get(role) or {}).get("time_namespace_inode") != ref
            for role in ("host", "peer", "capturer")):
        raise Refused(f"{where}: no shared zero-offset CLOCK_MONOTONIC namespace proof")


def completions(text):
    """(output, native_owner, native_frame) -> fields, refusing contradictions."""
    found = {}
    for line in text.splitlines():
        line = line.rsplit("\t", 1)[-1]  # bounded captures prepend TSV provenance
        words = line.split()
        if not words or words[0] != COMPLETION:
            continue
        fields = dict(w.split("=", 1) for w in words[1:] if "=" in w)
        if len(fields) != len(words) - 1:
            raise Refused("native.log: malformed completion field")
        key = tuple(fields.get(k) for k in ("output", "native_owner", "native_frame"))
        if found.setdefault(key, fields) != fields:
            raise Refused(f"native.log: contradictory completion {key}")
    return found


def anchor_ns(sample, lines, generation):
    """The joined completion, proven kernel CLOCK_MONOTONIC by its own line."""
    key = tuple(str(sample[k]) for k in ("output", "native_owner", "native_frame"))
    line = lines.get(key)
    if line is None:
        raise Refused(f"generation {generation}: no native.log completion {key}")
    if set(line) != COMPLETION_FIELDS:
        missing = sorted(COMPLETION_FIELDS - set(line))
        raise Refused(f"generation {generation}: completion fields differ"
                      + (f"; missing {missing} (no recorded DRM monotonic capability)" if missing else ""))
    if (line["timestamp_source"], line["missing_kernel_timestamp"], line["drm_timestamp_monotonic"]) \
            != ("kernel", "0", "1"):
        raise Refused(f"generation {generation}: completion is not a kernel CLOCK_MONOTONIC timestamp")
    if (line["heads"], line["monotonic_usec"]) != (str(sample["heads"]), str(sample["completed_monotonic_usec"])):
        raise Refused(f"generation {generation}: joined sample differs from its native.log line")
    return sample["completed_monotonic_usec"] * 1000


def build_run(name, arm, stage):
    stage = Path(stage).resolve()
    d = load(stage / "panel-diagnostic.json")
    plain_ring = d.get("schema") == 1
    gpu = d.get("schema") == 3
    if d.get("schema") not in (1, 2, 3) or not is_int(d.get("warmup"), 1) or not is_int(d.get("counted"), 1) \
            or (not plain_ring and not gpu and not isinstance(d.get("raster"), dict)) \
            or ((plain_ring or gpu) and ("raster" in d or "workload" in d)) \
            or (gpu and d.get("bar_height") not in (24, 512)) \
            or (not gpu and "bar_height" in d):
        raise Refused("panel-diagnostic.json: expected a plain schema-1, pinned-helper schema-2 or GPU schema-3 descriptor")
    start, end = d["warmup"], d["warmup"] + d["counted"]
    plan_bytes = (stage / "plan.json").read_bytes()
    if hashlib.sha256(plan_bytes).hexdigest() != (d.get("plan") or {}).get("sha256"):
        raise Refused("plan.json differs from the descriptor's pinned plan")
    request = json.loads(plan_bytes).get("request") or {}
    compared = {"workload": d.get("workload"), "target": d.get("target"),
                "sophia_sha256": ((request.get("artifacts") or {}).get("sophia") or {}).get("sha256"),
                "peer_sha256": ((request.get("artifacts") or {}).get("peer") or {}).get("sha256"),
                "bar_height": d.get("bar_height"),
                "raster_sha256": None if (plain_ring or gpu) else d["raster"].get("sha256")}
    if not (isinstance(compared["sophia_sha256"], str) and (plain_ring or gpu or isinstance(compared["raster_sha256"], str))
            and isinstance(compared["target"], dict)):
        raise Refused("descriptor or plan lacks the target, Session or helper pin")

    result = load(stage / "result.json")
    if result.get("status") != "captured_only" or result.get("error") is not None:
        raise Refused(f"capture failed: {result.get('error')!r}")
    env = load(stage / "environment.json")
    if env.get("finish_error") is not None:
        raise Refused(f"environment monitor failed: {env.get('finish_error')!r}")

    native = load(stage / "native-samples.json")
    journal = native.get("journal") or {}
    source = journal.get("source")
    if native.get("schema") != 2 or native.get("verdict") != "joined_only" \
            or (source not in SOURCES and not (gpu and source in GPU_SOURCES)):
        raise Refused(f"native-samples.json: want a schema-2 joined_only {sorted(SOURCES | GPU_SOURCES)} arm")
    draw = journal.get("draw") or {}
    if plain_ring:
        if journal.get("schema") != 5 or source != "owned_ring" or draw \
                or journal.get("workload", "fresh") != "fresh" \
                or journal.get("interval_msec") is not None or journal.get("retained_resources") is not None:
            raise Refused("schema-1 descriptor requires a fresh schema-5 owned-ring journal")
    elif gpu:
        expected_schema = 7 if source == "owned_ring" else 2
        if journal.get("schema") != expected_schema or source not in GPU_SOURCES or draw \
                or journal.get("workload", "fresh") != "fresh" \
                or journal.get("interval_msec") is not None or journal.get("retained_resources") is not None:
            raise Refused("schema-3 descriptor requires a fresh GPU journal (schema-7 SDK ring, otherwise schema-2)")
        # The journal alone cannot prove the GPU binding: the pinned
        # request must declare it and agree on source and wire.
        want_wire = "ipc" if source == "bytes" else "files"
        if request.get("producer") != "gpu" \
                or request.get("source") != source \
                or request.get("wire") != want_wire \
                or journal.get("wire") != want_wire:
            raise Refused("schema-3 descriptor requires request producer=gpu with source and wire matching the GPU journal")
    elif journal.get("schema") != 6 or DRAW_SOURCES.get(draw.get("source")) != source \
            or d.get("workload") not in WORKLOADS or draw.get("workload") != d.get("workload"):
        raise Refused("descriptor workload or source differs from the schema-6 draw journal")
    samples = {s.get("candidate_generation"): s for s in native.get("samples", [])}
    if sorted(samples) != list(range(1, end + 1)) or len(native["samples"]) != end:
        raise Refused(f"native-samples.json: generations are not exactly 1..{end}")
    if len({(s["connection_epoch"], s["content_grant_epoch"], s["output"]) for s in samples.values()}) != 1:
        raise Refused("connection, grant or output changed during the run")
    lines = completions((stage / "native.log").read_text())
    a, b = anchor_ns(samples[start], lines, start), anchor_ns(samples[end], lines, end)
    if not a < b:
        raise Refused("anchors do not increase")

    binding = load(stage / "cpu-binding.json")
    monitor = env.get("binding") or {}
    for role in ("wrapper", "host", "peer"):
        if identity(binding, role, "cpu-binding.json") != identity(monitor, role, "environment.json"):
            raise Refused(f"cpu-binding.json {role} differs from the environment monitor's binding")
    same_time_namespace(binding, "cpu-binding.json")
    expect = {"session": identity(binding, "host", "cpu-binding.json"),
              "peer": identity(binding, "peer", "cpu-binding.json")}
    inputs = ["panel-diagnostic.json", "result.json", "environment.json", "native-samples.json",
              "native.log", "cpu-binding.json"]

    raster_files = [f for f in ("cpu-raster-binding.json", "cpu-raster-lifetime.json", "cpu-raster-ended.json")
                    if (stage / f).exists()]
    if source not in RASTER_SOURCES:
        if raster_files:
            raise Refused(f"non-raster arm has raster helper evidence: {raster_files}")
    else:
        rb = load(stage / "cpu-raster-binding.json")
        for role in ("wrapper", "host"):
            if identity(rb, role, "cpu-raster-binding.json") != identity(binding, role, "cpu-binding.json"):
                raise Refused(f"raster helper {role} is not the bound Session's")
        same_time_namespace(rb, "cpu-raster-binding.json")
        if (rb.get("peer_executable") or {}).get("prepared_path") != d["raster"].get("path"):
            raise Refused("raster helper is not the descriptor's pinned executable")
        raster = identity(rb, "peer", "cpu-raster-binding.json")
        if raster["pid"] in (expect["session"]["pid"], expect["peer"]["pid"]):
            raise Refused("raster helper pid collides with Session or peer")
        life = load(stage / "cpu-raster-lifetime.json")
        if {"pid": life.get("pid"), "starttime": life.get("starttime")} != raster:
            raise Refused("cpu-raster-lifetime.json names another helper than its binding")
        first, last, ended = (life.get(k) for k in ("first_bound_usec", "last_bound_usec", "first_ended_usec"))
        if not (is_int(first, 1) and is_int(last, 1)) or not (ended is None or is_int(ended, 1)):
            raise Refused("cpu-raster-lifetime.json: bound times missing or malformed")
        if first * 1000 > a:
            raise Refused("raster helper was first bound after the window start")
        if last * 1000 < b:
            raise Refused("raster helper was not proven alive at the window end")
        if ended is not None and ended < last:
            raise Refused("raster helper ended before its last bound observation")
        if "cpu-raster-ended.json" in raster_files:
            if ended is None or load(stage / "cpu-raster-ended.json").get("monotonic_usec") != ended:
                raise Refused("cpu-raster-ended.json differs from the lifetime record")
        expect["raster"] = raster
        inputs += raster_files

    run = {"name": name, "arm": arm, "samples": str(stage / "cpu-window.jsonl"),
           "start_frame": start, "end_frame": end, "anchor_start_ns": a, "anchor_end_ns": b,
           "expect": expect}
    # Ignored by cpuwin; ties the plan to the exact evidence it was derived from.
    run["evidence"] = {"stage": str(stage), "source": source, "compared": compared,
                       "clock": "kernel page-flip, drm_timestamp_monotonic=1 on both anchor completions",
                       "sha256": {f: hashlib.sha256((stage / f).read_bytes()).hexdigest() for f in inputs}}
    return run


def build(runs, baseline=None):
    plan, errors, arms, hosts = [], [], {}, {}
    for name, arm, stage in runs:
        try:
            run = build_run(name, arm, stage)
        except Refused as e:
            errors.append(f"{name}: {e}")
            continue
        source = run["evidence"]["source"]
        if arms.setdefault(arm, source) != source:
            errors.append(f"{name}: arm {arm!r} mixes {arms[arm]} and {source}")
        host = run["evidence"]["compared"]["sophia_sha256"]
        if hosts.setdefault(arm, host) != host:
            errors.append(f"{name}: arm {arm!r} mixes Session binaries")
        plan.append(run)
    # Historical I/S/G split hosts. The current ring comparison must use one host.
    gpu_plan = plan and all(r["evidence"]["compared"]["bar_height"] is not None for r in plan)
    for key in ("workload", "target", "raster_sha256", "bar_height"):
        if len({json.dumps(r["evidence"]["compared"][key], sort_keys=True) for r in plan}) > 1:
            errors.append(f"compared runs differ in {key}")
    matched_ring = gpu_plan and any(r["evidence"]["source"] == "owned_ring" for r in plan)
    if (not gpu_plan or matched_ring) and len({r["evidence"]["compared"]["sophia_sha256"] for r in plan}) > 1:
        errors.append("compared runs differ in sophia_sha256")
    if matched_ring:
        peers = [r["evidence"]["compared"]["peer_sha256"] for r in plan]
        if any(not isinstance(p, str) or len(p) != 64 for p in peers) or len(set(peers)) != 1:
            errors.append("GPU owned-ring comparison requires one pinned peer_sha256")
    if len({n for n, _, _ in runs}) != len(runs):
        errors.append("run names must be unique")
    if baseline is not None and baseline not in {a for _, a, _ in runs}:
        errors.append(f"baseline {baseline!r} names no arm")
    if errors:
        raise Refused("; ".join(errors))
    out = {"max_gap_ms": MAX_GAP_MS, "runs": plan}
    if baseline is not None:
        out["baseline"] = baseline
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--baseline")
    ap.add_argument("--run", nargs=3, action="append", required=True, metavar=("NAME", "ARM", "RUN_DIR"))
    args = ap.parse_args()
    try:
        plan = build([tuple(r) for r in args.run], args.baseline)
    except Refused as e:
        print(f"refused: {e}", file=sys.stderr)
        return 1
    with open(args.out, "x") as f:  # never overwrite an earlier plan
        json.dump(plan, f, indent=1)
        f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
