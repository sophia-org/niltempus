# Quickshell: closure and coverage map

Sophia rule 13 moves the Quickshell probes here. Two trace probes and the panel
runner/verifier move whole. Sophia gains no seam, CLI or public type for them.
This follows root's review at
`development-evidence/g4-s6-closure/quickshell-public-api-review-6fda6f3b.md`.
Source revision: the pin, `de776c68afdf9a133818f86917893c3362dc9fb7`. The
retained regression is at `6fda6f3b`.

## What moved, and where

| Sophia at the pin | Here |
| --- | --- |
| `sophia x-authority-quickshell-smoke` (`commands/x_authority.rs:76-79`, `basic_smokes.rs:522-590`) | `quickshell-probe --renderer=gpu` (`crates/quickshell-probe`) |
| `sophia x-authority-quickshell-software-smoke` (`x_authority.rs:80-88`, `basic_smokes.rs:592-652`) | `quickshell-probe --renderer=software` |
| The Quickshell entries in the shared stage/error policy (`external_probe.rs:347`, `:548`) and the observer's null-window pre-filter (`:98-104`) | `quickshell_probe::{Renderer::required_stages, allowed}` |
| The GPU provider and allocator (`render_device_provider.rs`, `basic_smokes.rs:653-712`) | `quickshell_probe::runner::gpu` |
| `crates/xtask/src/panel.rs` (`cargo xtask panel`) | `crates/xtask/src/panel.rs` (`cargo xtask panel`) |
| `sophia_conformance::panel::verify` (`cargo xtask conformance verify panel`) | `xtask::panel::verify` (`cargo xtask panel verify LOG`) |
| `crates/sophia-conformance/tests/panel.rs` (3 tests) | `crates/xtask/tests/panel.rs` (the same 3) |
| `tools/fixtures/quickshell_sophia/{shell.qml,core.kdl,desktop.kdl}` | The same paths, byte for byte (`tools/fixtures/PROVENANCE`) |
| `docs/quickshell-x11-panel.md` | This page ("Running") |

The shared external-probe harness, its other X11 clients, the render-device
helpers, the generic Qt popup tests and the Present tests stay in Sophia.

The probe uses only public API:
- `sophia-x-authority`: `XServerFrontendConfig::new`,
  `with_render_device_provider` and `with_pixmap_allocator`;
  `XServerFrontend::bind_exclusive`;
  `try_serve_next_concurrently_routed_traced`; and `poll_client_workers`,
  `shutdown_all_client_workers` and `wait_for_clients`.
- The trace: `X11CoreTraceObserver`, `X11DispatchObservation`,
  `X11ObservedRequestStage::evidence_name` and `XClientOutput::Error`.
- For the GPU variant, `sophia-backend-live` (feature `gbm-probe`):
  `query_dma_buf_import_formats` and `allocate_shared_buffer`.

The probe is a separate package, so the GBM/EGL stack never enters the
packaged xtask. The panel verifier uses the public
`sophia_conformance::record::after_marker`. The helper it called before only
forwarded to it.

## What the old probes actually asserted

This is read from the code, not the prose.

**GPU (`quickshell`).**
- Setup: a measured render provider and a pixmap allocator on
  `first_openable_render_node()`. Without Sophia's `native-session` feature,
  the import list was empty and there was no allocator.
- The client ran with `--path CONFIG` and a 20-second window. The loop stopped
  early on the first authority transaction, on client exit, or at the deadline.
- Required stage: `RENDER:QueryPictFormats` only. It was checked by substring on
  the joined stage list: `details.contains(required)`.
- Errors: only the first error was kept. It passed if it matched
  `BadWindow:major=138:minor=3:` (any resource).
- The client had to exit 0, unless it was stopped at the deadline.
- Not required: authority transactions, runtime commits and pixel proof. The
  runtime and pixel counts were reported but did not gate the verdict.

**Software (`quickshell_software`).**
- Setup: `QT_QUICK_BACKEND=software`, with no provider and no allocator.
- No required stage.
- Errors: first error only, with no tolerance.
- The same exit and deadline rule applied, and the same things were not
  required.

**In both variants:**
- The trace observer dropped every `BadWindow` with resource 0, minor 0 and
  major 3 or 14 (GetWindowAttributes/GetGeometry on window 0) before the
  verdict saw it.
- Any dispatch failure counted as an error, reported as `parse_error`.
- Trace delivery used `try_send` on a 4096-slot channel, so a full channel
  dropped evidence silently.
- Output was collected with `wait_with_output` after the group was killed.
  That call does not bound a pipe that a descendant still holds.
- A client that never connected was not refused. The server thread was joined
  only when requests were greater than 0. So a software client that exited 0
  without opening the display, or that was stopped at the deadline, passed.
  The GPU variant failed only through its missing stage.

## What the prose claimed beyond that

These claims have no executable assertion:
- **"Reaches GLX and DRI3 against Sophia" and "reaches DRI3 and Present"** (the
  `basic_smokes.rs` docs). No GLX, DRI3 or Present stage was required.
- **"No refusal in it is the server's"** (`basic_smokes.rs:572`). Only the first
  error was examined. An allowed `BadWindow` first hid everything after it.
- **Row 102 of `x11-compatibility-matrix.md`.** It calls the trace `proven` for
  a "326-request trace across 33 opcodes including DRI3 and Present", reaching
  "GLX context creation and DRI3 `PixmapFromBuffers`". The counts, the opcodes
  and those stages were observations, not assertions.
- **Rows 81 and 101 of the same matrix.** They cite these commands as evidence
  for SHAPE (opcode 145 reached) and for the MIT-SHM software path. Neither
  stage was asserted by the probe.

## What the external probe asserts

`crates/quickshell-probe` keeps every old acceptance condition and adds the
review's corrections:

| Condition | Old | Now |
| --- | --- | --- |
| Required stage | GPU: `RENDER:QueryPictFormats` by substring | GPU: the same token, compared **exactly** against the set of `evidence_name` tokens; software: none, as before |
| X errors | first only | **every** error is checked against explicit allowed shapes; each disallowed one is named in the refusal |
| Allowed shapes | GPU: `BadWindow` 138/3; all: null-window `BadWindow` 3/14 dropped | the same two shapes, no wider; 138/3 stays GPU-only |
| Trace overflow | dropped silently | channel of 65,536 requests; any full send marks the run and **refuses** it |
| Dispatch failure | refused (`parse_error`) | refused, except `ClientDeparted` for a request in flight after the probe began stopping the client |
| Never connected | passed | **refused** (`client_never_connected`) |
| Client exit | nonzero refused; deadline stop allowed | the same; a signal death is refused too |
| Early stop | first transaction | the same |
| Accept | one-shot traced server; the idle timeout did not bound the initial accept | public nonblocking frontend on its own thread, an overall deadline, explicit worker shutdown, and a 10-second teardown bound |
| Custody | kill the group, then `wait_with_output` | xtask's `ProcessGroup` (WNOWAIT status, TERM, 2 s grace, KILL, reap last); stdout/stderr go to files capped at 4 MiB and polled while running |
| Render node | first openable node, discovered | GPU: an explicit `--render-node=/dev/dri/renderDN`, never discovered; software: none, and the CLI refuses unless `/dev/dri` is absent or empty (device-hidden) |
| Socket | `/tmp/.X11-unix/X{7900 or 7950 + pid%1000}` | the first free `/tmp/.X11-unix/XN`, N from 7900 to 8899, bound exclusively; removed on return |

The `ClientDeparted` exception is new. The old probe killed the client and then
drained, so a request in flight at the kill could fail it. Here the observer
records whether the probe had begun stopping the client. Only that one failure
kind, after that point, is not the server's. Any X error, whenever it arrives,
is still checked.

Evidence goes into the `--out` directory, which is new and mode 0700. It
contains `identity.txt` (renderer, binary and QML paths with SHA-256, render
node), `client.stdout`, `client.stderr` and `verdict.txt`.

Tests, offline, with no X client:
- `crates/quickshell-probe/tests/evaluate.rs` covers the verdict:
  - an allowed error followed by a disallowed one (the two-error negative);
  - several disallowed errors, all named;
  - the exact shapes, and 138/3 as GPU-only;
  - exact stage tokens, including near-miss tokens;
  - overflow, dispatch failure, server error and log-cap refusals;
  - nonzero exit, signal death and never-connected refusals;
  - acceptance at the deadline, after a transaction and on exit 0.
- `crates/quickshell-probe/tests/runner.rs` covers the real frontend and
  custody against stub clients:
  - a client that never connects is stopped at the deadline, refused, its
    socket removed and its evidence written;
  - a client that exits 0 without connecting is refused;
  - GPU without a node, a non-render node, software with a node and relative
    paths are refused before anything runs.

## The teardown assertion

Only the GPU probe tolerated the `BadWindow` Present 138/3. Behind that
tolerance was a server claim: that BadWindow is the correct answer when a
client unselects Present events on a destroyed window, and that the
connection keeps serving afterwards. That claim is generic protocol coverage.
Sophia retains it in
`present_selection_after_destroy_reports_bad_window_and_keeps_serving`
(`crates/sophia-x-authority/tests/present_msc_ordering.rs:344`, signed
`6fda6f3b`). The test covers:
- selection, destroy, two unrelated requests and zero-mask unselection;
- the exact BadWindow identity;
- continued use of the connection;
- both byte orders.

The existing unknown-window dispatch test stays as well. The external probe
keeps only the client-specific tolerance: it allows that shape without
re-proving it.

## Running

Every run needs an execution grant. The software probe runs device-hidden
(`bwrap --dev /dev`, `--unshare-net`, a private `/tmp`) with DISPLAY and
WAYLAND_DISPLAY unset. `--unshare-net` also privatises the abstract X socket
namespace, which Xlib tries first.

```sh
quickshell-probe --renderer=software --quickshell=/ABS/quickshell \
  --config=/ABS/tools/fixtures/quickshell_sophia/shell.qml --out=/ABS/NEW
```

The GPU variant exposes exactly one named render node and needs its own grant:

```sh
quickshell-probe --renderer=gpu --render-node=/dev/dri/renderD128 \
  --quickshell=/ABS/quickshell --config=/ABS/.../shell.qml --out=/ABS/NEW
```

### The panel probe

The isolated panel probe runs a prepared Sophia binary. It never uses a source
tree's `target/debug`:

```sh
cargo xtask panel --probe --renderer=software --quickshell=/ABS/quickshell \
  --wm=/ABS/hagia --sophia=/ABS/prepared/sophia [--output=/ABS/NEW]
cargo xtask panel verify /ABS/NEW/session.log
```

- The run uses `sophia session run --session-mode=normal --no-input`, a
  deterministic 1280x720 output, the given Hagia and an xterm witness.
  `/usr/bin/xterm`, `/usr/bin/firefox` and `/bin/sh` must exist. Firefox is
  registered but never started.
- The probe-mode session is spawned in its own process group under
  `wait_logged`, with a 60 s limit and a 4 MiB log cap. The session's own
  deadline is 15 s.
- To pass, the evidence must show all of the following:
  - panel (1280x32) and popup (240x112) surfaces with visual detail;
  - at least three panel and two popup buffer generations;
  - the popup sampled before the last panel sample;
  - a terminal below the reservation;
  - the four work-area transitions;
  - a clean protocol tally and a clean cleanup;
  - software rendering;
  - the isolated session record.

Live mode (`cargo xtask panel` without `--probe`) is operator-only. It attaches
the fixture to the current session's DISPLAY, and no gate here runs it: gates
unset DISPLAY, which live mode refuses. The operator checklist from
`docs/quickshell-x11-panel.md` applies unchanged.

## Root deletion boundary

This applies after the external gate passes and this map is accepted. It
follows root's review.

**Remove:**
- the two named arms in `commands/x_authority.rs` and their two help lines
  (`help.rs:57-58`);
- `run_x_authority_quickshell_smoke` and
  `run_x_authority_quickshell_software_smoke` in `basic_smokes.rs`;
- only their entries in the stage table (`external_probe.rs:347`) and in
  `probe_tolerates_client_error` (`:548`, which then has no caller);
- xtask `panel` wiring and `crates/xtask/src/panel.rs`;
- `conformance verify panel`;
- `sophia_conformance::panel` and `tests/panel.rs`;
- `tools/fixtures/quickshell_sophia/`;
- `docs/quickshell-x11-panel.md` and its `docs/README.md` entry.

**Keep:**
- the null-window pre-filter in the shared observer, which other clients
  still rely on;
- shared render-device helpers, the other X11 probes, generic Qt popup tests
  and the Present tests.

**Update the inbound references first:** `x11-compatibility-matrix.md` rows
81, 101 and 102 cite the removed commands. They should cite
`present_msc_ordering.rs` and the x11_wire SHAPE tests, and point to this
repository for the client trace. The claims in "What the prose claimed"
above should not carry over.

`tools/fixtures/panel_gate_*.kdl` are Sophia's own generic session fixtures,
used by `desktop_probe.rs` and `desktop_launch_reload.rs`. They stay.
