<!-- Provenance: moved from Sophia docs/operations.md at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13). The installed desktop stack is packaged here now; installation starts from an explicit cargo xtask package-desktop release. S5 completes the doc set. -->
# Installed Sophia Operations

This is the operator runbook for an immutable Sophia release installed below
`/opt/sophia`. The ordinary desktop is the native Hagia WM plus Narthex shell.
Sophia does not package or run an unmodified legacy X11 WM as policy.

## Support Boundary

The retained physical reference is a Void Linux x86-64 host with an AMD Radeon
RX 7900 GRE (`amdgpu`), two DisplayPort outputs, and keyboard and pointer
devices on `seat0`. That identifies the proven combination; it is not a general
hardware promise.

An installed candidate requires:

- a local Linux virtual terminal supplied by greetd;
- an absolute, user-owned `XDG_RUNTIME_DIR`;
- a working libseat provider and readable udev input discovery;
- atomic KMS plus GBM/Mesa, libdrm, libudev, libinput, and libxkbcommon;
- Bubblewrap 0.11.2 or newer at `/usr/bin/bwrap` for Hagia/Narthex;
- Bash, Python 3, GNU core utilities, procps, Kitty, Helium, and xterm.

X11 applications connect to Sophia's X Authority. Hagia connects directly to
`sophia_wm_v1`, and Narthex connects directly to `sophia_shell_v1`. These are
separate supervised protection domains. X Authority is not an X11 policy
environment and does not expose root-window WM authority.

## Installation And Session Entries

Package an explicit release from this repository (clean, signed HEAD), the
pinned Sophia checkout and a prepared Hagia/Narthex pair, then install it:

```sh
cargo xtask prepare-wm-pair --hagia /ABS/hagia <commit> --narthex /ABS/narthex <commit> /ABS/wm-pair \
    --build-dir=/ABS/private-build \
    --hagia-nim-deps=/ABS/hagia.nim-deps --hagia-nim-deps-sha256=<reviewed sha256> \
    --narthex-nim-deps=/ABS/narthex.nim-deps --narthex-nim-deps-sha256=<reviewed sha256> \
    --hagia-c-sdk-rev=<the C SDK revision Hagia vendors>
cargo xtask package-desktop --sophia-root=/ABS/sophia --sophia-rev=<pinned rev> \
    --wm-pair=/ABS/wm-pair --wm-pair-commits=<hagia>,<narthex> \
    --wm-pair-sha256=<hagia>,<narthex> --wm-pair-profile-sha256=<default.kdl> \
    --wm-pair-c-sdk-rev=<the same C SDK revision> \
    --build-dir=/ABS/private-build --out=/ABS/release
tools/install_live_session.sh /ABS/release
```

Hagia and Narthex build only from REVIEWED Nim dependency manifests (below);
the pair records both manifests and their digests, plus Hagia's vendored C SDK
revision and manifest (pair schema 3).

Packaging never switches or overwrites your own default window manager
(`$XDG_STATE_HOME/sophia/bin/hagia` and its reload workflow); an installed
session still prefers that user-owned client and falls back to the packaged
pair. The installer verifies every artifact digest,
installs a new immutable directory below `/opt/sophia/releases`, and atomically
updates `current` while retaining the former release as `previous`. Activation
and rollback validate the complete target surface before changing command links
or greetd entries.

A native-only schema-7 artifact records the Sophia commit and whether Hagia is
included. A Hagia artifact additionally records its signed source commit, the
canonical default-profile digest, Hagia and Narthex executable digests, and
Hagia's vendored C SDK revision and manifest digest. The SDK manifest itself is
sealed at `share/sophia-policy/hagia/c-sdk.manifest.json`. A schema-6 artifact
is not accepted as a candidate.
Installation rejects missing, non-executable, or mismatched artifacts. Legacy
WM executables, compatibility configuration, and bridge fields are forbidden.

### Bound Nim dependencies

`prepare-wm-pair`, `prepare-product-artifact hagia` and
`prepare-physical-inputs` share one builder. A Nim product builds only from a
reviewed dependency manifest whose sha256 root or the operator supplies
separately; nothing picks a version or looks packages up implicitly.

1. Draft a closure for review. Every requirement, transitively, needs an
   explicit `--pin`; `.nimble` files are read as data (nimble never runs), and
   an unsupported `requires` expression fails by name:

   ```sh
   cargo xtask nim-deps draft --store=/ABS/.nimble/pkgs2 --source=/ABS/hagia \
       --commit=<signed commit> --product=hagia --nim=/ABS/nim --nim-lib=/ABS/nim/lib \
       --gcc=/ABS/gcc --bwrap=/ABS/bwrap --build-dir=/ABS/private-build \
       --pin=chronicles=<version> ... --out=/ABS/hagia.nim-deps
   ```

   The printed digest is a DRAFT digest, never an authorization.
2. Root reviews the file (every package's provenance and complete file
   inventory, the toolchain records), changes `status=draft` to
   `status=reviewed`, and supplies the final file's sha256 separately.
3. The builder refuses a draft, a digest mismatch, or a manifest reviewed for
   another commit. It stages the closure read-only in a private scratch under
   `--build-dir` and verifies it before and after the build, and requires the
   host toolchain to be the reviewed one before and after.
4. The compiler is a verified, read-only STAGED copy of the reviewed Nim
   installation (`bin/nim`, its installation configuration `config/` and its
   stdlib `lib/`) in that scratch; nothing reads the live installation, which
   is hidden inside bwrap together with /home, /opt, /root and /etc/nim, with
   no network. The installation configuration is kept (one approach): before
   the build, `config/nim.cfg` (and every file it `@include`s, which must lie
   inside the staged configuration) and `config/config.nims` are traced as
   data, and any directive that applies, or cannot be shown not to apply, to
   this build and names an ambient or unresolved input (a host path, `$HOME`
   or another environment expansion, `@putenv`, a tool or compiler path, an
   implicit import, or in config.nims any file, process or environment call
   or path switch) is refused. `path="$lib/..."` resolves inside the staged
   stdlib; `nimblepath` entries are recorded as disabled (`--noNimblePath`).
   User, parent and project configurations stay skipped
   (`--skipUserCfg --skipParentCfg --skipProjCfg --clearNimblePath`).
5. Every artifact records the effective nim command line, the staged
   configuration and stdlib inventory digests (checked against the reviewed
   manifest) and each configuration file read, with its sha256.

Host-toolchain identity (nim, its standard library and installation config,
gcc, cc1, as, ld, bwrap, and the owning host packages) is recorded and
re-checked; it is NOT a fully reproducible closure. The host toolchain is
identified, not rebuilt.

### Physical gate inputs

Physical runners never build in a source tree. They take a prepared,
read-only input directory:

```sh
cargo xtask prepare-physical-inputs --sophia-root=/ABS/sophia --build-dir=/ABS/private-build \
    --out=/ABS/NEW --sophia-features=native-session|atomic-scanout-live \
    [--sophia-packages=sophia-cli,sophia-wm-demo] \
    [--hagia=/ABS/hagia --hagia-commit=<c> --hagia-nim-deps=/ABS --hagia-nim-deps-sha256=<s>] \
    [--narthex=/ABS/narthex --narthex-commit=<c> --narthex-nim-deps=/ABS --narthex-nim-deps-sha256=<s>] \
    [--profile=hagia:examples/config/default.kdl ...]
cargo xtask prepare-physical-inputs verify --out=/ABS/NEW --manifest-sha256=<printed sha256>
```

The output holds `bin/`, the exact pinned Sophia tree (`sophia-tree/`, the
runners' `SOPHIA_ROOT`), profiles copied from their owning staged source,
the reviewed dependency manifests, `inputs.env` (read by a strict parser,
never sourced) and `physical-inputs.manifest`. `verify` requires the expected
manifest sha256.

The physical runners call the helper themselves (tools/lib/physical_inputs.sh)
and never build: they need the prebuilt recipe tool of this checkout
(`SOPHIA_INTEGRATION_XTASK`), the pinned checkout (`SOPHIA_SOURCE`), the
provisioned `CARGO_HOME`, a private `SOPHIA_GATE_BUILD_DIR` (0700, outside
every source tree) and, for Hagia and Narthex, `SOPHIA_HAGIA_NIM_DEPS`,
`SOPHIA_HAGIA_NIM_DEPS_SHA256`, `SOPHIA_NARTHEX_NIM_DEPS` and
`SOPHIA_NARTHEX_NIM_DEPS_SHA256` (reviewed manifests, no default). They read
Sophia's retained files from the staged tree, run the atomic-scanout
preflight with the prepared binary (Sophia's own preflight script
`cargo run`s in its tree and is never called), hand Sophia's session wrapper
`SOPHIA_BUILD_SESSION=false`, and verify the prepared inputs again before
archiving. `crates/xtask/tests/physical_runner_bounds.rs` refuses any script
that builds, reads a source checkout's target, uses a Nim cache or archives
without that verification. Its pending list (the Lom scripts, frozen for the
Lom lockstep; provisioning's self-check) may only shrink; its exempt list is
fixed: tools/reload_policy_client.sh, the operator's own default-WM reload
tool (not a gate; its conversion belongs with the niltempus prepare/reload
path).

Every release installs this base entry:

- `Sophia Kitty (Baseline)` — one application, no WM or shell.

A release built with explicit `SOPHIA_HAGIA_BIN` and
`SOPHIA_HAGIA_SHELL_BIN` paths also installs:

- `Sophia Hagia (Native Policy)` — the ordinary user-profile session;
- `Sophia Hagia (XTEST automation)` — the same session with synthetic input
  admitted, so a client on the display can drive it.

Every release also carries the proof and evidence profiles below, and installs
their operator commands, but leaves them out of the login menu unless
`SOPHIA_INSTALL_PROOF_SESSIONS=1` is set for the install. They are diagnostics
run deliberately, and listing them by default put four of them in front of the
two entries someone logs in with:

- `Sophia Native Chrome Proof` — a bounded Engine-chrome diagnostic;
- `Sophia Hagia Promotion (Packaged Default)` — immutable release evidence;
- `Sophia Firefox Proof` — the integrated browser workflow;
- `Sophia Recovery Proof` — a bounded watchdog/recovery gate.

The XTEST entry above is listed with the ordinary sessions rather than held
here, because the request that lists these is all of them or none and asking
for that one would put five entries in front of the two someone logs in with
— the crowding this rule exists to prevent. It is a session to work in, not a
gate to run occasionally. What it admits it declares. On it, any admitted client can synthesise input to any
other: that is what XTEST has always meant on one X display, it is same-uid
as every other client, and it is wider than the session to browse the web in.
It cannot forge physical evidence, and not by policy — synthetic input enters
at the broker, downstream of the session's physical input phase, so no amount
of injection raises `physical_events`, `physical_keys_routed` or
`physical_pointer_routed`. It also says what it is: `sophia_live_session_xtest
schema=1 status=admitted` at startup and the injected counts at completion.
So a run from it drives a scenario and never accepts one; an `@physical` row
still wants a hand on the keyboard.

Activating without the request retires proof entries a previous install
listed, matching only entries whose `Exec` line points into the install prefix,
so an operator's own file at the same path is preserved.

The generic `sophia-session` command is an internal launcher and requires an
explicit native profile. It has no legacy-WM profile and no compatibility
fallback. Activating a native-only release removes stale Sophia-managed legacy
entries, but preserves unrelated files or links at the same paths.

## Status And Logs

From the session or an independent text VT:

```sh
sophia-status
```

Status verifies the current release checksums and reports current/previous
targets, relevant processes, lifecycle outcomes, runtime identity, and the
newest native proof attempts.

Durable user evidence lives below
`${XDG_STATE_HOME:-$HOME/.local/state}/sophia/`:

| Evidence | Path |
| --- | --- |
| Ordinary session identity, events, guard, recovery, lifecycle | `sessions/SESSION_ID/` |
| Current ordinary Hagia record | `hagia-session/current/` |
| Explicitly preserved diagnostic snapshots | `session-investigations/` |
| Kitty fallback session | `kitty-session/` |
| native diagnostics | `native-session/` |
| installed launch and runtime identity | `installed-session/` |
| Legacy Hagia attempts and explicit proof coverage | `promotion/hagia-runs/` |
| packaged-default Hagia attempts | `promotion/hagia-promotion-runs/` |
| Firefox, xterm, and TrueColor attempts | `promotion/{firefox,xterm,truecolor}-runs/` |
| fallback, emergency, watchdog, and native-chrome attempts | `promotion/{fallback,emergency,watchdog,native-chrome}-runs/` |

Legacy/proof log paths retain at most one `.previous` generation. Daily sessions
use the rolling history described below. Immutable proof attempts bind
the Sophia executable, relevant native policy executable, selected profile
identity, lifecycle, recovery, and reduced session evidence. Personal
configuration contents are never copied into an archive. Logs must not contain
typed text, clipboard data, window titles, or application content.

## Mark and investigate a problem

Installation exposes `sophia` in `/usr/local/bin`; it follows the selected
release through activation and rollback. On older installations that lack this
command, use `/opt/sophia/current/target/release/sophia` with the same arguments.

In an ordinary installed session, use a terminal or switch to another TTY and run:

```sh
sophia session mark "window stopped responding"
```

This command writes a local marker without asking the desktop to respond. It
selects the sole live session. If more than one session is live, select its ID
with `--session=ID`. After a crash, select the previous launch explicitly:

```sh
sophia session mark --session=latest "previous session crashed"
sophia session inspect latest
sophia session keep latest
```

The marker command prints the exact session and marker IDs and the command to
inspect that marker. `inspect ID --marker=MARKER_ID` shows events within sixty
seconds of either side of the marker, as far as retained evidence permits. A
marker written after the session ended is a report time, not an inferred crash
time; inspection shows the final retained minute. `sophia session list` lists
retained launches and storage totals. `sophia-status` includes that listing.

Each launch gets its own record before graphics takeover. Its manifest binds
Sophia's executable digest and installed release identity. The identity journal
records loaded core/desktop profile digests, WM profile activation, and the
executed WM and native-shell digests at each connection epoch. Hashing runs
separately from event recording; a pending or unavailable digest is not an
assertion about which executable ran. Component-private configuration remains
owned by its component and is reported as unobserved. File contents are not
copied, and editing a profile later does not change a recorded digest.

Events carry a write sequence, UTC milliseconds, boot-clock milliseconds, and
an approved Sophia record, separated by tabs. The manifest identifies the boot.
The boot clock permits correlation across wall-clock corrections. Identity
records may arrive after ordinary events; inspection orders events by their
observation time. Guard and TTY recovery records retain their existing schemas.
VT lifecycle fields and approved failure codes survive reduction. An
`unclassified` failure means the cause has no approved code; it does not imply
that the raw error was retained. Application content and arbitrary error text
remain excluded.

Interaction records retain counts for input-lease and pointer-grab outcomes,
focused and unfocused frames, and available Present timing feedback. Each
observed pointer-button batch records observed, routed, and suppressed counts,
including after the first successful click. These counts distinguish session
routing from an application that fails to respond; they do not prove that the
application consumed an event. Coordinates, button and key codes, and application
content remain excluded. Present detail records still require their existing
diagnostic setting; retaining their fields does not enable that setting.

For a diagnostic launch, set `SOPHIA_LIVE_VISUAL_PROGRESS=1` in the environment
of the session process before launch. The default is off. This enables reduced
visual-progress records and Present feedback detail in the existing bounded
recorder; it does not change rendering policy. The value `true` also enables it.

`content stage=offered` identifies CPU updates or Present submissions offered
to a production cycle, not their acceptance. `committed_snapshot` identifies an
observed committed surface generation and its buffer source. Surface tokens are
salted for that session and contain no raw XIDs, titles, pixels or checksums.
`head_snapshot` reports observed pending, rendering, submitted and presented
frame identities, plus cumulative submission and retirement counts. It is a
snapshot, not an exhaustive frame journal. A baseline has no inferred history;
`missed_count` counts intermediate counter transitions not individually seen
between snapshots, not distinct lost frames.

`feedback_ready` means retirement has authorized the feedback. A subsequent
feedback record with `routed=true` means it was queued to the frontend
connection, not that the client consumed it. Check the recorder's discarded
and storage-error counts before drawing conclusions from absent events.
Application stderr is separate private evidence. Recorded daily sessions capture
it by default for startup, shortcut and catalog launches. Use
`sophia session launches latest` to find the session-scoped launch ID, then
`sophia session stderr latest --launch=ID`. This displays escaped bytes, including
terminal controls and invalid UTF-8. `--raw` deliberately writes the original
retained bytes; redirect it to a private file rather than a terminal. No command
can recover stderr discarded by an older release or a launch outside capture.

Launch metadata records the requested executable (privately), source, transaction when
available, request/spawn times, spawn outcome, and observed exit code or signal.
It does not record arguments, environment, URLs or terminal input. Children
inheriting the pipe belong to that initiating launch; their output does not prove
the initiating process was still alive. Exit observation, pipe EOF, queued bytes
and synchronized storage are separate facts. A successful spawn is not evidence
that an application mapped a window or finished starting.

Capture permits 64 concurrent registrations. A single nonblocking collector
visits each stream once per turn and reads at most 4,060 bytes from each. It
yields after productive turns and sleeps five milliseconds only when idle;
discarding a flood is not throttled by a fixed timer. One separate storage worker owns filesystem
writes and synchronization. The aggregate queued record storage is at most
1 MiB; the worker additionally owns one record. Retention stops after the first
1 MiB read from each launch, but pipes continue draining and discarded bytes are
counted. Four rotating 4 MiB binary segments bound the session application store.
Metadata shares that store and may rotate out. Stream-capacity or collector
setup failure leaves application execution intact and records unavailable
capture; it does not promise an exit record for that unregistered launch.

Storage refusal or a full queue discards diagnostic bytes without retrying the
application. Inspection reports original stream offsets and missing ranges.
`application-health` reports rotation, refused registrations, storage errors,
lost final metadata and synchronization status; it is private and shown by
`session launches`. A missing or stale health record is not a healthy capture.
Shutdown allows 250 ms for collection/storage completion. Descendant-held pipes
and slow storage cannot hold logout; an unfinished tail is reported, not invented.

Set `diagnostics application-stderr=#false` in the core KDL to disable retention.
Live disabling drains existing pipes without keeping new bytes; already queued
bytes may still be written. Re-enabling applies to subsequent launches, not the
old pipes. Launch outcome metadata remains available. Stdout is unchanged, and
proof runs keep their existing output ownership.

Ordinary logout reports lifecycle and cleanup success independently of X11
error replies. Those replies remain compatibility evidence in
`sophia_live_session_protocol_error_tally` schema 3: bounded major/minor/error
codes, retained counts, discarded observations, and a cumulative total. A
nonzero tally has status `compatibility_refusals`; it does not establish a
session failure or certify that the application worked. Explicit application
proofs still require zero unexpected protocol errors.

`sophia_session_failure` records the owning phase and an approved cause before
the session adds request-tally context to its error. Runtime failures retain
their original phase across cleanup. Check preceding runtime-fatal records
for typed causes that cleanup's contextual error may no longer carry. Missing
or `unclassified` causes require further investigation; successful TTY recovery
alone does not establish a successful session.

Quiescence schema 3 records `pending_control_count` on completion and timeout.
Successful shutdown requires that count to reach zero through acknowledgement
processing; frontend EOF alone does not settle commands issued by final layout
work. Queue, acknowledgement, and quiescence deadlines remain bounded.

Resource observations continue every five seconds throughout an ordinary
recorded session. Storage keeps four event segments of at most 15 MiB each,
with separate bounded identity and marker journals. Within a segment, one
record name may write at most a quarter of it; beyond that its records are
suppressed so a single high-volume kind cannot rotate every other kind out of
the history. The first record a name loses is reported where it happens, as a
`sophia_session_record_budget schema=1 status=share_spent` record naming the
kind, so a gap is visible in the segment it opens in rather than only in a
total. A segment that closed with suppressed names begins the next one with a
`status=suppressed` record per name and the count it lost. A busy client
reaches its share in well under a minute, which is the bound working: the
dominant kind is truncated so the segment keeps far more history of every
other kind.
Ordinary session volume is far below the share, and the identity journal is
never suppressed. Automatic history retains
at most twenty finished sessions within a 1 GiB budget, reserving space for the
active session's 80 MiB allowance, including application diagnostics. Active sessions are never deleted. If active
sessions alone exceed the budget, their protection takes precedence.

`keep ID` copies structured evidence currently available into a private, checksummed
snapshot outside automatic pruning. A running-session snapshot records its
cutoff and is incomplete. Marking alone does not exempt a session from pruning;
keep the evidence when an investigation needs it. Preserved snapshots remain
until you remove them, and their storage total is reported separately. Existing
archives are not migrated or pruned.

Ordinary `inspect` and `keep` exclude private application records. Include them
only with `sophia session keep latest --include-application-stderr`; the snapshot
copies the exact binary records and hashes them without decoding or changing
their bytes. Directories are private (0700), files are 0600, and unsafe ownership,
symlinks, hardlinks and nonregular files are refused.

Recording uses bounded queues and synchronizes periodically. Its health record
reports discarded records, rotated bytes, records suppressed by the per-name
share, storage failures, and the last confirmed synchronization time. An abrupt power loss can lose the newest
unsynchronized tail. An unfinished record with no live owner is `interrupted`;
Sophia does not invent an exit code or crash cause. A clean process exit is
`exited`, a nonzero exit is `failed`, and neither is a proof verdict.

The structured daily recorder accepts Sophia's evidence, not mixed application
stdout/stderr. The separate private stderr store above does not feed this stream.
Sensitive fields and arbitrary strings are excluded from structured events. Labels are
operator-supplied local notes, limited to 256 UTF-8 bytes without control
characters. Session directories require mode 0700 and files mode 0600; unsafe
owners and links are refused. The ownership manifest contains the minimal host
process/boot identity needed to distinguish a live wrapper from PID reuse;
`inspect` does not print that private control identity. No new scripting socket,
namespace grant, WM metadata, tracing, or pixel capture is enabled.

Diagnostic failure is reported without replacing the ordinary session's exit
status. Explicit proof/promotion workflows retain their exact evidence and
strict verifiers. For a recorded daily session, use `inspect`; to verify a
legacy Hagia archive, pass its directory explicitly to `sophia-verify-hagia`.

## Normal Stop

Use `Ctrl+Alt+Delete` for ordinary Hagia logout. A customized profile may choose
another nonreserved binding. Normal logout commits the policy action, drains
presentation and application ownership, restores the VT, and returns to greetd.

If the graphical session cannot accept the shortcut, use an independent text
VT as the same user:

```sh
sophia-stop
```

The command signals the outer session wrapper. Do not kill Sophia, Hagia,
Narthex, Kitty, or Firefox individually; that bypasses the owner responsible
for bounded cleanup. `sophia-stop hagia` remains available when an explicit
profile is useful.

## Emergency Recovery And Fallback

An ordinary installed login arms the independent input guard automatically,
after it opens keyboard input and before graphics takeover. There is no login
chord to rehearse. If rendering and routed input become unusable, press
`Ctrl+Alt+Backspace` once for emergency recovery. The guard runs outside the
compositor, WM, and shell; the supervisor ends the session process group,
restores keyboard/KD/termios state, and returns control to greetd.

Launch preparation runs in the packaged Sophia binary: controls, executable
discovery, argument/environment vectors, private proof inputs and exact command
acceptance. Installed startup requires no Cargo or xtask. Preparation refuses
an unsupported binary instead of treating a zero exit as acceptance. The
Kitty override parser and final session argument parser have ten-second
deadlines and process-group cleanup; neither starts a graphical session.
The shell adapters retain TTY/display-manager handoff, the independent input
guard, watchdog and restoration. Development builds supply the matching
validator before controls are checked; installed startup forbids builds.

Development launchers and installed proof or promotion sessions retain manual
arming: press and release the chord when prompted, then use it again if recovery
is needed. Set `SOPHIA_INPUT_GUARD_ARMING=manual` to request this check for an
ordinary installed login too. The underlying `sophia session input-guard`
command accepts `--arming=manual|automatic` and defaults to manual. Neither mode
disables recovery or permits graphics takeover without guard readiness.

After greetd returns:

1. Select `Sophia Kitty (Baseline)` to isolate the core display/input path.
2. Exit Kitty normally.
3. Run `sophia-verify-fallback` from a text VT.
4. Inspect `sophia-status` before retrying Hagia.

`Sophia Recovery Proof` exercises the process-external watchdog. Verify its
latest archive with `sophia-verify-watchdog`. An independent emergency chord
from a Hagia session is archived separately and verified with
`sophia-verify-emergency`.

## Rollback

From an independent text VT after the current session has ended:

```sh
sophia-status
sudo sophia-rollback
sophia-status
```

Rollback swaps `current` and `previous`; it does not edit either immutable
release. A second invocation swaps the same pair back.

## Native Evidence Workflows

Explicit proof and promotion sessions reserve an immutable attempt before
graphics takeover. Verified normal logout records `status=passed`, emergency
recovery records `status=recovered`, unexpected exits record `status=failed`,
and interruption before finalization leaves `status=pending`. Ordinary daily
sessions use the diagnostic history above.

Verify retained legacy and packaged-default proof sessions with:

```sh
sophia-verify-hagia /absolute/path/to/legacy/archive
sophia-verify-hagia-promotion
```

Focused X11-application proofs remain commands rather than desktop policy
profiles:

```sh
sophia-xterm-proof
sophia-verify-xterm-runs 1

sophia-truecolor-proof
sophia-verify-truecolor-runs 1
```

Both run under Hagia/Narthex. The xterm proof covers CPU-backed placement,
work-area reservation, VT switch/resume, page-flip retirement, and clean
logout. The TrueColor proof covers core color requests, CPU and DMA-BUF
composition, independent output readiness, retirement, and exact recovery.

`Sophia Native Chrome Proof` records the ordered ring/frame sequence and is
verified with `sophia-verify-native-chrome`. Firefox proof attempts are checked
with `sophia-verify-firefox-runs`.

There is no installed cycle or two-hour soak gate. Long durability runs are
optional overnight diagnostics and do not block product work. Historical
bridge-era evidence remains in the Git history and roadmap archives; it is not
installed, re-executed, or accepted as current native-policy evidence.

## Known Limitations

- Sophia is a native X11 application-server candidate, not a full Xorg
  replacement. Compatibility is limited to the admitted operations in the X11
  matrix.
- Wayland application protocol support is intentionally absent.
- Only the retained AMD reference has physical daily-driver evidence; other
  drivers, topologies, and architectures remain unproven.
- Applications that require a desktop portal, notification service, or
  accessibility bus are outside the current support boundary.
- Only one previous release is addressable through rollback. Preserve older
  immutable release directories until a successor has clean Hagia/Narthex and
  recovery evidence.
