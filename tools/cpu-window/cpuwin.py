#!/usr/bin/env python3
"""CPU-window sampler and offline bounds analyzer for named process sets.

  cpuwin.py sample --out RUN.jsonl --seconds S [--interval-ms 5]
                   --pid session=PID:STARTTIME --pid peer=PID:STARTTIME
      Periodic bracketed /proc/PID/stat snapshots. Each record carries
      CLOCK_MONOTONIC ns taken immediately before (t0) and after (t1) reading
      every named process, plus pid, starttime, comm and utime+stime ticks.
      Stops on SIGINT/SIGTERM, at --seconds, or once every bound process has
      exited; a process that exits is recorded as an error and sampling goes
      on. Exit 0 (samples kept; offline coverage decides validity), 2 binding
      mismatch at start (no file written), 3 identity changed.

  cpuwin.py analyze PLAN.json [--json OUT.json]
      For each run, bounds the named processes' CPU over [A, B], where A and B
      are exact kernel completion times (CLOCK_MONOTONIC ns, all heads: the
      latest head's completion is used). There is no interpolation and no
      point estimate:
        inner pair  i, j: first sample with t0 >= A, last with t1 <= B
                    (both reads lie inside [A, B]) -> lower bound
        outer pair  k, l: last sample with t1 <= A, first with t0 >= B
                    (the reads span [A, B]) -> upper bound
      utime and stime are each truncated to whole ticks, so a process's true
      utime+stime at a read lies in [r, r + 2 ticks); every delta therefore
      carries +-2 ticks per process:
        lower = sum max(0, inner - 2 ticks), upper = sum (outer + 2 ticks).
      A run is refused unless every named process keeps its pid, starttime
      and comm, reads cleanly and never goes backwards from k through l; both
      anchors are covered by samples within --max-gap; and record sequence
      numbers are contiguous.
      The per-frame denominator is end_frame - start_frame completion intervals.

Plan (JSON):
  {"baseline": "sealed",            # optional: other arms are compared with it
   "max_gap_ms": 20,                # optional, default 4 x sample interval
   "runs": [{"name": "r1", "arm": "sealed", "samples": "r1.jsonl",
             "start_frame": 120, "end_frame": 2120,
             "anchor_start_ns": 123 | [head0, head1, ...],
             "anchor_end_ns":   456 | [head0, head1, ...],
             "expect": {"session": {"pid": 11, "starttime": 999},
                        "peer": {"pid": 12}}}]}   # starttime optional

For idle workloads, set window_mode to "fixed", omit frame fields, and use
controller CLOCK_MONOTONIC anchors. Results are CPU ms/s, not CPU/frame.

The intervals are attribution intervals, not confidence intervals. One run is a
stage observation, never a bound on another run.
"""
import argparse
import json
import os
import resource
import signal
import statistics
import sys
import time

NS = 1_000_000_000
EXIT_BINDING = 2  # a --pid did not match its bound starttime at start
EXIT_IDENTITY = 3  # a bound fd read a different starttime (should be impossible)


class Refused(Exception):
    pass


def parse_stat(raw):
    """(comm, starttime, utime, stime) from one /proc/PID/stat line."""
    rp = raw.rindex(")")  # comm may contain ')' and spaces
    comm = raw[raw.index("(") + 1 : rp]
    f = raw[rp + 2 :].split()  # f[0] is field 3 (state)
    return comm, int(f[19]), int(f[11]), int(f[12])


def self_cpu_ns():
    ru = resource.getrusage(resource.RUSAGE_SELF)
    return round((ru.ru_utime + ru.ru_stime) * NS)


# ---------------------------------------------------------------- sampler


def cmd_sample(args):
    procs = {}
    for spec in args.pid:
        name, _, rest = spec.partition("=")
        pid, _, start = rest.partition(":")
        if (not name or name in procs or not pid.isdigit() or not start.isdigit()
                or int(pid) in (p["pid"] for p in procs.values())):
            print(f"bad --pid {spec!r}: want NAME=PID:STARTTIME, names and pids unique",
                  file=sys.stderr)
            sys.exit(EXIT_BINDING)
        # Open first, verify second: the fd is then bound to the verified
        # process, and after that process exits a reused PID reads as ESRCH
        # through it, never as the new process.
        try:
            fd = os.open(f"/proc/{pid}/stat", os.O_RDONLY | os.O_CLOEXEC)
            comm, got, _, _ = parse_stat(os.pread(fd, 4096, 0).decode())
        except OSError as e:
            print(f"{name}: pid {pid} unreadable: {e.strerror}", file=sys.stderr)
            sys.exit(EXIT_BINDING)
        if got != int(start):
            print(f"{name}: pid {pid} starttime {got} != bound {start}", file=sys.stderr)
            sys.exit(EXIT_BINDING)
        procs[name] = dict(pid=int(pid), fd=fd, comm=comm, starttime=got)
    clk_tck = os.sysconf("SC_CLK_TCK")
    interval = int(args.interval_ms * 1_000_000)
    with open("/proc/sys/kernel/random/boot_id") as f:
        boot_id = f.read().strip()

    stop = []
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda *_: stop.append("signal"))

    out = open(args.out, "x", buffering=1)  # never overwrite an earlier run

    def emit(rec):
        out.write(json.dumps(rec, separators=(",", ":")) + "\n")

    emit(dict(kind="header", version=1, clk_tck=clk_tck, interval_ns=interval,
              boot_id=boot_id, sampler_pid=os.getpid(),
              procs={n: dict(pid=p["pid"], starttime=p["starttime"], comm=p["comm"])
                     for n, p in procs.items()}))

    gone = set()

    def sample(seq):
        t0 = time.monotonic_ns()
        reads = {}
        for name, p in procs.items():
            if name in gone:
                reads[name] = dict(pid=p["pid"], error="exited")
                continue
            try:
                comm, start, ut, st = parse_stat(os.pread(p["fd"], 4096, 0).decode())
                reads[name] = dict(pid=p["pid"], starttime=start, comm=comm, ticks=ut + st)
                if start != p["starttime"]:
                    stop.append("identity_changed")
            except OSError as e:  # ESRCH: the bound process has been reaped
                reads[name] = dict(pid=p["pid"], error=e.strerror)
                gone.add(name)
        t1 = time.monotonic_ns()
        emit(dict(kind="sample", seq=seq, t0=t0, t1=t1, self_cpu_ns=self_cpu_ns(), procs=reads))

    begin = time.monotonic_ns()
    deadline = begin + int(args.seconds * NS)
    seq = slot = late = 0
    while True:
        sample(seq)
        seq += 1
        if len(gone) == len(procs):
            stop.append("all_exited")
        now = time.monotonic_ns()
        if stop or now >= deadline:
            break
        slot += 1
        due = begin + slot * interval
        if now >= due:  # overran: skip the missed slots and count them
            missed = (now - due) // interval + 1
            late += missed
            slot += missed
            due = begin + slot * interval
        time.sleep((min(due, deadline) - now) / NS)
    reason = stop[0] if stop else "seconds"
    emit(dict(kind="footer", samples=seq, late_slots=late, self_cpu_ns=self_cpu_ns(), stop=reason))
    out.close()
    return EXIT_IDENTITY if reason == "identity_changed" else 0


# ---------------------------------------------------------------- analyzer


STOPS = {"signal", "seconds", "all_exited", "identity_changed"}


def is_int(v, low=0):
    return isinstance(v, int) and not isinstance(v, bool) and v >= low


def check_header(h, where):
    if h.get("version") != 1 or isinstance(h.get("version"), bool):
        raise Refused(f"{where}: header version {h.get('version')!r}, want 1")
    if not is_int(h.get("clk_tck"), 1) or not is_int(h.get("interval_ns"), 1):
        raise Refused(f"{where}: clk_tck and interval_ns must be positive integers")
    if not isinstance(h.get("boot_id"), str):
        raise Refused(f"{where}: header has no boot_id")
    procs = h.get("procs")
    if not isinstance(procs, dict) or not procs:
        raise Refused(f"{where}: header names no processes")
    for name, p in procs.items():
        if not (isinstance(p, dict) and is_int(p.get("pid"), 1) and is_int(p.get("starttime"))
                and isinstance(p.get("comm"), str)):
            raise Refused(f"{where}: header process {name!r} needs pid, starttime and comm")
    pids = [p["pid"] for p in procs.values()]
    if len(set(pids)) != len(pids):  # two labels on one PID would count it twice
        raise Refused(f"{where}: header binds one pid under two names")
    return h


def check_sample(rec, names, where):
    if not all(is_int(rec.get(k)) for k in ("seq", "t0", "t1", "self_cpu_ns")):
        raise Refused(f"{where}: sample needs integer seq, t0, t1 and self_cpu_ns")
    procs = rec.get("procs")
    if not isinstance(procs, dict) or set(procs) != names:
        raise Refused(f"{where}: sample processes differ from the header's")
    for name, r in procs.items():
        ok = isinstance(r, dict) and is_int(r.get("pid"), 1) and (
            (set(r) == {"pid", "error"} and isinstance(r["error"], str))
            or (set(r) == {"pid", "starttime", "comm", "ticks"} and is_int(r["starttime"])
                and isinstance(r["comm"], str) and is_int(r["ticks"])))
        if not ok:
            raise Refused(f"{where}: malformed read for {name!r}")


def load(path):
    """Exactly one version-1 header first, then samples, then at most one footer."""
    header, samples, footer = None, [], None
    with open(path) as f:
        for n, line in enumerate(f, 1):
            where = f"{path}:{n}"
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                raise Refused(f"{where}: unparsable record (truncated write?)")
            kind = rec.get("kind") if isinstance(rec, dict) else None
            if footer is not None:
                raise Refused(f"{where}: record after the footer")
            if header is None:
                if kind != "header":
                    raise Refused(f"{where}: first record is {kind!r}, want the header")
                header = check_header(rec, where)
            elif kind == "sample":
                check_sample(rec, set(header["procs"]), where)
                samples.append(rec)
            elif kind == "footer":
                footer = rec
            elif kind == "header":
                raise Refused(f"{where}: second header")
            else:
                raise Refused(f"{where}: unknown record kind {kind!r}")
    if header is None:
        raise Refused(f"{path}: no header")
    if footer is not None:
        if footer.get("stop") not in STOPS or not all(
                is_int(footer.get(k)) for k in ("samples", "late_slots", "self_cpu_ns")):
            raise Refused(f"{path}: malformed footer")
        if footer["stop"] == "identity_changed":
            raise Refused(f"{path}: sampler stopped on identity_changed")
        if footer["samples"] != len(samples):
            raise Refused(f"{path}: footer counts {footer['samples']} samples, file has {len(samples)}")
    return header, samples, footer


def anchor(v, what):
    heads = v if isinstance(v, list) else [v]
    if not heads or not all(isinstance(h, int) for h in heads):
        raise Refused(f"{what}: want an int or a non-empty list of ints (ns)")
    return max(heads)


def analyze_run(run, max_gap_ms=None):
    header, s, footer = load(run["samples"])
    names = sorted(header["procs"])
    expect = run.get("expect", {})
    if sorted(expect) != names:
        raise Refused(f"process names {names} != plan expect {sorted(expect)}")
    for name, e in expect.items():
        h = header["procs"][name]
        if e["pid"] != h["pid"] or ("starttime" in e and e["starttime"] != h["starttime"]):
            raise Refused(f"{name}: sampled pid/starttime {h['pid']}/{h['starttime']} "
                          f"!= expected {e['pid']}/{e.get('starttime', '*')}")

    for n, rec in enumerate(s):
        if rec["seq"] != n:
            raise Refused(f"record sequence gap at index {n} (seq {rec['seq']})")
        if rec["t1"] < rec["t0"] or (n and rec["t0"] < s[n - 1]["t1"]):
            raise Refused(f"sample seq {n} out of order")

    a = anchor(run["anchor_start_ns"], "anchor_start_ns")
    b = anchor(run["anchor_end_ns"], "anchor_end_ns")
    if not a < b:
        raise Refused("anchor_start_ns must precede anchor_end_ns")
    mode = run.get("window_mode", "frames")
    if mode not in ("frames", "fixed"):
        raise Refused("window_mode must be frames or fixed")
    intervals = None
    if mode == "frames":
        intervals = run["end_frame"] - run["start_frame"]
        if intervals <= 0:
            raise Refused("end_frame must exceed start_frame")
    elif "start_frame" in run or "end_frame" in run:
        raise Refused("fixed windows must not specify frame denominators")

    def last(pred):
        idx = [n for n, r in enumerate(s) if pred(r)]
        return idx[-1] if idx else None

    def first(pred):
        idx = [n for n, r in enumerate(s) if pred(r)]
        return idx[0] if idx else None

    k = last(lambda r: r["t1"] <= a)
    i = first(lambda r: r["t0"] >= a)
    j = last(lambda r: r["t1"] <= b)
    l = first(lambda r: r["t0"] >= b)
    if k is None:
        raise Refused("no sample completed before the start anchor")
    if l is None:
        raise Refused("no sample began after the end anchor")
    if i is None or j is None or i > j:
        raise Refused("no pair of samples lies inside the window")

    gap = int((max_gap_ms if max_gap_ms is not None else 4 * header["interval_ns"] / 1e6) * 1e6)
    edges = dict(start_before=a - s[k]["t1"], start_after=s[i]["t0"] - a,
                 end_before=b - s[j]["t1"], end_after=s[l]["t0"] - b)
    for edge, ns in edges.items():
        if ns > gap:
            raise Refused(f"anchor coverage gap {edge}={ns / 1e6:.3f} ms > {gap / 1e6:.3f} ms")

    for n in range(k, l + 1):
        for name in names:
            r, h = s[n]["procs"].get(name), header["procs"][name]
            if r is None or "error" in r:
                raise Refused(f"{name}: unreadable at seq {n}: {r and r.get('error')}")
            if (r["pid"], r["starttime"], r["comm"]) != (h["pid"], h["starttime"], h["comm"]):
                raise Refused(f"{name}: identity changed at seq {n}")
            if n > k and r["ticks"] < s[n - 1]["procs"][name]["ticks"]:
                raise Refused(f"{name}: counter went backwards at seq {n}")
    outside = sum(1 for n, rec in enumerate(s) if not k <= n <= l
                  for name in names
                  if "error" in rec["procs"].get(name, {"error": 1})
                  or rec["procs"][name]["starttime"] != header["procs"][name]["starttime"])

    tick = NS // header["clk_tck"]
    per = {}
    for name in names:
        inner = s[j]["procs"][name]["ticks"] - s[i]["procs"][name]["ticks"]
        outer = s[l]["procs"][name]["ticks"] - s[k]["procs"][name]["ticks"]
        per[name] = dict(inner_ticks=inner, outer_ticks=outer,
                         lower_ns=max(0, inner - 2) * tick, upper_ns=(outer + 2) * tick)
    lower = sum(p["lower_ns"] for p in per.values())
    upper = sum(p["upper_ns"] for p in per.values())

    span = s[k : l + 1]
    steps = [y["t0"] - x["t0"] for x, y in zip(span, span[1:])]
    brackets = [r["t1"] - r["t0"] for r in span]
    window = b - a
    sampler_cpu = s[l]["self_cpu_ns"] - s[k]["self_cpu_ns"]
    result = dict(
        name=run["name"], arm=run["arm"], intervals=intervals,
        window_ns=window, tick_ns=tick, boot_id=header["boot_id"],
        procs={n: dict(pid=header["procs"][n]["pid"], starttime=header["procs"][n]["starttime"],
                       comm=header["procs"][n]["comm"], **per[n]) for n in names},
        lower_ns=lower, upper_ns=upper,
        edges_ns=edges, samples_used=(k, i, j, l),
        cadence_ns=dict(nominal=header["interval_ns"], p50=statistics.median(steps), max=max(steps)),
        bracket_ns=dict(p50=statistics.median(brackets), max=max(brackets)),
        sampler_cpu_ns=sampler_cpu, sampler_cpu_per_s_ms=sampler_cpu / 1e6 / ((s[l]["t0"] - s[k]["t1"]) / NS),
        footer=footer["stop"] if footer else "missing",
        late_slots=footer["late_slots"] if footer else None,
        unclean_reads_outside_window=outside,
    )
    if mode == "frames":
        result["per_frame_us"] = (lower / intervals / 1000, upper / intervals / 1000)
        result["per_proc_per_frame_us"] = {
            n: (p["lower_ns"] / intervals / 1000, p["upper_ns"] / intervals / 1000)
            for n, p in per.items()}
    else:
        # CPU milliseconds per elapsed second; 1000 ms/s is one whole core.
        result["cpu_ms_per_s"] = (lower / window * 1000, upper / window * 1000)
        result["per_proc_cpu_ms_per_s"] = {
            n: (p["lower_ns"] / window * 1000, p["upper_ns"] / window * 1000)
            for n, p in per.items()}
    return result


def mean_interval(rows, metric="per_frame_us"):
    return (statistics.mean(r[metric][0] for r in rows),
            statistics.mean(r[metric][1] for r in rows))


def cmd_analyze(args):
    with open(args.plan) as f:
        plan = json.load(f)
    modes = {run.get("window_mode", "frames") for run in plan["runs"]}
    if len(modes) != 1 or not modes <= {"frames", "fixed"}:
        print("REFUSED: compare only one window_mode per plan")
        return 1
    fixed = modes == {"fixed"}
    metric = "cpu_ms_per_s" if fixed else "per_frame_us"
    per_proc_metric = "per_proc_cpu_ms_per_s" if fixed else "per_proc_per_frame_us"
    units = "ms/s" if fixed else "us/frame"
    results, refused = [], []
    for run in plan["runs"]:
        try:
            results.append(analyze_run(run, plan.get("max_gap_ms")))
        except Refused as e:
            refused.append(dict(name=run["name"], arm=run["arm"], reason=str(e)))
    arms = {}
    for r in results:
        arms.setdefault(r["arm"], []).append(r)
    summary = {arm: mean_interval(rows, metric) for arm, rows in arms.items()}
    diffs = {}
    base = plan.get("baseline")
    if base in summary:
        bl, bu = summary[base]
        for arm, (lo, hi) in summary.items():
            if arm != base:
                diffs[f"{arm}-{base}"] = (lo - bu, hi - bl)

    heading = "CPU ms/s" if fixed else "CPU/frame us"
    print(f"run      arm      order  {heading} [lower, upper]  per process [lower, upper]")
    for n, run in enumerate(plan["runs"]):
        r = next((x for x in results if x["name"] == run["name"]), None)
        if r is None:
            why = next(x["reason"] for x in refused if x["name"] == run["name"])
            print(f"{run['name']:<8} {run['arm']:<8} {n:<6} REFUSED: {why}")
            continue
        lo, hi = r[metric]
        procs = "  ".join(f"{p}=[{a:.1f}, {b:.1f}]" for p, (a, b) in r[per_proc_metric].items())
        print(f"{r['name']:<8} {r['arm']:<8} {n:<6} [{lo:.1f}, {hi:.1f}]{'':<14} {procs}")
    print()
    for r in results:
        e = r["edges_ns"]
        scope = "fixed window" if fixed else f"{r['intervals']} intervals"
        print(f"{r['name']}: {scope} over {r['window_ns'] / 1e9:.3f} s; "
              f"cadence p50/max {r['cadence_ns']['p50'] / 1e6:.2f}/{r['cadence_ns']['max'] / 1e6:.2f} ms "
              f"(nominal {r['cadence_ns']['nominal'] / 1e6:.2f}); bracket p50/max "
              f"{r['bracket_ns']['p50'] / 1e3:.0f}/{r['bracket_ns']['max'] / 1e3:.0f} us; "
              f"anchor edges {e['start_before'] / 1e6:.2f}+{e['start_after'] / 1e6:.2f} / "
              f"{e['end_before'] / 1e6:.2f}+{e['end_after'] / 1e6:.2f} ms; "
              f"sampler CPU {r['sampler_cpu_per_s_ms']:.2f} ms/s (not in the sampled set); "
              f"footer {r['footer']}; late slots {r['late_slots']}; unclean reads outside window {r['unclean_reads_outside_window']}")
    print()
    for arm, (lo, hi) in summary.items():
        print(f"arm {arm}: mean over {len(arms[arm])} runs [{lo:.1f}, {hi:.1f}] {units}")
    for k, (lo, hi) in diffs.items():
        print(f"{k}: [{lo:.1f}, {hi:.1f}] {units} (interval of the difference of run means)")
    tick_ms = results[0]["tick_ns"] / 1e6 if results else 0
    print(f"Ticks are {tick_ms:.0f} ms; each process adds 4 ticks of width to its interval. "
          "Attribution intervals, not confidence intervals; stage observations, not bounds on other runs.")
    if args.json:
        with open(args.json, "w") as f:
            json.dump(dict(runs=results, refused=refused, arms=summary, differences=diffs), f, indent=1)
            f.write("\n")
    return 1 if refused else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("sample")
    s.add_argument("--out", required=True)
    s.add_argument("--interval-ms", type=float, default=5.0)
    s.add_argument("--seconds", type=float, required=True, help="hard bound: runtime plus margin")
    s.add_argument("--pid", action="append", required=True, help="NAME=PID:STARTTIME, repeatable")
    s.set_defaults(fn=cmd_sample)
    a = sub.add_parser("analyze")
    a.add_argument("plan")
    a.add_argument("--json")
    a.set_defaults(fn=cmd_analyze)
    args = ap.parse_args()
    sys.exit(args.fn(args) or 0)


if __name__ == "__main__":
    main()
