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

## What every runner requires

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
