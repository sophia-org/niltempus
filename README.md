# niltempus

My desktop assembled from Sophia, Hagia, Lom, Bemenu and kleis. This
repository holds the flake that builds it, the desktop profile, the session
launchers and the tests and physical gates for this choice of components. It
is also an example for anyone assembling their own desktop. Sophia stays shell
and WM agnostic, and the C and Rust desktop SDKs are separate repositories.

## Building and installing

```sh
nix build .#desktop        # the whole release, as one store path
tools/desktop install      # copy it into /opt and make it current; log in again
tools/desktop rollback     # back to the previous release
```

Each component is a flake input, and `flake.lock` records exactly what a build
contains; `nix flake update` moves the inputs to their branch heads. The
profile is `profiles/desktop.kdl`. [Building and installing](docs/install.md)
covers the release, the profile, rollback, pruning and building with work in
progress.

Conformance tests for Sophia's generic contracts stay in Sophia; tests of this
desktop's components and their combinations live here. Session recipes select
shell components from the desktop profile.

## Documentation

- [Building and installing](docs/install.md).
- [Operations](docs/operations.md): the installed session's runbook, logs and
  recovery, bound Nim dependencies and prepared physical inputs.
- [Physical runners](docs/physical-runners.md): every attended hardware gate
  and what it requires.
- [Hagia gates](docs/hagia-gates.md) and [Hagia workspaces](docs/hagia-workspaces.md).
- [Narthex reference sheets](docs/narthex-reference-sheets.md).
- [Same-hardware desktop comparison](docs/desktop-comparison.md).
- [Lom content](docs/lom-content.md) (9P transport and scoped proof coverage).
- [Provlita's protected file-export join](docs/provlita-files.md) (device-free
  negotiation and the separate GPU boundary).
- [Quickshell](docs/quickshell.md): the X11 trace probe and panel probe, with
  the old coverage map.
- [E1 descriptor hosts](docs/E1-descriptor-hosts.md): retained coverage of the
  descriptor host modes (a 9P seam for the Narthex half).
- [Output file native](docs/output-file-native.md).

## Inputs and pins

- Sophia's public crates (`sophia-runtime`, `sophia-protocol`) come from
  `https://github.com/sophia-org/sophia.git` at the revision in
  `pins/sophia.toml`. No sibling path is named anywhere.
- `pins/c-desktop-sdk/manifest.json` is Sophia's vendored C SDK manifest at
  that revision; `pins/contracts.sha256` holds the sixteen protocol contract
  digests a client's SDK snapshot must carry.
- `assets/fonts/` holds the pinned test font and its license, copied from
  Sophia at that revision.
- A Bemenu artifact is named only by explicit arguments and environment
  variables; no client checkout path is built in.

`cargo xtask check-pins` checks, offline, that `pins/sophia.toml`, every Cargo
manifest and `Cargo.lock` agree on that one revision and that every copied
file still hashes to its pin. `cargo xtask audit-pins ABSOLUTE-SOPHIA-REPO`
re-derives the copied files and contract digests from a Sophia clone.

## Attended Lom, launcher and dock gates

The 9P Lom runners (see [Lom content](docs/lom-content.md)) moved
from Sophia (rule 13): the Lom GPU/content proof, the tty4 panel,
launcher and dock runners, their transcript verifiers, the workload budgets
and fixtures, and the `xtask dock` profile generator and verifier. See
[tools/probes/README.md](tools/probes/README.md). Every input is explicit:

    cargo xtask prepare-product-artifact lom|provlita|hagia REPO SIGNED-COMMIT NEW-DIR \
        --build-dir=/ABS [--nim-deps=/ABS --nim-deps-sha256=SHA]   # Hagia: reviewed deps required
    cargo xtask prepare-bemenu-artifact BEMENU-REPO SIGNED-COMMIT NEW-DIR
    SOPHIA_LOM_NATIVE_GATE_ARM=1 SOPHIA_SOURCE=/abs/sophia SOPHIA_GATE_BUILD_DIR=/abs/build \
    CARGO_HOME=/abs/private-cargo-home SOPHIA_INTEGRATION_XTASK=/abs/xtask \
    SOPHIA_SESSION_PREFLIGHT=/abs/active-session-preflight \
    SOPHIA_LOM_ARTIFACT=DIR SOPHIA_LOM_COMMIT=REV SOPHIA_LOM_SHA256=SHA SOPHIA_LOM_CONFIG_SHA256=SHA \
    SOPHIA_HAGIA_ARTIFACT=DIR SOPHIA_HAGIA_COMMIT=REV SOPHIA_HAGIA_SHA256=SHA \
    [SOPHIA_BEMENU_ARTIFACT=DIR SOPHIA_BEMENU_COMMIT=REV SOPHIA_BEMENU_SHA256=SHA] \
    [SOPHIA_PROVLITA_ARTIFACT=DIR SOPHIA_PROVLITA_COMMIT=REV SOPHIA_PROVLITA_SHA256=SHA \
     SOPHIA_PROVLITA_CONFIG_SHA256=SHA] \
        tools/run_current_lom_panel_gate_tty4.sh [launcher|dock]

These runs are manual (tty4, real hardware). They also require
`SOPHIA_GATE_BUILD_DIR` (an absolute private directory outside every source
tree; every build and the staged Sophia tree live there) and, per product,
`SOPHIA_<PRODUCT>_SHA256` (and `_CONFIG_SHA256` for Lom and Provlita), the
digests printed by the prepare command. `SOPHIA_SOURCE` must be a clean
checkout whose HEAD is exactly the signed revision in `pins/sophia.toml`; the
gate stages that revision's exact tree (`git archive`, tree hash proven) and
reads Sophia only from there. Source authorization (the signed revision) and
artifact binding (the expected digests) are separate checks. Rust products
build `--offline --locked`, so run `cargo fetch --locked` in the product
repository first. Their verifier
self-tests are part of the offline gate (`crates/xtask/tests/verifier_self_tests.rs`).

The read-only timing join is available separately:

    cargo xtask shell-timing collect CLIENT_NDJSON EXPECTED_OFFERS > CLIENT_JSON
    cargo xtask shell-timing metrics CLIENT_NDJSON EXPECTED_OFFERS
    cargo xtask shell-timing cost-metrics HOST_COUNTERS PEER_COUNTERS CLIENT_NDJSON EXPECTED_OFFERS
    cargo xtask shell-timing join HOST_LOG CLIENT_JSON MODE_MILLIHZ EXPECTED_OFFERS

`collect` reads the generic comparison peer's complete, paced journal. It
requires a start, a permit/offer/terminal sequence for every declared frame,
and matching final counts. Truncation, repeated records, changed identities,
backward clocks, rejection and records after the end all fail the command.
Keep the original journal: it also contains demand-to-permit times. Collection
validates the recorded sequence; it does not attest how the journal was captured.
The peer timestamps each offer before flushing its record and submitting it,
so the measured interval includes that logging cost on both wires.

Schema 2 also requires an upload offer and matching acceptance before each
permit. It binds the resource identity, dimensions and RGBA byte count to that
candidate. Upload time starts before flushing the offer record and sending
ResourceBegin, and ends after decoding Accepted; pixel preparation is excluded.
`metrics` exports these timestamps and demand-to-permit timestamps alongside
the candidate offers. It retains slow samples, including uploads over 2000 ms;
budget assessment is separate. Missing acceptance invalidates the whole journal.
Schema 1 remains readable for old diagnostics and has no upload measurements.
The native runner includes these metrics in its schema 2 report. Neither format
establishes CPU/MiB accounting or the full B9 verdict by itself.

The peer configuration may set `probe_bytes` to `4194304`; absent or zero keeps
the original workload. Each frame then uploads a separate 1024×1024 RGBA
resource, waits for Accepted, retires it and waits for Released before uploading
the ordinary bar. This measures the 4 MiB transfer bound without changing bar
geometry or pretending those pixels were displayed. Metrics label the probe
`transfer_probe` and the bar `presentation`; the probe's release is required.
The prepared configuration hash binds this choice, and capture rejects a
different declared probe size. Run latency-only bar measurements with the probe
disabled. A probe-enabled process counter interval includes both uploads and
their preparation and retirement; it is not a pure transfer CPU measurement.

`cost-metrics` requires a complete schema 2 journal and both counter streams.
Every upload, permit and terminal must lie inside its frame's counter interval.
The report divides whole-workload CPU by exact uploaded bytes (one MiB is
1,048,576 bytes), and allocation operations by the exact frame count. It keeps
probe bytes separate from presentation bytes and rounds bounds outward.
Native instrumented reports use the same calculation. Old offer-only `costs`
reports remain unnormalized; they cannot establish a bytes denominator.
CPU covers all threads in Session and the peer, including foreign code.
Allocations cover their Rust global allocators, counting alloc and realloc.
Both exclude other processes; allocation counts also exclude foreign allocators.
These explicit limits do not approve this scope as the full B9 acceptance method.

`HOST_LOG` contains captured `sophia_shell_native_binding` and
`sophia_shell_native_completion` records. `CLIENT_JSON` is an array of every
offered candidate, with integer `connection_epoch`, `content_grant_epoch`,
`output`, `candidate_generation`, `submitted_monotonic_usec`, and string
`outcome`. This paced workload requires every outcome to be `presented`.
The explicit mode must be exactly `60000` or `120000` mHz; the expected offer
count must match. The checker requires complete head bindings and kernel
timestamps, preserves repeated redraws, and reports the first exact completion.
Its `joined_only` result proves sample provenance, not B9 acceptance, artifact
identity or the completeness of the client recorder. The campaign must bind
those inputs and compare the paired distributions separately.

An attended native timing run has two separate commands:

    cargo xtask shell-timing prepare-native REQUEST_JSON /ABS/NEW_PLAN.json
    /ABS/PREPARED_XTASK shell-timing run-native /ABS/NEW_PLAN.json PLAN_SHA256 /ABS/NEW_RUN

Preparation reads files and signed source identities; it opens no devices and
starts no session. The request is a JSON object with these fields:

| Field | Required value |
| --- | --- |
| `sophia_source`, `integration_source` | Objects with an absolute repository `path` and full signed `commit`; both checkouts must be clean. |
| `artifacts` | Exactly `sophia`, `peer`, `wm`, `factotum`, `pam-helper`, `preflight`, `runner`, `core`, `profile`, `peer-config`; each has an absolute `path` and lowercase `sha256`. The first seven must be executable. The authenticator is required for both wires. |
| `tty`, `display`, `input_seat` | Explicit console `/dev/tty1`–`/dev/tty63`, private display number 90–99, and seat name. |
| `runtime_ms`, `frames` | 60,000–600,000 milliseconds and 1–10,000 offered frames. The peer reuses two resource IDs with fresh generations after retirement. |
| `wire`, `output`, `mode_millihz` | `files` or `ipc`, positive output ID, and exactly 60,000 or 120,000 mHz. |
| `instrumentation` | `none` for latency; `rust-global` for `comparator-allocation-counting`; `cpu-only` for `comparator-cpu-counting`; `profile` for `comparator-profiling`. Instrumented runs are limited to 60,000 ms. Build both binaries in the same mode; Sophia also requires `native-session`. |

The `progress` diagnostic uses `comparator-progress` on Sophia and its peer.
It records bounded numeric totals for owner/component passes, core server
turns/flushes, readiness handoff/fallback, core bytes and replies, pending-read
attempts, and peer content polls/waits. Core totals include all 9P servers in
the process; replies include errors and are not presentation completions.
Snapshots are independently timestamped, not atomic across counters. Use the
validated journal for offered frames and outcomes. Reports retain progress
samples with `costs: null`; cost/latency arms reject progress records. This mode
cannot be combined with CPU, allocation or phase profiling features and does
not establish a performance verdict. Native progress arms retain the same
60-second bound and attended-console checks.

CPU-only builds use the ordinary allocator. Their reports mark allocation
results as `null`, not zero. Profiling builds add bounded phase observations;
they cannot supply a cost or latency acceptance arm. Phase CPU belongs to the
calling thread, but allocation deltas include the whole process while each
scope is open. Scopes overlap and their totals must not be added. Match their
monotonic sample bounds to the workload interval before comparing service counts.
Neither mode subtracts estimated instrumentation overhead from a result.

The PAM helper and every ancestor of its canonical path must be root-owned
and not writable by group or others, matching Factotum's startup check. An
immutable file under a group-writable Nix store is not sufficient; select a
verified installed helper with the same bytes.

The flattened desktop profile must select exactly one bar component with the
prepared peer/config paths, denied GPU access, an explicit transport and empty
startup. Use `9p2000.L` for files or `comparison-ipc` in the isolated comparator.
Core and profile includes are refused. The peer JSON must match the declared
wire, output and frame count, with `height` 1–128 and `hold_msec` equal to the
full session runtime so it cannot restart after finishing the workload.
Validate the full profile with the prepared Sophia configuration checker too;
this runner checks workload bindings, not every desktop setting.

After authorization, execution requires `SOPHIA_SHELL_TIMING_ARM=1` on the
prepared console while that console is in the foreground. It checks all hashes
and signed source revisions again and requires the exact prepared runner.
The shared session wrapper owns the input guard, watchdog and console recovery;
the runner keeps its logs, bounds output, and verifies normal recovery. It then
preserves every client JSON record, extracts native witnesses, validates the
journal and writes `native-samples.json` only if every offer joins successfully.
No binaries are copied into evidence. `result.json` records wrapper capture and
recovery; `native-samples.json` reports `joined_only`, never B9 acceptance.

The source commits and executable digests are separate assertions: preparation
does not prove how an executable was built. Retain build commands and source
evidence. The expected refresh rate is checked against native completion
bindings; it does not change the monitor mode. The campaign still needs a
reviewed hardware/output selection, accounting scope and frozen
sample, warmup, load and comparison rules.

Allocation runs retain separate Session and peer counters. The peer records
before pixel preparation/upload and after presentation plus retirement; Session
samples cumulative Rust global-allocation and process-CPU counters every two
milliseconds, subject to scheduling. Each snapshot carries clock bounds. The
report brackets each peer frame with enclosing and enclosed host snapshots,
giving upper and lower counts without interpolation. It also brackets the full
workload once, preserving the between-frame work. Missing samples, process
changes, backward counters or incomplete windows fail verification. Allocation
operations include allocations and reallocations; foreign allocators and other
processes are outside this scope. The instrumentation's own work is included.

    cargo xtask shell-timing costs HOST_COUNTERS PEER_COUNTERS CLIENT_JSON

This command checks recording independently of native presentation. The native
runner requires both checks and rejects instrumented data in a latency-only run.
Counter bounds alone do not freeze the B9 accounting scope or warmup/aggregation
rules, and a diagnostic run cannot qualify the native path.

## Provisioning

    sh tools/provision.sh --cargo-home ABSOLUTE-DIR [--source ABSOLUTE-SOPHIA-REPO] \
        [--generate-lockfile | --update-lockfile]

This fetches every dependency into an explicit private `CARGO_HOME` that must
lie outside this repository and the Sophia source (inside it, Cargo's upward
workspace search from the pinned checkout's vendored SDK reaches this
repository's workspace and skips those packages), seeded by copying the operator's `~/.cargo/registry` index, cache
and src (never credentials or config, never a link back), and accepts it
(`.provision/accepted`, binding the pin's url and rev, the `Cargo.lock`
sha256 and the `CARGO_HOME` path) only after `check-pins` passes. `.provision/provision.log` records
whether the registry index or git database changed and every crate that was
downloaded. The default fetches the pinned revision from its public URL;
`--source` redirects that URL to a local clone for the script's own cargo and
git children only. A private `CARGO_HOME` rather than `cargo vendor` is used
because `sophia-conformance` builds only from an exact git checkout.
Afterwards every command is offline:

    CARGO_HOME=/abs/private-cargo-home cargo <command> --offline --locked

## Gates

All with a private target outside this tree and a hard timeout. Builds use
the caller's priority and every CPU; set `CARGO_BUILD_JOBS` (or prefix
`nice`) to limit them:

    export CARGO_HOME=/abs/private-cargo-home CARGO_TARGET_DIR=/abs/private-target
    timeout -s KILL 600 cargo run --offline --locked -p xtask -- check-pins
    timeout -s KILL 600 cargo run --offline --locked -p xtask -- check-provision
    timeout -s KILL 3600 cargo test --workspace --offline --locked
    timeout -s KILL 3600 cargo clippy --workspace --all-targets --offline --locked -- -D warnings
    cargo fmt --check
    timeout -s KILL 600 bash tools/check_lom_gpu_content_proof_verifiers.sh

The live Bemenu gate is ignored by default and fails closed on any missing or
mismatched input:

    cargo xtask prepare-bemenu-artifact SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR
    SOPHIA_BEMENU_ARTIFACT=OUTPUT-DIR SOPHIA_BEMENU_SHA256=BINARY-SHA256 \
    SOPHIA_BEMENU_COMMIT=SIGNED-COMMIT \
    cargo test --offline --locked -p live-tests --test bemenu_files -- --ignored --nocapture
