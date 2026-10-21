# CPU-window sampler: CLI, schema, recipe (D4 diagnostic)

Files (job tmp source only):

- `cpuwin.py`: the sampler and analyzer, stdlib Python 3.
- `test_cpuwin.py`: 41 synthetic tests. Run `python3 -I -W error test_cpuwin.py`.

## Sampler CLI

```
python3 -I cpuwin.py sample --out RUN.jsonl --seconds S [--interval-ms 5] \
    --pid session=PID:STARTTIME --pid peer=PID:STARTTIME
```

- `--pid` takes `NAME=PID:STARTTIME` and may be repeated. STARTTIME is field 22 of `/proc/PID/stat`, in clock ticks since boot.
  - The sampler opens `/proc/PID/stat` first and then checks the start time through that fd. The fd is therefore bound to the verified process.
  - After that process is reaped, the fd reads ESRCH. It never reads a reused PID.
- `--seconds` is required: the hard bound, set to the runtime plus a margin.
- `--interval-ms` defaults to 5. Slots run on an absolute schedule; an overrun skips slots and counts them in `late_slots`.
- `--out` is opened with O_EXCL, so the sampler never overwrites an earlier run. It writes one line per record and flushes each line.

The sampler stops on whichever comes first:

- SIGTERM or SIGINT;
- `--seconds`;
- every bound process having exited.

It takes one sample at stop and then writes the footer.

| Exit | Meaning |
| --- | --- |
| 0 | Samples kept, including after a normal process exit. The offline analysis decides validity. |
| 2 | Binding mismatch or unreadable PID at start. No file is written. |
| 3 | Identity changed: a bound fd returned a different starttime. This should be impossible; the file is kept and the footer has `stop:"identity_changed"`. |
| 1 | Python error. |

A process that exits mid-run is recorded as `{"pid":N,"error":"No such process"}` (later reads as `"exited"`). The sampler keeps sampling the others.

A SIGKILLed sampler loses only its footer. The analyzer accepts a run with no footer and reports `late_slots` as null.

## Schema (JSONL, version 1)

```
{"kind":"header","version":1,"clk_tck":100,"interval_ns":5000000,"boot_id":"…",
 "sampler_pid":N,"procs":{"session":{"pid":N,"starttime":T,"comm":"…"},"peer":{…}}}
{"kind":"sample","seq":0,"t0":MONO_NS,"t1":MONO_NS,"self_cpu_ns":N,
 "procs":{"session":{"pid":N,"starttime":T,"comm":"…","ticks":UTIME+STIME},
          "peer":{"pid":N,"error":"No such process"}}}
…
{"kind":"footer","samples":N,"late_slots":N,"self_cpu_ns":N,
 "stop":"signal|seconds|all_exited|identity_changed"}
```

- `t0` and `t1` are CLOCK_MONOTONIC ns (`time.monotonic_ns`), taken immediately before and after reading all procs.
- `ticks` is utime+stime of the whole thread group: live threads plus exited threads, excluding children.
- `self_cpu_ns` is the sampler's own rusage, used for the overhead label.

## Analyzer

```
python3 -I cpuwin.py analyze PLAN.json [--json OUT.json]   # exit 1 if any run is refused
```

Plan:

```json
{"baseline": "sealed", "max_gap_ms": 20,
 "runs": [{"name": "A1", "arm": "sealed", "samples": "A1.jsonl",
           "start_frame": 120, "end_frame": 2120,
           "anchor_start_ns": [head0_ns, head1_ns], "anchor_end_ns": [head0_ns, head1_ns],
           "expect": {"session": {"pid": 11, "starttime": 100}, "peer": {"pid": 12, "starttime": 200}}}]}
```

Anchors are kernel completion times in CLOCK_MONOTONIC ns. They must be in the same clock domain as the sampler. DRM page-flip event timestamps are CLOCK_MONOTONIC when DRM_CAP_TIMESTAMP_MONOTONIC is 1; the observer should confirm this. For a list of per-head times, the latest head is the all-head completion.

The denominator is `end_frame - start_frame`, which is 2000 completion intervals.

### Bounds

Take k as the last sample with t1 ≤ A, i as the first with t0 ≥ A, j as the last with t1 ≤ B, and l as the first with t0 ≥ B. Samples whose bracket straddles an anchor are never used.

For each process:

- lower = max(0, ticks[j] − ticks[i] − 2) ticks
- upper = ticks[l] − ticks[k] + 2 ticks

The ±2 holds because utime and stime are each truncated, so the true value at a read is in [r, r+2) ticks.

The totals are the sums over processes. The analyzer does no interpolation and gives no point estimate.

### Record validation

These checks run before any bounds:

- The header must come first and appear exactly once, with integer `version` 1 and positive integer `clk_tck` and `interval_ns`.
- The header needs a `boot_id` and a non-empty process set, where each process has an integer pid > 0, a starttime and a comm.
- PIDs must be unique across labels, since two labels would count one PID twice. The sampler also refuses this at start, with exit 2.
- Only `sample` records follow. Each must have integer seq, t0, t1 and self_cpu_ns, exactly the header's labels, and each read must be either {pid, error} or {pid, starttime, comm, ticks}.
- At most one footer may appear, and it must be last. Its stop must be a known value and its integers well formed. `samples` must equal the number of sample records. `stop:"identity_changed"` refuses the run.
- A missing footer is accepted and reported as `footer missing`, with `late_slots` null.
- Unknown kinds, a second header, any record after the footer and blank or unparsable lines are refused.

### Refusals

A run is refused when any of the following holds:

- the record sequence has a gap, or brackets overlap or run out of order;
- a record is truncated;
- the plan's expected names, PIDs or start times differ from the header;
- there is no k or no l (coverage of both anchors);
- there is no inner pair with i ≤ j;
- any anchor edge gap (A−t1[k], t0[i]−A, B−t1[j], t0[l]−B) exceeds `max_gap_ms`, which defaults to 4 × the interval;
- any process is unreadable, or changes pid, starttime or comm, anywhere from k to l;
- a counter goes backwards from k to l.

Reads outside k..l, such as a normal exit after the final presentation, are only counted (`unclean_reads_outside_window`).

### Labels per run

- cadence p50/max against nominal;
- bracket (proc read time) p50/max;
- the four anchor edge gaps;
- sampler CPU in ms/s over k..l. The sampler is not in the sampled set; this is the overhead it adds elsewhere on the machine.
- `late_slots`;
- `boot_id`.

### Across runs

Per arm, the analyzer reports the mean of the per-frame lower bounds and the mean of the upper bounds: an interval for the mean of the true values. Each non-baseline arm also gets `arm − baseline`: [lo_arm − hi_base, hi_arm − lo_base]. These are attribution intervals, not confidence intervals; single runs are stage observations, not bounds.

## Recipe: first D4 diagnostic

1. Use the same binary for both arms (sealed and ring), with warmup 120 and counted frames 2000. The order is ABBA: sealed, ring, ring, sealed.
2. Per run, the parent binds the Session and peer PIDs and start ticks (CampaignObserver plus EnvironmentMonitor), then starts:
   `cpuwin.py sample --out <run>.jsonl --seconds <runtime+margin> --interval-ms 5 --pid session=P:T --pid peer=P:T`.
   The sampler must start before frame 120 completes; coverage enforces this.
3. Record the all-head kernel completion times for frames 120 and 2120 from the observer.
4. Keep the Session and peer alive for at least one interval plus the bracket after frame 2120 completes. Otherwise there is no `l` sample and the run is refused. Either keep the peer running a few frames past 2120 or hold it before exit.
5. On runner cleanup, send SIGTERM to the sampler and check its exit status. Exit 0 means the file is kept; 2 or 3 is a hard failure.
6. Write the plan with `baseline: "sealed"`, then run `cpuwin.py analyze plan.json --json out.json`. Quote the per-run and arm intervals with their labels.

### Expected resolution

The tick slack is fixed at 4 ticks (40 ms) per process. That is 20 µs per frame per process over 2000 intervals, so 40 µs per frame for Session and peer combined, before edge effects. Edge gaps add CPU over at most about 2 × (interval + bracket) per anchor.

A ring−sealed difference smaller than the combined width will not separate.

Option: summing per-thread `/proc/PID/task/*/schedstat` gives nanoseconds, but it loses exited threads. It is not built.

### Not handled

- **Journal join.** The ring journal is schema 5, which is not external rev 5, and account.py's journal join does not apply. Anchors come from the observer, not from journal parsing.
- **Children.** cutime is ignored.
- **PIDs outside the named set.** Engine or renderer processes, if separate, need their own `--pid`.
