#!/usr/bin/env python3
"""External-bar accounting: CPU per delivered frame and submit->presented latency.

Two subcommands, no desktop interaction:

  account.py sample --out before.json PID [PID...]
      Snapshot /proc/PID/stat (utime, stime, minflt, majflt, starttime, comm)
      for the compositor/server process set. Run once before and once after
      the client; the report refuses pairs whose PID identities differ.

  account.py report --arm NAME=JOURNAL:BEFORE:AFTER [--arm ...]
      Join each arm's journal with its CPU snapshots and print one table in
      the shape of the candidate-23 FINDINGS (CPU/frame, latency medians and
      p95/p99, intercompletion spacing, faults). Journal headers must agree
      in every field except `arm` and `presented_clock`, otherwise the join
      is refused: identical work per frame is checked, never assumed.

Everything printed is an observation of that run. Nothing here is a gate
verdict; the external reference thresholds are applied
by the reader against the rows, and the external rows price the pixel path
only (mutable buffers, cooperative release, no identity/checksum/evidence).
"""
import argparse
import json
import os
import statistics
import sys
import time
from thread_stats import read_threads, endpoint_delta

CLK_TCK = os.sysconf("SC_CLK_TCK")


def read_stat(pid):
    with open(f"/proc/{pid}/stat") as f:
        raw = f.read()
    rp = raw.rindex(")")
    comm = raw[raw.index("(") + 1 : rp]
    fields = raw[rp + 2 :].split()
    threads = read_threads(pid)
    with open(f'/proc/{pid}/stat') as f:
        again = f.read()
    if int(again[again.rindex(')') + 2:].split()[19]) != int(fields[19]):
        sys.exit('process identity changed during snapshot')
    # fields[0] is state (field 3); utime is field 14 -> index 11
    return dict(
        pid=int(pid),
        comm=comm,
        minflt=int(fields[7]),
        majflt=int(fields[9]),
        utime_ticks=int(fields[11]),
        stime_ticks=int(fields[12]),
        starttime=int(fields[19]),
        threads=threads,
    )


def cmd_sample(args):
    snap = dict(
        mono_ns=time.monotonic_ns(),
        clk_tck=CLK_TCK,
        pids=[read_stat(p) for p in args.pids],
    )
    snap['mono_end_ns'] = time.monotonic_ns()
    with open(args.out, "w") as f:
        json.dump(snap, f, indent=1)
        f.write("\n")
    print(f"sampled {len(snap['pids'])} pids -> {args.out}")


def parse_journal(path):
    header, rows, meta = None, [], {}
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            if not line:
                continue
            if line.startswith("# header "):
                if header is not None:
                    sys.exit(f"{path}: duplicate header")
                fields, separator, clock_note = line[len("# header "):].partition(" presented_clock=")
                if not separator:
                    sys.exit(f"{path}: missing presented_clock note")
                pairs = [kv.split("=", 1) for kv in fields.split()]
                if any(len(pair) != 2 for pair in pairs) or len({pair[0] for pair in pairs}) != len(pairs):
                    sys.exit(f"{path}: malformed or duplicate header field")
                header = dict(pairs, presented_clock=clock_note)
            elif line.startswith("# "):
                k, _, v = line[2:].partition(" ")
                meta[k] = v
            else:
                p = line.split("\t")
                if len(p) != 6:
                    sys.exit(f"{path}: expected six row fields")
                rows.append(
                    dict(
                        frame=int(p[0]),
                        fill_start=int(p[1]),
                        submit=int(p[2]),
                        presented=int(p[3]),
                        pace=int(p[4]),
                        source=p[5],
                    )
                )
    if header is None:
        sys.exit(f"{path}: no header line")
    return header, rows, meta


def ns_quantiles(values):
    if not values:
        return None
    s = sorted(values)

    def q(p):
        k = max(0, min(len(s) - 1, round(p * (len(s) - 1))))
        return s[k]

    return dict(p50=q(0.5), p95=q(0.95), p99=q(0.99), n=len(s))


def cpu_delta(before, after):
    if before["clk_tck"] != after["clk_tck"] or before["clk_tck"] <= 0:
        sys.exit("snapshot clock tick rates differ or are invalid")
    if after["mono_ns"] <= before["mono_ns"]:
        sys.exit("snapshot window is empty or reversed")
    if not before["pids"] or not after["pids"]:
        sys.exit("empty server PID set")
    b = {p["pid"]: p for p in before["pids"]}
    a = {p["pid"]: p for p in after["pids"]}
    if len(b) != len(before["pids"]) or len(a) != len(after["pids"]):
        sys.exit("duplicate PID in snapshot")
    if set(b) != set(a):
        sys.exit("pid sets differ between snapshots")
    out = dict(cpu_ns=0, minflt=0, majflt=0, pids=[],
               window_ns=after['mono_ns']-before['mono_ns'],
               endpoint_context_switches=dict(voluntary=0, involuntary=0))
    for pid in sorted(b):
        if b[pid]["starttime"] != a[pid]["starttime"] or b[pid]["comm"] != a[pid]["comm"]:
            sys.exit(f"pid {pid} identity changed between snapshots")
        ticks = (a[pid]["utime_ticks"] - b[pid]["utime_ticks"]) + (
            a[pid]["stime_ticks"] - b[pid]["stime_ticks"]
        )
        if any(a[pid][key] < b[pid][key] for key in ("utime_ticks", "stime_ticks", "minflt", "majflt")):
            sys.exit(f"pid {pid}: counter went backwards")
        out["cpu_ns"] += ticks * (1_000_000_000 // before["clk_tck"])
        out["minflt"] += a[pid]["minflt"] - b[pid]["minflt"]
        out["majflt"] += a[pid]["majflt"] - b[pid]["majflt"]
        out["pids"].append(f"{pid}:{b[pid]['comm']}")
        switches = endpoint_delta(b[pid].get('threads'), a[pid].get('threads'))
        if switches is None:
            out['endpoint_context_switches'] = None
        elif out['endpoint_context_switches'] is not None:
            for key, value in switches.items():
                out['endpoint_context_switches'][key] += value
    out["tick_ns"] = 1_000_000_000 // before["clk_tck"]
    # Each process's utime and stime are truncated tick counters, so each
    # delta is exact to within one tick per counter at each end: the true
    # CPU lies within +-2 ticks per process (attribution interval, not a
    # confidence interval), the same form as the candidate-23 counter bounds.
    slack = 2 * len(out["pids"]) * out["tick_ns"]
    out["cpu_ns_lower"] = max(0, out["cpu_ns"] - slack)
    out["cpu_ns_upper"] = out["cpu_ns"] + slack
    return out


def cmd_report(args):
    arms = []
    for spec in args.arm:
        name, _, rest = spec.partition("=")
        journal, before, after = rest.split(":")
        header, rows, meta = parse_journal(journal)
        with open(before) as f:
            b = json.load(f)
        with open(after) as f:
            a = json.load(f)
        arms.append(dict(name=name, header=header, rows=rows, meta=meta, cpu=cpu_delta(b, a)))

    ignore = {"arm", "presented_clock"}
    for arm in arms:
        required = {'lifecycle', 'workload', 'tiles', 'interval_ms', 'damage', 'idle_sec'}
        if not required <= arm['header'].keys():
            sys.exit(f"{arm['name']}: rev 5 workload header required")
        if arm['header']['workload'] not in {'fresh', 'bursty', 'retained', 'idle'}:
            sys.exit('unknown workload')
        h = arm['header']
        workload = h['workload']
        if (int(h['tiles']) != (8 if workload == 'retained' else 0)
                or (workload == 'bursty') != (int(h['interval_ms']) > 0)
                or (workload == 'idle') != (int(h['idle_sec']) > 0)
                or h['damage'] != ('96x24+1824+0' if workload == 'bursty' else 'full')
                or (workload != 'fresh' and int(h['bulk']) != 0)
                or (workload == 'idle' and (int(h['frames']) != 1 or int(h['warmup']) != 0))):
            sys.exit('workload header contradicts its declared shape')
    ref = {k: v for k, v in arms[0]["header"].items() if k not in ignore}
    for arm in arms[1:]:
        h = {k: v for k, v in arm["header"].items() if k not in ignore}
        if h != ref:
            diff = {k: (ref.get(k), h.get(k)) for k in set(ref) | set(h) if ref.get(k) != h.get(k)}
            sys.exit(f"journal header mismatch between {arms[0]['name']} and {arm['name']}: {diff}")

    results = []
    for arm in arms:
        rows = arm["rows"]
        frames_expected = int(arm["header"]["frames"])
        submitted = len(rows)
        if [r["frame"] for r in rows] != list(range(1, submitted + 1)):
            sys.exit(f"{arm['name']}: counted frame indices not contiguous from 1")
        if submitted != frames_expected:
            sys.exit(f"{arm['name']}: {submitted} counted rows, header frames={frames_expected}")
        for row in rows:
            if row["fill_start"] < 0 or row["submit"] < row["fill_start"] or row["presented"] < -1:
                sys.exit(f"{arm['name']}: invalid timestamp order")
            if row["presented"] >= 0 and row["presented"] < row["submit"]:
                sys.exit(f"{arm['name']}: presentation precedes submission")
        presented = [r for r in rows if r["presented"] >= 0]
        delivered = len(presented)
        lat = ns_quantiles([r["presented"] - r["submit"] for r in presented])
        # The comparable end-to-end latency across arms: the Sophia peer
        # prepares its upload before demanding a permit, while reference
        # clients fill after their pacing event, so submit and pace sit at
        # different points of the pipeline; fill start does not.
        fill_lat = ns_quantiles([r["presented"] - r["fill_start"] for r in presented])
        pace_lat = ns_quantiles([r['presented']-r['pace'] for r in presented if r['pace'] >= 0])
        pace_to_submit = ns_quantiles([r["submit"] - r["pace"] for r in presented if r["pace"] >= 0])
        spacing = ns_quantiles(
            [b2["presented"] - a2["presented"] for a2, b2 in zip(presented, presented[1:])]
        )
        sources = sorted({r["source"] for r in presented})
        if len(sources) > 1:
            sys.exit('mixed presentation sources cannot supply one latency distribution')
        ru = {}
        if "rusage" in arm["meta"]:
            ru = dict(kv.split("=") for kv in arm["meta"]["rusage"].split())
        client_cpu = (int(ru["utime_ns"]) + int(ru["stime_ns"])) if {"utime_ns", "stime_ns"} <= ru.keys() else None
        # Rows are counted frames only (warmup writes none). The client's
        # rusage covers warmup and setup too, so client CPU is divided by
        # counted + warmup submissions and is an upper-side observation; the
        # server CPU window is set by the caller's snapshots, divided by
        # delivered counted frames.
        warmup = int(arm["header"]["warmup"])
        server = arm["cpu"]
        workload = arm['header']['workload']
        selected = pace_lat if workload == 'retained' else fill_lat
        if workload == 'idle' or any(s in {'xpresent_recv', 'xpresent_late', 'shm_completion'} for s in sources):
            selected = None
        per_second = 1e9 / server['window_ns']
        results.append(
            dict(
                arm=arm["name"],
                header_arm=arm["header"]["arm"],
                frames_expected=frames_expected,
                submitted=submitted,
                delivered=delivered,
                server_cpu_ns=server["cpu_ns"],
                server_cpu_per_delivered_frame_us=(server["cpu_ns"] / delivered / 1000) if delivered else None,
                server_cpu_per_delivered_frame_us_bounds=(
                    server["cpu_ns_lower"] / delivered / 1000,
                    server["cpu_ns_upper"] / delivered / 1000,
                ) if delivered else None,
                server_tick_ns=server["tick_ns"],
                server_pids=server["pids"],
                server_minflt=server["minflt"],
                server_majflt=server["majflt"],
                server_cpu_per_second_us=server['cpu_ns'] * per_second / 1000,
                server_cpu_per_second_us_bounds=[server[k] * per_second / 1000 for k in ('cpu_ns_lower','cpu_ns_upper')],
                server_context_switches_endpoint_lower_bound_per_second={
                    k: v * per_second for k,v in server['endpoint_context_switches'].items()
                } if server['endpoint_context_switches'] is not None else None,
                context_switch_coverage='surviving_endpoint_threads_only_not_wakeups',
                client_window_cpu_per_second_us=None,
                client_cpu_ns=client_cpu,
                client_cpu_per_submitted_frame_us=(client_cpu / (submitted + warmup) / 1000)
                if submitted and client_cpu is not None
                else None,
                client_minflt=int(ru.get("minflt", -1)),
                client_majflt=int(ru.get("majflt", -1)),
                submit_to_presented_us={k: v if k == "n" else v / 1000 for k, v in lat.items()} if lat else None,
                fill_to_presented_us={k: v if k == "n" else v / 1000 for k, v in fill_lat.items()} if fill_lat else None,
                cross_arm_latency_kind='pace_to_presented' if workload == 'retained' else 'fill_to_presented',
                cross_arm_latency_us={k: v if k == 'n' else v / 1000 for k,v in selected.items()} if selected else None,
                pace_to_submit_us={k: v if k == "n" else v / 1000 for k, v in pace_to_submit.items()} if pace_to_submit else None,
                intercompletion_us={k: v if k == "n" else v / 1000 for k, v in spacing.items()} if spacing else None,
                presented_sources=sources,
                presented_clock=arm["header"].get("presented_clock"),
                clock_check=arm["meta"].get("presented_clock_check"),
                mode=arm["meta"].get("mode"),
            )
        )

    if args.json:
        with open(args.json, "w") as f:
            json.dump(dict(qualification="diagnostic_only_unaligned_cpu_windows_and_unproven_scanout", common_header=ref, arms=results), f, indent=1)
            f.write("\n")

    print("DIAGNOSTIC ONLY: CPU windows are not synchronized by this reporter; presentation source does not establish final composed scanout. No gate verdict.")
    print("common header:", " ".join(f"{k}={v}" for k, v in sorted(ref.items())))
    print()
    cols = [
        ("arm", lambda r: r["arm"]),
        ("delivered/submitted", lambda r: f"{r['delivered']}/{r['submitted']}"),
        ("server CPU/frame us [bounds]", lambda r: bounds(r)),
        ('server CPU us/s', lambda r: fmt(r['server_cpu_per_second_us'])),
        ("client CPU/frame us", lambda r: fmt(r["client_cpu_per_submitted_frame_us"])),
        ("workload latency p50/p95/p99 ms", lambda r: q3(r["cross_arm_latency_us"])),
        ("submit->presented p50/p95/p99 ms", lambda r: q3(r["submit_to_presented_us"])),
        ("pace->submit p50 ms", lambda r: fmt(r["pace_to_submit_us"]["p50"] / 1000) if r["pace_to_submit_us"] else "-"),
        ("spacing p50 ms", lambda r: fmt(r["intercompletion_us"]["p50"] / 1000) if r["intercompletion_us"] else "-"),
        ("server faults min/maj", lambda r: f"{r['server_minflt']}/{r['server_majflt']}"),
        ("presented source", lambda r: ",".join(r["presented_sources"])),
    ]
    if ref['workload'] == 'idle':
        cols = [
            ('arm', lambda r: r['arm']),
            ('server CPU us/s', lambda r: fmt(r['server_cpu_per_second_us'])),
            ('endpoint thread switches/s (lower bound)', lambda r: str(r['server_context_switches_endpoint_lower_bound_per_second'])),
            ('client window CPU', lambda r: 'unavailable (whole-run rusage only)'),
        ]
    widths = [max(len(c[0]), *(len(c[1](r)) for r in results)) for c in cols]
    print("  ".join(c[0].ljust(w) for c, w in zip(cols, widths)))
    for r in results:
        print("  ".join(c[1](r).ljust(w) for c, w in zip(cols, widths)))
    print()
    print("Server CPU sums /proc stat utime+stime over the sampled PID set (all threads);",
          f"counters tick at {results[0]['server_tick_ns'] / 1e6:.0f} ms, so bounds are +-2 ticks per process",
          "spread over delivered frames. Attribution intervals, not confidence intervals;",
          "a single run is an observation, not a bound on any other run.")
    print('Workload latency is pace->presented for retained, fill->presented for fresh/bursty; idle has no per-frame latency gate.')
    print('Context switches cover surviving endpoint threads only, not wakeups; short-lived threads need separate lifetime evidence.')
    print("External rows price the pixel path only: mutable buffers, cooperative release,",
          "no identity, checksum or evidence journal. They are not acceptance rows.")
    for r in results:
        if r["clock_check"]:
            print(f"{r['arm']}: presented clock check {r['clock_check']}")


def bounds(r):
    b = r["server_cpu_per_delivered_frame_us_bounds"]
    if not b:
        return "-"
    return f"{r['server_cpu_per_delivered_frame_us']:.1f} [{b[0]:.1f}-{b[1]:.1f}]"


def fmt(v):
    return "-" if v is None else f"{v:.3f}"


def q3(d):
    if not d:
        return "-"
    return f"{d['p50']/1000:.3f}/{d['p95']/1000:.3f}/{d['p99']/1000:.3f}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("sample")
    s.add_argument("--out", required=True)
    s.add_argument("pids", nargs="+")
    s.set_defaults(fn=cmd_sample)
    r = sub.add_parser("report")
    r.add_argument("--arm", action="append", required=True, help="NAME=JOURNAL:BEFORE.json:AFTER.json")
    r.add_argument("--json", help="also write the full report as JSON")
    r.set_defaults(fn=cmd_report)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
