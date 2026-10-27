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

The run scans every process in the stack's sessions at the start and end of
the window. A native report refuses a run with any unbound process; activated
services or helpers must be bound or disclosed, never silently omitted.

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

# An existing Sophia shell-timing run, read only.
python3 -I -B tools/desktop-cpu/bench.py import-sophia --out RUN_S SOPHIA_RUN_DIR

python3 -I -B tools/desktop-cpu/bench.py report --out REPORT RUN_S RUN_N RUN_L
```

Native defaults:

- DP-1 at 1920x1080@119.880. Every other connector on the rig is off; see
  `RIG_OTHER_OUTPUTS`.
- niri: `/usr/bin/niri --config PRIVATE`, with no `--session` and no startup
  commands. It runs with `xwayland-satellite { off; }`, animations and the
  hotkey overlay off, and `RUST_LOG=warn` (niri's source default is debug).
- XLibre:
  - the prefix server `~/.local/opt/xlibre-56be9f4320ef/bin/Xorg`, never the
    setuid `/usr/bin/Xorg` wrapper;
  - run with `-keeptty -seat seat0`, a private cookie, and absolute private
    `-config` and `-configdir`;
  - modesetting with glamor, `BusID PCI:3:0:0` and `kmsdev /dev/dri/card0`;
  - `TearFree` off explicitly, VRR off and page flips on;
  - the mode is chosen with xrandr from the server's own modes, with no
    invented modeline.

## What the report checks

- Every counted frame presented. Native rows require scanout-time sources:
  `wp_presentation`, or a validated `xpresent` UST.
- The output receipt shows the mode the server actually set, not the one
  requested:
  - niri: `niri msg --json outputs` must show DP-1 at 1920x1080 and 119.880,
    VRR off and every other output off;
  - XLibre: `xrandr --verbose` must show the current mode's vertical clock
    within 0.05% and no other active output.
- All native rows' measured refresh agrees within 0.5%.
- The cpuwin analyzer's identity, monotonicity and coverage checks pass.
- Differences are reported only when every row is native.

## Not verified

- No native run has been made. Rootless start of XLibre and niri inside the
  VT session, and GPU selection, are unproven on this rig.
- The Wayland smoke stands in a headless wlroots compositor (River 0.3.7
  with pixman) for niri. niri's headless backend is test-only, and its nested
  winit output is always transformed, which the client refuses. The niri
  config is still validated by niri itself.
- The installed niri 26.04 has not been proven to match `~/src/niri`.

Tests: `python3 -I -B -W error tools/desktop-cpu/test_bench.py`.
