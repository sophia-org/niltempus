# Desktop CPU per frame: XLibre, niri, Sophia

`bench.py` runs one workload on each stack and reports CPU per presented frame
through one `tools/cpu-window/cpuwin.py` plan. It reuses the external-bar
clients (`tools/external-bar`) and the Sophia shell-timing evidence as they are.

## Workload and window

- A 1920x24 strip at the top of DP-1 gets a new full panel every presented
  frame (external-bar `workload=fresh`, the Sophia peer's fill). Lifecycle
  `reuse` with 2 buffers is the closest control to Sophia's two-slot owned
  ring; it is not custody equivalence.
- Each run presents 119 warmup and 2001 counted frames. Journal rows 1 and
  2001 are generations 120 and 2120: the window is 2000 intervals, the same
  as Sophia's shell-timing runs. Row 1 is the start anchor and is not dropped.
- The recipe is pinned (`RECIPE`) and has no command-line variants. Every
  external row's journal header and `run.json` must match it field by field
  (size, buffers, fill, frames, warmup, bulk, lifecycle, workload, tiles,
  interval, damage, idle, clock), and the header must name the stack's client.
- Anchors are CLOCK_MONOTONIC presentation times:
  - Sophia: kernel page-flip completion (`native-samples.json`).
  - niri: `wp_presentation` feedback.
  - XLibre: Present COMPLETE_NOTIFY UST, only when the client checked
    `submit <= ust <= receipt`. The panel is an override-redirect strip, so
    Present completes in COPY mode at vblank; that is not a kernel scanout
    completion like Sophia's flip and is reported as such.

## CPU set

`cpuwin.py` counts each bound thread group: all its threads, never its
children.

| stack | bound processes |
| --- | --- |
| Sophia | session, peer |
| niri | niri, wl-panel |
| XLibre | Xorg, x-panel, plus `--wm` and `--compositor` when given |

CPU is these known thread groups only, not whole-desktop CPU. The run also
takes a boundary inventory of every process in the stack's sessions at the
start and end of the window, and a native report refuses a run with any
unbound process there. The inventory is not exhaustive: transient children
and setsid helpers that live only between the two snapshots are missed.

The pixel producers differ. The external clients fill shared memory on the
CPU, while the Sophia peer renders on the GPU and copies a readback into its
owned ring. Server and client rows are therefore reported separately, and a
combined figure describes these concrete paths, not compositor efficiency.

## Client lifetime

The client is started traced (`PTRACE_TRACEME`). The kernel stops it right
after `execve`, so it is bound under its own pid, start time and comm before
it submits anything. Both clients write rows at the end, so binding after the
first row would miss the window. It is resumed only once the sampler holds
samples. With `PTRACE_O_TRACEEXIT` it stops again at exit, still the same
readable process with its final counters, and is held 0.3 s so samples past
the final presentation exist. No client source changes.

Teardown never waits on the traced client through `Popen`: a signal to a
tracee is a trace stop, which `Popen` would record as its exit. It kills the
client's group and drains its stops with `waitpid` until the kernel reports
death, bounded at 3 s. The client is registered before its exec stop is
checked, and the sampler is a Stack process, so any error still stops both.
Anything teardown could not stop is recorded in `run.json` and refuses the
row. A clean `run.json` does not prove VT, PAM or process-group cleanup;
that is the packaging wrapper's job.

## Commands

All Python runs as `python3 -I -B`. Build the clients first:
`make -C tools/external-bar all build/headless-output`.

```sh
# Smoke: GPU- and device-free in bwrap; never CPU evidence.
python3 -I -B tools/desktop-cpu/bench.py run --stack xlibre --mode smoke --out RUN_X
python3 -I -B tools/desktop-cpu/bench.py run --stack niri --mode smoke --out RUN_W

# Native: as uid 1000 inside a fresh active seat0 PAM session on a text VT,
# stdin on that VT (the privileged VT/PAM handoff and restoration are the
# packaging wrapper's job, not this tool's).
python3 -I -B tools/desktop-cpu/bench.py run --stack niri --mode native --output DP-1 --out RUN_N
python3 -I -B tools/desktop-cpu/bench.py run --stack xlibre --mode native --output DP-1 --out RUN_L

# An existing Sophia shell-timing run, read only. It must be uninstrumented
# (native-samples instrumentation "none") to be native; every continuous
# package so far used capture-profile, which only imports as a diagnostic row:
python3 -I -B tools/desktop-cpu/bench.py import-sophia --out RUN_S SOPHIA_RUN_DIR
python3 -I -B tools/desktop-cpu/bench.py import-sophia --allow-instrumented --out RUN_S SOPHIA_RUN_DIR

python3 -I -B tools/desktop-cpu/bench.py report --out REPORT RUN_S RUN_N RUN_L
```

Native defaults:

- DP-1 at 1920x1080@119.880, every other output off. niri gets the rig's
  kernel connector names (`RIG_OTHER_OUTPUTS`). X names connectors itself
  (modesetting calls kernel HDMI-A-1 `HDMI-1`), and xrandr ignores an unknown
  name with exit 0, so native X turns off every output the server lists
  other than `--output`.
- niri: `/usr/bin/niri --config PRIVATE`, with no `--session` and no startup
  commands. It runs with `xwayland-satellite { off; }`, animations and the
  hotkey overlay off, and `RUST_LOG=warn` (niri's source default is debug).
- XLibre:
  - the prefix server `~/.local/opt/xlibre-56be9f4320ef/bin/Xorg`, never the
    setuid `/usr/bin/Xorg` wrapper;
  - run with `-keeptty -seat seat0`, a private cookie, and absolute private
    `-config` and `-configdir`;
  - kept in the VT session (its own process group, SIGTTOU ignored): with
    `-keeptty` Xorg never takes the VT as its controlling tty, and Linux
    refuses `VT_ACTIVATE` and `KDSETMODE` from any other session (EPERM).
    The boundary inventory finds it and its children by process group;
  - modesetting with glamor, `BusID PCI:3:0:0` and `kmsdev /dev/dri/card0`;
  - `TearFree` off explicitly, VRR off and page flips on;
  - the mode is chosen with xrandr from the server's own modes, with no
    invented modeline.

## What the report checks

- An external run finished cleanly: client exit 0 after its exit stop, the
  sampler stopped with 0, no teardown errors. A full journal alone is not
  enough.
- Every counted frame presented. Native rows require scanout-time sources:
  `wp_presentation`, or a validated `xpresent` UST.
- Presentation times strictly increase. In a native row every counted
  interval is one refresh of the observed mode: no gap over 1.5 nominal
  periods, and the median rate within 0.5% of it. A half-rate run is refused,
  never matched to 120 Hz. Smoke rows count missed gaps against their own
  median, for description only.
- A Sophia import is validated before it is written, and again at report
  time over the hashed files:
  - the run outcome: `result.json` captured_only and postflight passed;
  - the output: postflight saw DP-1 at 119880 mHz; the panel diagnostic shows
    the 1920x24 strip on DP-1 1920x1080 with VRR off, 120+2000 frames;
  - the request: owned_ring, 2120 frames, at the samples' instrumentation;
  - exact joins: generations 1..2120 once each, in order; completions
    strictly increasing; each latency is completion minus submission, on
    output 1 with one head; every offer presented;
  - identity: the cpu-binding host and peer are the processes cpu-window
    sampled.

  Any failure refuses the import.
- The output receipt shows the mode the server actually set, not the one
  requested:
  - niri: `niri msg --json outputs` must show DP-1 at 1920x1080 and 119.880,
    VRR off and every other output off;
  - XLibre: `xrandr --verbose` must show the current mode's vertical clock
    within 0.05% and no other active output.
- All native rows' measured refresh agrees within 0.5%.
- The cpuwin analyzer's identity, monotonicity and coverage checks pass.
- Differences are reported only when every requested row is native and
  accepted; one refused row withholds them all.

## Not verified

- No uninstrumented Sophia continuous run exists yet, so a native
  three-stack report needs a new Sophia run with instrumentation `none`.
  Per root, the shell-timing path supports this: request instrumentation
  `none`, peer `capture_profile` false, the same 2120 frames, 2 slots and
  120 Hz, with inherited profiling environment cleared. Its journal and
  exact-join logging remain, so `none` means no capture-profile timing, not
  ordinary production logging parity.
- Launch qualification is the packaging wrapper's, per root:
  - prove the server belongs to a fresh active seat0 VT session; the
    harness's `ttyname` check alone is insufficient;
  - drain residual process groups;
  - restore KD/KBD mode and termios, and close PAM.
- The X receipt shows the mode, not the observed TearFree, VRR or renderer
  state; the requested config is not proof of them. The niri GPU and renderer
  are not pinned or proved. Packaging qualifies both.
- No native run has been made. Rootless start of XLibre and niri inside the
  VT session, and GPU selection, are unproven on this rig.
- The Wayland smoke stands in a headless wlroots compositor (River 0.3.7
  with pixman) for niri. niri's headless backend is test-only, and its nested
  winit output is always transformed, which the client refuses. The niri
  config is still validated by niri itself.
- The installed niri 26.04 has not been proven to match `~/src/niri`.

Tests: `python3 -I -B -W error tools/desktop-cpu/test_bench.py`.
