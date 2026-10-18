<!-- Provenance: the physical-runner guidance moved from Sophia docs/validation.md (the physical sections: output topology, frame-fed output, mirror and mixed output, keyboard independence, milestone 4/5 hardware proofs, standalone and benchmark runners) and docs/live-session-bootstrap.md (the guarded Kitty TTY3 session and persistent proof) at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13). Rewritten for this repository's commands and the prepared-inputs custody. -->
# Physical runners

These runners take real DRM, input and TTY ownership on the reference rig
(Void Linux x86-64, AMD Radeon RX 7900 GRE, DP-1 2560x1440 and DP-2
1920x1080, `seat0`). Each one is run deliberately by the operator from the text
console it names; none runs in any automated gate. The offline gate covers
their verifiers, archivers, identity preflights and static bounds instead.

Sophia rule 13 put them here: they name Hagia, Narthex, Kitty, xterm,
Firefox, vkcube, glxgears or zenity. Sophia keeps only generic primitives,
which the runners read from the staged pinned tree (never a checkout):
`run_sophia_session.sh`, `sophia_tty_mode.py`, `stop_sophia_session.sh`,
`lib/session_preparation.sh`, `lib/session_lifecycle.sh`,
`lib/drm_master_guard.sh`, `config/sophia/core.kdl`, the probe profiles
(`mirror_group_probe.kdl`, `mixed_output_probe.kdl`,
`frame_fed_output_proof.kdl`, `direct_scanout_*.kdl`,
`native_launcher_core.kdl`) and the retained verifiers and reporters (the product-neutral live-session
evidence verifiers `verify_live_session_{persistent,two_xterm,milestone3,milestone4,milestone5_gtk}_evidence.sh`,
`verify_atomic_scanout_preflight.sh`, `verify_native_egl_mixed_evidence.sh`,
`verify_sophia_native_composition_pixels.sh`,
`report_sophia_input_latency.sh`, `check_bounded_xterm_geometry.sh`).

## Shell timing evidence replay

`cargo xtask shell-timing latency HOST_LOG CLIENT_NDJSON MODE_MILLIHZ EXPECTED_OFFERS`
reads existing evidence without opening devices or starting a session. It
validates the complete client journal and joins every candidate to its exact
kernel-backed presentation before reporting nearest-rank p50, p95, p99 and
maximum latency. Use `60000` or `120000` for the exact refresh rate.

The report includes candidate-to-native-presentation, demand-to-permit and,
when recorded, separate presentation and probe upload-to-acceptance timings.
It retains all samples and lists every accepted upload that reached the
2,000,000 µs transfer bound. Missing upload measurements in older journals
are `null`, not zero. Rejected or missing outcomes invalidate the entire
report; they are never dropped to produce a successful subset.

This command summarizes all offered frames with no warmup exclusion. It does
not compare a pair, validate campaign conditions or decide B9 acceptance.
Keep the raw logs, including failed runs, alongside the report. CPU and
allocation accounting remain separate in `shell-timing cost-metrics`.

The prepared request, peer configuration and journal start name the image
`source`: `bytes` or `sealed`. Missing fields in older evidence mean `bytes`.
Sealed evidence requires the files wire and schema 2; the capture must match
the prepared source, wire and probe size. This labels the payload path without
claiming descriptor custody from a journal alone: that also depends on the
reviewed peer artifact and its SDK/export qualification.

Sealed peer configurations also require an `image` object with positive
`max_resource_bytes`, `max_width`, `max_height`, `max_admitted_outputs`,
`transfer_slots` and `transfer_timeout_ms`. The first is a 64-bit value; the
others are 32-bit values. Unknown or missing fields are refused. The reviewed
peer checks these caller bounds against its negotiated content limits; the
prepared artifact digest binds their exact values. Byte configurations omit
`image` and retain their existing behavior.

Cost reports retain the source and normalize CPU by both frames and logical
accepted payload bytes. `transferred_bytes` and the compatibility field
`uploaded_bytes` name the same denominator, including sealed payloads; neither
counts socket copies or memory bandwidth. `cpu_ns_per_frame` uses the same
outward-rounded bounds as CPU per MiB. Existing byte-path gate calculations
are unchanged. No report by itself establishes a paired performance pass.

## t331 panel and bulk cost diagnostics

The separate cost pilot freezes six prepared plans and nine attempts.
Schema 1 retains the original bulk-only contract. New schema-2 requests must
include `"workload": "panel"` or `"workload": "bulk"`; the frozen descriptor
and report retain that declaration. A panel pilot requires `probe_bytes: 0`
in every plan and capture; a bulk pilot requires `probe_bytes: 4194304`.
Neither kind can consume the other's captures. The arm names, ordering,
measurement methods and environment checks below are identical for both.
Panel-only follow-ups reuse the exact host and peer binaries from the bulk
pilot, changing only the workload configuration and necessary plan bindings.

The bulk workload uses 1000 changing frames with a 4 MiB transfer probe plus the 1920×24 panel per
frame, no warmup exclusion, and a fresh 60-second Session for each arm.
The selected target is DP-1 at 1920×1080, nominal 120000 mHz, scale 1, VRR off.
Keep the governor, affinity and scheduling identical across all arms; the
report refuses differences in retained environment fact values. CPU-only and
Rust allocation-counting builds must come from the same signed sources.

The request contains `schema`, `target` (the calibration target fields), a
`quiet_window` declaration, and `plans`; schema 2 also requires `workload`.
The plans object maps these six names to
absolute prepared-plan paths: `cpu-ipc`, `cpu-bytes`, `cpu-sealed`, `alloc-ipc`,
`alloc-bytes`, `alloc-sealed`. Profiles may differ only in their checked peer
executable, configuration and transport fields. The host and peer binaries
may differ across instrumentation sets, but not within a set. Prepared plans
and artifacts remain external, bound by their paths and digests; retain them.

```text
/absolute/prepared/xtask shell-timing prepare-cost-pilot REQUEST_JSON NEW_FROZEN_DIR
SOPHIA_SHELL_TIMING_ARM=1 /absolute/prepared/xtask shell-timing run-cost-pilot-arm FROZEN_DIR SHA256 ARM
/absolute/prepared/xtask shell-timing report-cost-pilot FROZEN_DIR SHA256
```

Run one arm at a time from the attended declared console, in this frozen order:

1. `cpu-1-ipc`, `cpu-1-bytes`, `cpu-1-sealed`.
2. `cpu-2-sealed`, `cpu-2-bytes`, `cpu-2-ipc`.
3. `alloc-1-ipc`, `alloc-1-bytes`, `alloc-1-sealed`.

Each attempt lives under `FROZEN_DIR/ARM/`, with `claim.json`, `attempt.json`
and `run/`. Claims bind the previous result digest. A missing, unfinished or
failed predecessor prevents subsequent attempts. A failed or unfinished arm
requires a new pilot; preserve the old evidence. Preparation and reporting
open no devices. Launch still requires the prepared runner image, source
checks, foreground console and explicit arm flag. The normal desktop remains
outside the experiment; this route launches no CPU load workers.

The report requires schema-3 counters, authenticated environment receipts and
complete native presentation joins for every arm. It retains Session, peer
and combined CPU, allocation-operation and minor/major page-fault bounds,
normalized per frame and logical MiB where applicable. CPU comparisons use
only CPU-only arms; allocation comparisons use Rust-global arms. Both CPU
blocks and their variation are shown separately. Differences use cross-bounds;
counter bounds are not statistical confidence intervals. One allocation
observation per variant does not establish repeatability.

All three paths use the same canvas, allocated inside the first measured
frame and rewritten for each upload. Sealed still creates, writes and seals a
fresh memfd each upload; SDK byte chunk allocations remain measured. Schema-3
sampling adds a getrusage read inside each snapshot's clock bounds. Page-fault
counts do not measure kernel page allocation, zeroing, copy volume or their
CPU cost. Old schema-1/2 captures remain replayable without invented fault
counts, but cannot supply this pilot. This diagnostic does not close t331,
produce a B9 verdict, or choose a transport.

## What every runner requires

### Paired sealed/bytes panel calibration

The t338 calibration measures the files byte and sealed paths without running
the full t331 campaign. Both plans must use the same CPU-only host, peer and
support artifacts, 1000 frames, a 24-pixel panel, no bulk probe and a 60-second
hold. They require environment readback. Freeze the selected card, output,
connector, 1920×1080@120 Hz mode, whole-number scale, VRR off and quiet-window
declaration before running either arm.

```text
/absolute/prepared/xtask shell-timing prepare-calibration REQUEST_JSON NEW_FROZEN_DIR
SOPHIA_SHELL_TIMING_ARM=1 /absolute/prepared/xtask shell-timing run-calibration-arm FROZEN_DIR SHA256 bytes NEW_BYTES_RUN
SOPHIA_SHELL_TIMING_ARM=1 /absolute/prepared/xtask shell-timing run-calibration-arm FROZEN_DIR SHA256 sealed NEW_SEALED_RUN
/absolute/prepared/xtask shell-timing report-calibration FROZEN_DIR SHA256 NEW_BYTES_RUN NEW_SEALED_RUN
```

The preparation request contains `schema: 1`, `target`, `quiet_window`, and
absolute `bytes` and `sealed` prepared-plan paths. The target contains `card`,
`output`, `scale_milli`, `connector`, `width`, `height`, `mode_millihz` and `vrr`.
Preparation is read-only apart from creating the frozen directory. Each run
still requires its declared foreground console, the explicit arm, exact signed
sources and prepared runner image. The daily desktop is not a target.

Run directories are create-only. Each retains `attempt.json` and a `run/`
subdirectory with the native capture, environment monitor evidence and receipt
sidecar. Preserve failed attempts; do not replace their files or present a
selected successful retry as the original pair. Replay rechecks digests,
process and sender provenance, the environment and kernel timestamp bindings,
then reports the sealed-minus-bytes cost difference. It supplies no IPC +10%
gate or campaign-condition verdict and launches no load workers.

Environment binding selects a verified host, its direct `/usr/bin/bwrap`
supervisor and that supervisor's sole peer child. Executable device/inode,
process start times and parent links are checked again around receipt intake.
Same-user non-dumpable processes such as Factotum may deny executable reads;
they are retained as unexamined and must be outside the selected chain and
supervisor subtree. Uniqueness is limited to examined candidates, not every
process in those opaque subtrees. No process is made dumpable for measurement.
The clock proof compares host, peer and capturer namespace links with Linux's
initial time namespace inode and requires zero monotonic offset; it does not
need permission to inspect the host's PID 1.

Use a short private run-directory path: `run/environment.sock` must fit the
Unix socket pathname limit. Binding or socket failures invalidate a run even
when the compositor completed its workload and restored the console.

### Frozen shell timing campaigns

The campaign commands run one explicitly selected arm at a time. They do not
start an automatic sequence or choose a connector. Freeze the complete manifest
and prepared plans first with `shell-timing freeze-campaign`; the schema-2
manifest fixes the output, card, applied scale, quiet window and all 108 arms.
Every prepared request must set `environment_readback: true`. Use the same
reviewed Sophia artifact for IPC bytes, files bytes and files sealed within
each instrumentation method.

From the declared foreground text console, invoke the **prepared runner
executable**, not `cargo run`:

```text
SOPHIA_SHELL_TIMING_ARM=1 /absolute/prepared/xtask shell-timing run-campaign-arm /absolute/campaign CAMPAIGN_SHA256 ARM_ID
```

Console, source, PAM-helper, runner-image and absent-run-directory checks happen
before claiming an arm. The launch repeats the checks. The claim is single-use:
failed attempts are retained and cannot be replaced. An unfinished claim,
including a failed final result write or changed predecessor chain, blocks the
campaign. Retain that evidence and freeze a new campaign; do not remove claims
or replace failed arms in place.

The runner binds a private environment socket before starting the wrapper.
It accepts host records only with kernel credentials, a matching live sender
pidfd and supervised process-identity brackets. Raw session-log lines never
supply environment authority. Start and end kernel, CPU-governor, online-CPU
and DRM-driver snapshots must agree. These snapshots and leader-thread
scheduling observations do not prove continuous stability between reads.

Load arms start exactly 16 copies of the prepared runner's bounded CPU worker,
before the monitor's capture window begins. They run at inherited scheduling
and affinity; qualification requires normal policy, nice zero, all online CPUs,
one thread, no descendants and positive CPU progress in every worker. The
runner retains bracketed start/alive observations and stops/reaps workers on
every normal error path. Only requested SIGTERM termination qualifies; timeout,
natural exit, SIGKILL or incomplete cleanup refuses the load. Worker lifetime
is arm runtime plus 120 seconds, including wrapper restoration and final
observation room. No load is started by replay or import commands.

Each arm retains `run/plan.json`, `result.json`, `session.log`,
`environment.json`, `environment-receipts.json`, and, for load arms only,
`load.json`. The attempt records their digests. Auxiliary split logs and samples
remain diagnostic; the importer rebuilds timings and costs from retained raw
evidence. `captured_only` means the run completed capture, not that any gate
passed.

After a complete condition, replay without devices:

```text
/absolute/prepared/xtask shell-timing report-campaign-condition /absolute/campaign CAMPAIGN_SHA256 idle
/absolute/prepared/xtask shell-timing report-campaign-condition /absolute/campaign CAMPAIGN_SHA256 load
```

The JSON report includes every arm's evidence digests and environment facts,
load intervals and CPU ticks, per-pair gate outcomes and reported-only pooled
distributions. Read the comparison verdict; producing a report is not a pass.
A missing, failed or inconsistent arm refuses the whole condition. Provenance
assumes the trusted owned runner and retained attempt digests; arbitrary edited
JSON is not authenticated. This campaign covers 120 Hz only and cannot close
the separate 60 Hz B9 requirement or allocation-attribution/read-pairing work.

### Prepared inputs

No runner builds anything. Binaries and the pinned Sophia tree come from
`cargo xtask prepare-physical-inputs` ([operations](operations.md#physical-gate-inputs)),
called by the runner through `tools/lib/physical_inputs.sh`:

| Variable | Meaning (no default for any) |
| --- | --- |
| `SOPHIA_SOURCE` | the clean Sophia checkout whose signed HEAD is `pins/sophia.toml`'s revision |
| `SOPHIA_GATE_BUILD_DIR` | a private (0700, owned) build directory outside every source tree |
| `CARGO_HOME` | the provisioned private Cargo home (`tools/provision.sh`) |
| `SOPHIA_INTEGRATION_XTASK` | this checkout's prebuilt `xtask` |
| `SOPHIA_SESSION_PREFLIGHT` | this checkout's prebuilt `active-session-preflight` |
| `SOPHIA_HAGIA_ROOT`, `SOPHIA_NARTHEX_ROOT` | the clean signed Hagia/Narthex checkouts (Hagia runners only) |
| `SOPHIA_HAGIA_NIM_DEPS[_SHA256]`, `SOPHIA_NARTHEX_NIM_DEPS[_SHA256]` | the reviewed Nim dependency manifests and their independently supplied digests |
| `SOPHIA_SESSION_TTY` | the target console, when not the controlling terminal |

The helper runs under a KILL deadline at the caller's priority and
`CARGO_BUILD_JOBS` (every CPU when unset). It stages every source as its exact
signed tree, builds offline and `--locked` into private targets, proves the
trees again, and writes a new read-only output. The runner then verifies that
output against its manifest sha256, reads `inputs.env` with a strict parser
(it is never sourced), binds this repository's signed commit, checks every
checkout is still clean at its signed HEAD, runs the atomic-scanout preflight
with the prepared binary, and verifies the inputs again before archiving. Sophia's session wrapper always receives
`SOPHIA_BUILD_SESSION=false`.

`crates/xtask/tests/physical_runner_bounds.rs` refuses any script under
`tools/` that builds, reads a source checkout's target, uses a Nim cache or
archives without that verification. Its pending list contains only
provisioning's self-check; its exempt
list is fixed (`tools/reload_policy_client.sh`, the operator's own WM reload
tool).

## Hagia gates

See [Hagia gates](hagia-gates.md) for the native session gate
(`tools/run_current_hagia_native_gate_tty4.sh`), the physical policy gate
(`tools/run_current_hagia_policy_gate_tty4.sh`), the combined critical path
(`tools/run_current_critical_path_tty4.sh`) and the offline Hagia smokes.

## Output topology loss and return

The revision-1 output file role has a separate
[native acceptance runner](output-file-native.md). Its Rust planner and verifier
run without devices; only its explicitly armed `run` command starts the staged
Session through the existing TTY recovery wrapper. It uses the independent SDK
peer, not the older startup-only frame-fed IPC-WM proof below.

```sh
tools/run_output_topology_gate_tty4.sh
```

From `/dev/tty4` with at least two connected outputs (the runner supplies the
arm and the `seat0` default); the operator disconnects
and reconnects one output. Hagia comes from the prepared inputs unless
`SOPHIA_HAGIA_BIN` names an external binary. The gate requires one
security-epoch barrier per change, complete `N - 1` loss and `N` return
publications with advancing generations, matching policy settlements, later
page-flip retirements, a surviving Kitty input proof and clean shutdown.

## Frame-fed output apply and rollback (historical)

```sh
SOPHIA_FRAME_FED_OUTPUT_ARM=1 tools/run_frame_fed_output_gate_tty4.sh
```

Historical. Phase two needs Sophia's startup-only
`--output-proof-rollback-after-apply` hook, which t272 retired at Sophia
`ae5a746c553584bff8bb54eeeeee8d2bc63431f8`. The launcher refuses a source
without the hook before preparing anything. Archived pairs remain verifiable
for their own pins. The native output gate (`xtask output-file-native`)
qualifies peer-transaction rollback. No current runner proves startup-transaction
rollback after physical apply.

Reference-rig specific (exactly DP-1 2560x1440 and DP-2 1920x1080). Phase one
applies, first-presents and publishes the checked-in profile; phase two forces
reverse-card rollback after final KMS acceptance. Both phases need distinct
physical text confirmation and clean teardown. A verified pair is archived under
`$XDG_STATE_HOME/sophia/promotion/frame-fed-output-runs/` by
`tools/archive_frame_fed_output_physical_run.sh` and re-verified by
`tools/verify_frame_fed_output_physical_archive.sh`. It changes real output
state: run it only with explicit operator authorization.

## Mirror group and mixed output

```sh
tools/run_mirror_group_gate_tty4.sh        # two heads, one mirror group
tools/run_mixed_output_gate_tty4.sh        # a mirror pair beside an extended head
```

Both perform real modesets from `/dev/tty4`. The mirror gate reads Sophia's
`mirror_group_probe.kdl` from the staged tree through a private 0600 copy,
scrolls a deterministic xterm workload, and asks for visible-pixel acceptance;
failures are archived as diagnostics (`tools/archive_mirror_group_diagnostic_run.sh`)
and passes as physical runs (`tools/archive_mirror_group_physical_run.sh`).
The mixed gate needs exactly three connected heads and archives with
`tools/archive_mixed_output_physical_run.sh`. The mirror-group corpus is
re-verified offline by `cargo xtask verify-archives`.

## Large presented-surface diagnostic and CPU profiles (t331)

Cost-pilot schema 3 uses `"workload":"panel"` and
`"presentation_height":512`. Its six prepared plans must request a 1920×512
surface, probe bytes 0, and exactly `reservation "top" 512` in the sole shell
component. The peer checks the granted geometry and presents that resource;
it is not the old 4 MiB probe that was retired without presentation. Each
frame changes all 3,932,160 bytes. This fits the existing 4 MiB resource and
512 logical-pixel panel limits. It is not fullscreen, a real-client benchmark
or a production-limit change. Schema 1/2 and the acceptance campaign retain
24px presentation heights.

The same nine-arm order and separate CPU/allocation builds apply. Schema 3
reports check every presented upload's shape and include raw native joins and
all-frame nearest-rank latency distributions. Timing is instrumented and has
no warmup exclusion. It is not a latency gate. The closed-loop peer waits for
completion before submitting another frame; slow uploads can reduce achieved
cadence without generating a rejected candidate. Do not call all such gaps
"missed deadlines" or treat 1000 successful frames as proof of 120 fps.

For explaining CPU differences, use a separate three-transport diagnostic
with identical **cpu-only** plans and the same pixels. Prepare one JSON
request per arm:

```json
{
  "schema": 1,
  "plan": { "path": "/ABS/plan.json", "sha256": "PLAN_SHA256" },
  "perf": { "path": "/usr/bin/perf", "sha256": "PERF_SHA256" },
  "target": {
    "card": "card0", "output": 1, "scale_milli": 1000,
    "connector": "DP-1", "width": 1920, "height": 1080,
    "mode_millihz": 120000, "vrr": false
  }
}
```

From the declared active console, with the prepared runner:

```sh
SOPHIA_SHELL_TIMING_ARM=1 /ABS/xtask shell-timing run-cpu-profile REQUEST NEW_DIR
/ABS/xtask shell-timing report-cpu-profile NEW_DIR > NEW_REPORT.json
```

`run-cpu-profile` rechecks the normal attended/source/artifact prerequisites
before creating its single-use directory. It invokes the pinned perf executable
with user-space `cpu-clock:u`, 199 Hz, 8192-byte DWARF stacks and the monotonic
clock, then runs the ordinary observed native runner as its child. Perf inherits
through descendants and threads. No system-wide recording, privileges or sysctl
changes are requested. The internal `capture-cpu-profile` command is the child
entry point; direct use does not produce a complete profile attempt.

Retain `profile.json`, `profile-result.json`, `perf-command.json`,
`perf-record.log`, `perf.data`, and the normal `run/` evidence. The report
rechecks hashes and reconstructs the workload and authenticated environment;
only then does it select the bound Session and peer outer TGIDs and the
whole-workload counter interval, including pixel preparation. Missing samples
or a missing/nonzero loss count refuse the report. Unknown leaf symbols and
all perf diagnostics remain visible. Keep the exact binaries available for
symbolization; symbol tables alone do not guarantee complete unwinding.

Self percentages are shares within each selected process's sampled user CPU,
not exclusive combined CPU percentages. Callchains overlap. Sampling perturbs
the workload, and user-space events cannot explain kernel copy, page-allocation
or zeroing cost. Never substitute these captures for unprofiled cost arms.
There is no automated winner or causal conclusion. Compare IPC/files/sealed
stacks and sample counts before choosing an optimization. This path has
fixture controls and a device-free real-perf thread/inheritance control;
protected native-session profiling still needs the attended run.

No builds may overwrite the measured binaries or runner while either diagnostic
is in progress. Preserve failed runs and use a new directory for any later
experiment, explicitly labelled as a new attempt.

## Keyboard independence

The acceptance path is an ordinary installed session: type on the keyboard you
will unplug, unplug it, type on the other, plug it back and type, then log out.

```sh
tools/verify_keyboard_independence_session.sh            # newest finished session
tools/verify_keyboard_independence_session.sh ~/.local/state/sophia/sessions/<id>
```

The optional attended gate adds the split emergency chord and the unshifted
proof phrase:

```sh
SOPHIA_KEYBOARD_A=/dev/input/by-id/...-event-kbd SOPHIA_KEYBOARD_B=/dev/input/by-id/...-event-kbd \
    tools/run_keyboard_independence_gate_tty4.sh
```

It runs from any text console but the display manager's, refuses while keyd or
another remapper runs, and archives through
`tools/archive_keyboard_independence_physical_run.sh`.

## Input latency

```sh
tools/run_sophia_input_latency_tty3.sh
```

From `/dev/tty3` with a writable `/dev/uinput` (`tools/setup_sophia_uinput.sh`).
Samples synthetic key injection end to end; the report comes from Sophia's
retained `report_sophia_input_latency.sh` in the staged tree. The archive's
`source.env` records the prepared inputs' manifest sha256.

## Terminal CPU path, standalone and benchmarks

```sh
tools/run_sophia_terminal_gate_tty3.sh
tools/start_sophia_vkcube_standalone_tty3.sh
tools/benchmark_sophia_vkcube_tty3.sh
tools/benchmark_sophia_glxgears_tty3.sh
tools/benchmark_sophia_glxgears_shake_tty3.sh
tools/benchmark_sophia_terminal_tty3.sh
tools/benchmark_xserver_graphics.sh
```

These take the prebuilt absolute `SOPHIA_BIN` (`runner_sophia_bin`) and never
build. Reporters (`report_sophia_*_performance.sh`,
`report_xserver_rendering_performance.sh`, `compare_sophia_xserver_rendering.sh`)
and verifiers (`verify_sophia_standalone_vkcube.sh`) are pinned by their
self-tests in the offline gate. Reference-stack numbers are never Sophia
correctness thresholds.

## Milestone proofs and the persistent runner

```sh
tools/live_session_milestone4_hardware_proof.sh
tools/live_session_milestone5_gtk_hardware_proof.sh
tools/native_egl_vkcube_mixed_smoke.sh
tools/run_x11_live_session_stability.sh [--diagnostic|--trace|--core] [--runs N]
```

`tools/live_session_persistent_hardware_proof.sh` is the shared runner behind
several callers; it never builds and requires the caller's prepared absolute
`SOPHIA_BIN`. The two-xterm and paired milestone 3 launchers are retired and
exit before touching any device (`tools/check_retired_milestone_launchers.sh`).

## Direct scanout

```sh
tools/direct_scanout_gate.sh [WIDTH HEIGHT HOLD WORKLOAD] [--overlay-proof] [--cost] [--cursor] [--atomic-cursor]
```

The wrapper prepares inputs with `--sophia-features=atomic-scanout-live` and
runs this repository's port of Sophia's direct-scanout gate through the
prebuilt `SOPHIA_INTEGRATION_XTASK direct-scanout-gate`. The gate verifies the
prepared inputs (this Sophia commit, atomic-scanout-live, this integration
commit), reads the direct-scanout fixtures from the staged tree and verifies
the inputs again before archiving. The archive verifier for
`direct-scanout-runs` stays in Sophia (`sophia_conformance::direct_scanout_archive`).

## Lom panel, launcher and dock gates

`tools/run_current_lom_panel_gate_tty4.sh [launcher|dock]`,
`tools/lom_gpu_content_hardware_proof.sh` select 9P with Lom `0d1ff046` and
consume verified prepared inputs. The panel gate also requests the pinned
Sophia profile-composition binary. Both require an absolute prebuilt
`SOPHIA_INTEGRATION_XTASK`; the panel gate additionally requires
`SOPHIA_SESSION_PREFLIGHT`. Their controls exercise preparation and launch
ordering; they do not establish an attended GPU pass. See
[Lom content](lom-content.md).

## Retired: IPC product coverage

Product IPC is being removed from Lom, Bemenu and Hagia. Physical and live
coverage that exercised a product over the current IPC wire is recorded as
retired together with that IPC, not moved.
