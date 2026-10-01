# Output file native acceptance

This gate joins the independent C-SDK peer to Sophia's native output owner.
It runs validate, admission rejection, commit A-to-B-to-A, and peer death after
all cards apply B. T253 requires this evidence before T272 removes output IPC.
The C executable is a protocol proof; the future Rust `sophia output` product
command belongs to T254.

## Prepare without devices

First prepare the generic peer in the clean signed Sophia checkout:

```sh
cargo xtask check output-file-native-proof prepare --output=/PRIVATE/NEW/PEER
```

Prepare physical inputs separately with this repository's existing
`prepare-physical-inputs`: `native-session`, the signed Hagia product and its
reviewed Nim dependencies, and a complete desktop output profile. A profile
owned here uses `--profile=integration:RELATIVE/PATH`; it is copied from this
repository's signed commit. Preparation builds; execution never does.
Preparation runs at the caller's priority; Rust and Nim builds use the
caller's `CARGO_BUILD_JOBS`, or every CPU when it is unset.
The Sophia revision, SDK manifest and contracts must match this repository's
pins. Hash `prepared.json` and the host `active-session-preflight` binary too.

The run planner needs explicit A/B layouts, not hardware discovery. A names
the profile's committed startup topology epoch, enabled heads, modes, transforms,
VRR, groups and primary group. B changes a mode timing, with the same enabled
heads and connector/CRTC/plane selections. Use the peer's compact layout grammar:
heads `HEAD:MODE:TRANSFORM:VRR,...`; groups
`OUTPUT@X,Y,WxH=HEAD/MAP[+HEAD/MAP][;...]`; primary is a zero-based group index.
Quote group values in the shell. Revision 1 cannot report current transform
or VRR: their declared values must agree with the baseline profile.

`validation/output-file-native/baseline.kdl` declares the reference rig's A:
DP-1 at 2560×1440/120 Hz and DP-2 at 1920×1080/60 Hz, side by side,
normal transforms, scale 1 and VRR disabled. It is a proof profile, not a
replacement for the user's profile. Before sealing a run plan, bind explicit
head/mode IDs from a reviewed topology inventory and choose a supported B
timing. A different connected set or missing mode must refuse qualification.

The proof profile starts a colored xterm running `/bin/sleep 600`, without a
user shell. Session's native completion check requires exported application
pixels; an empty desktop can apply the profile and restore the console yet
fail that check. Session owns the client and stops it during cleanup. Xterm,
sleep and their runtime dependencies are host prerequisites, like the wrapper's
host helpers; this does not claim a reproducible application closure.

```sh
cargo xtask output-file-native prepare-run \
  --inputs=/ABS/SEALED --inputs-manifest-sha256=SHA256 \
  --preparation=/ABS/PEER --preparation-sha256=SHA256 \
  --profile=integration/RELATIVE/PATH --out=/PRIVATE/NEW/PLAN \
  --preflight=/ABS/active-session-preflight --preflight-sha256=SHA256 \
  --tty=/dev/tty4 --display=:91 --input-seat=seat0 --runtime-ms=60000 \
  --a-topology-epoch=E --a-heads=HEADS --a-groups=GROUPS --a-primary=0 \
  --b-heads=HEADS --b-groups=GROUPS --b-primary=0
```

This writes `run-plan.json` with the exact four Session argument vectors and
prints its digest. It does not open DRM/input devices, start Session or install
anything. Review that file and the whole-release recovery route before requesting
an attended hardware window. Only private displays `:90`–`:99` are accepted. Runtime is bounded to 60–600
seconds per stage; the peer has 30 seconds per wait. Allow enough time for startup,
apply and physical restoration inside the Session runtime bound.

## Run only in an authorized attended window

From the exact prepared text console, with another VT available for recovery:

```sh
SOPHIA_FRAME_FED_OUTPUT_ARM=1 cargo xtask output-file-native run \
  --plan=/ABS/PLAN/run-plan.json --plan-sha256=SHA256 --out=/PRIVATE/NEW/RUN
```

The runner rechecks identities before and after each stage. It calls Sophia's
staged generic TTY wrapper with the prepared binary and arguments. The wrapper
owns the independent input guard, host preflight, bounded Session watchdog and
console restoration. The runner clears inherited display/bus/proof overrides,
does not manage services, and refuses a mismatched console or missing arm.
The wrapper's exit must be zero and its TTY/keyboard restoration must be proved.
Each stage's proof evidence must also pass before the next Session starts;
a clean process exit alone cannot authorize a later apply stage.
Keep the prepared console in the foreground through all four stages, including
the verification gaps between them. The runner checks the kernel's active VT
before launch, records it in `foreground-console.json`, and reports progress
after each passed stage. A terminal fd can remain attached to tty4 while tty2
is foreground; that does not grant tty4 input-device access. This check does
not prevent a later operator VT switch while Session is starting or running.
The existing wrapper/helper programs are from the sealed Sophia source tree;
host programs such as Bash, Python and the kernel remain host dependencies.

Each stage uses a private `XDG_STATE_HOME` under its evidence directory and leaves
daily diagnostic capture unset. Proof/runtime flags prevent automatic daily
capture, so Session start and completion retain their display identity in the
mixed proof log. `NO_COLOR=1` keeps tracing records free of ANSI escapes. The
wrapper's recovery and guard logs remain under `state/sophia/output-file-native-session/`;
the runner moves the mixed transcript to the canonical `NAME/session.log` after
clean wrapper exit and verified console restoration.

Four separate Session runs end normally at their runtime bound. For peer death,
the proof must pass before that shutdown: the outer runtime interrupt is never
restoration evidence. A timeout, log overflow or failed stage stops the sequence
and retains all logs and attempted arguments. A wrapper cleanup failure requires
operator recovery; the runner cannot infer native restoration from process exit.
Successful execution writes `run.manifest` and runs the offline verifier.

## Verify retained evidence

```sh
cargo xtask output-file-native verify \
  --inputs=/ABS/SEALED --inputs-manifest-sha256=SHA256 \
  --preparation=/ABS/PEER --preparation-sha256=SHA256 \
  --run=/ABS/RUN --run-manifest-sha256=SHA256
```

Verification only reads files. It binds source, SDK, peer/harness hashes,
profile, binaries, exact arguments and stage logs. Each stage needs one Session
start and one bounded completion on its declared private display, with native
presentation and physical input enabled and no native work left in flight or
awaiting cleanup. Peer death requires the
captured child's signalled exit, matching disconnect, local RolledBack
settlement, and both KMS and native-owner state restored to their before values.
Neither unchanged publication nor a peer exit alone is sufficient. Every
readback set needs complete row counts, and a mode timing must actually change.

The runner attests execution of the recorded paths. The verifier checks that
attestation against sealed inputs and logs; it cannot prove execution from
hashes alone. Session and child stdout share the wrapper's explicitly untrusted
log: the reader cannot distinguish a Session record from an identical line
printed by a child. This gate trusts the hash-bound frozen peer and signed,
pinned Hagia; it is not a verifier for arbitrary hostile children. Separating
per-role logs would strengthen that boundary without changing the wire.
These records do not prove rendered pixels, global desktop origins
or the primary output in hardware. Origin/primary intent is bound by the declared
layout and peer snapshot checks. Head disabling, routing changes and unselected
object leaks remain outside this gate. Promotion and archiving are separate.

## Candidate preparation (2026-09-30)

The `t253-native-run-5af9a03-01/` attempt passed validate, then stopped during
reject startup with `MissingKeyboard`, before the peer submitted a candidate.
Console and keyboard recovery passed for both stages. The retained elogind
state places tty2's activation between stages; tty4 was inactive when reject
started. The direct emergency guard found keyboards, but Session's seat-bound
input did not. The runner now refuses an inactive planned console before
launch and explains that the operator must remain there through all four stages.
The failed attempt is retained; commit-restore and peer-death did not run.

The first four-stage attempt (`t253-native-run-2808fcc-01/`) stopped after
validate because the integration verifier counted startup's apply and commit
as effects of ValidateOnly. The peer received Validated at epoch 2; Session
completed normally with 15 nonzero application exports, and console recovery
passed. Reject, commit-restore and peer-death did not run. The failed run and
its original verdict remain intact.

The corrected verifier binds startup's apply, presentation, settlement,
publication and commit before baseline capture. Snapshot publication IDs use
Session's separate counter: they are joined by topology epoch and owner-record
order, not client transaction ID. A byte-preserved extract of the attended
validate records is an offline regression fixture. Other regressions refuse
extra effects, missing startup evidence and stray publications, and allow peer
records to overtake owner logging after a transport command is enqueued.
The peer-death rollback check currently qualifies one DRM card; its exactly-one
rollback record does not establish multi-card recovery.

The first attended inventory (`t253-inventory-a503c31-01/`) recorded the
startup commit at topology epoch 2, DP-1 as head/output 1 with mode 260 at
2560×1440/120 Hz, and DP-2 as head/output 2 with mode 513 at 1920×1080/60 Hz.
DP-1 mode 257 supplies the same-size 60 Hz alternative. Complete KMS and owner
baseline rows matched the peer-exit rows. Console and keyboard recovery passed,
but Session exited 1: the empty desktop had zero nonzero application exports.
This is inventory evidence, not native acceptance. The visible startup client
above corrects the workload; the completion check remains unchanged. Later
stages must recheck the exact baseline before submitting any change.

An earlier preflight failed before hardware access: Git 2.55.0 crashed while
hashing a tree containing `.gitattributes` outside a repository. Tree identity
hashing now uses `git hash-object --literally` to avoid repository-dependent
fsck; identities still bind the exact walked bytes and modes. Regression checks
cover special entries outside a repository. Rebuild the verifier and prepare
new sealed inputs at the integration fix before using the acceptance runner.

Signed Sophia `170d606b6b3a398e18db5f52a85e4263ecbde54f` was the first T253 proof
candidate, with C SDK `7ccfece173b4b01e563a27b8fe5cc07d4369b55a` (0.3.0).
The exact release build passed all 13 real-export peer tests and the protected
Session fixture's four stages. Preparation is retained under the operator's
`development-evidence/ipc-retirement/t253-native-170d606b6-01/`; its
`prepared.json` digest is
`2b6ef35e518126bf7891d504553022991827e5c3b1d8a862d7352f530a5fe736`.
These tests supply physical observations and claim no hardware acceptance.

Niltempus's updated-pin checks, provisioning and source audit passed. Its
workspace ran 225 tests with zero failures and 10 existing ignored tests;
strict clippy and formatting passed. After the one-job builder adjustment,
the affected builder tests and strict clippy passed again. Sophia's profile
checker accepted the reference baseline as schema 1 and resolved exactly the
two declared outputs. Logs are retained in `t253-niltempus-170d606b6-01/`.

The single performance run at `t253-perf-170d606b6-01/` passed on the same
revision. Small/maximum topology connection p99 was 30.745/30.918 ms;
proposal p99 was 7.467/7.492 ms. All samples met the declared maxima, each
fixture delivered all 1,020 proposals exactly once including warm-ups, and
idle CPU was 0.215–0.231% of one core across six ten-second intervals.
Every worker joined on cleanup. The worker still wakes about 950 times per
second; this result meets the existing CPU gate and does not claim event-driven
idle behavior. Raw samples and machine/toolchain identities remain in that
directory.

The pin update does not repin Hagia or Bemenu. Hagia's artifact path verifies
its own explicitly named vendored SDK, so it can participate in this proof.
The existing Bemenu artifact and live-gate paths require equality with the
integration SDK pin and refuse its older SDK snapshot. Updating that product
or qualifying a launcher-specific compatibility bridge is separate work;
this output gate has no Bemenu dependency. The installed release and recovery
baseline remain unchanged. T253 stays open until attended native acceptance,
and T272 source retirement remains gated on it.

### Installed-mode rollback correction

The `t253-native-run-4106be4-01/` attended attempt passed validate and reject.
Commit-restore failed before the first peer candidate reached KMS:
`resource_preparation_rejected error=PublishedSnapshotMismatch kms_submits=0`.
Startup had installed 120 Hz, while rollback preparation still read the card
session's construction-time 60 Hz capability. Baseline and peer-exit readback
matched; all three stages recorded the correct foreground console and clean
recovery. Peer-death did not run. The verifier correctly refused this attempt.

Sophia `d4b06de8165559ca9ea91e15806692a44bcd74db` replaces that candidate.
Rollback timing now comes from the installed head selection, including the full
modeline. Discovery ordering remains fixed because public mode IDs depend on it.
This revision also includes the requested caller-priority and parallel build
defaults. SDK 0.3.0, all eleven contracts and the declared layouts are unchanged.

The replacement's `t253-native-d4b06de81-01/` preparation passed the thirteen
real-export tests and protected Session fixture at nice 0 with 32 build jobs.
Its `prepared.json` digest is
`b4d82db37f4377e8fec94fe558f3b863dffd7b9ac80baf28a169a926f75793a9`.
These supplied-owner tests still claim no KMS acceptance. A fresh sealed bundle
and all four attended stages are required for the replacement candidate.

The exact replacement also passed `t253-perf-d4b06de81-01/` at nice 0:
small/maximum connection p99 30.682/30.929 ms, proposal p99 7.487/7.502 ms,
and six idle intervals at 0.206–0.210% of one core. Counts, exactly-once
delivery and cleanup passed the unchanged gate. No compiler ran during the
measurement; the normal-priority build used 32 jobs.

### Singleton framebuffer custody correction

The `t253-native-run-e5dbf36-01/` attended attempt passed validate and reject.
Commit-restore passed the corrected rollback timing projection and prepared
both candidate images, then exhausted renderer slots while preparing rollback.
The peer timed out after 30 seconds. No candidate KMS submission occurred;
readback remained at the baseline and all three console recoveries were clean.
Peer-death did not run. This was a production failure, not a verifier failure.

Sophia `e51a17b8681123a53ca3f7b632c35dbe71e3a4ee` transfers the displayed
topology buffer into ordinary singleton runtime custody during rebind. The
first accepted frame can then release that predecessor. Rebind also explicitly
retires old runtime buffers through DRM and retains failed cleanup. Mirror
heads keep their existing custody. Candidate and rollback preparation share a
five-second limit; retries and the quarantined owner loop are paced instead of
spinning until the client disappears.

The backend's isolated library and DRM-resource tests passed (209 and 305);
Session's output-file group passed 14 tests with three opt-in tests ignored.
Transfer and deadline negative controls failed as expected. Strict backend and
Session Clippy passed for all targets and features. These checks supply cleanup
and timing observations and are not physical acceptance. The new candidate
requires fresh signed preparation, performance evidence and sealed inputs;
the previous attempts and their logs remain intact.

Before an attended run of that revision, a final audit found that rollback can
replace a topology buffer before any ordinary frame has supplied a native
frame identity. Sophia `344e9534b32e176f2ffb4cea8694269758ad57f0` assigns each
singleton runtime a separate native owner/head cleanup scope. Both semantic
startup and topology adoption use it; ordinary frame identities, when present,
must agree. The new regression hands off a candidate and then its rollback
without an intervening ordinary frame. Requiring the old identity makes it
fail. Backend checks pass 210 library and 305 feature tests, Session's
output-file group passes 14 with three opt-in tests ignored, and strict
backend/Session Clippy passes. The e51 preparation and performance evidence
remain retained as superseded; no attended run used that candidate.

### Rollback waits for candidate presentation retirement

The `t253-native-run-4f4e5ba-01/` attended attempt passed validate, rejection
and commit-restore on Sophia `344e9534b`. Peer-death reached accepted reverse
KMS programming but failed runtime rebind with two candidate first frames still
in flight. Installation had replaced their completion trackers before their
owners retired. There was no restored readback, local RolledBack settlement or
peer-loss pass. Console recovery was clean; the run remains failed.

Sophia `ddd27bd6d9ac6d8e73394d9326705a7a62916f35` drains unsubmitted work and
retires submitted candidate frames before reverse programming. Normal operation
and shutdown use the same runtime step, with a paced, two-second Session wait.
The displayed buffer remains owned until blocking restoration and the existing
rebind handoff. No topology first-presentation acceptance runs during the drain.

The backend checks passed 211 library, 305 feature and three presentation-skip
tests; Session output checks passed 90 with three ignored. Seven helper tests
and two failing mutant controls cover the wait, deadline and readiness latch.
Workspace strict Clippy passes. The full repository gate encountered an
unchanged shell-fixture race (`--serve` passed to the test binary), retained in
`t253-rollback-drain-01/06-xtask-check-runtime.log`; it is not claimed green.
The composed DRM/renderer drain still needs attended proof. SDK, contracts,
layouts and profile are unchanged; new preparation, performance and sealed
inputs must bind this revision before another run. See Sophia investigation
`0bc9j2k2` for the failure and validation limits.

### Accepted t253 proof and t272 retirement candidate

The four attended stages subsequently passed on Sophia `ddd27bd6d9`,
integration `bec6db1`, Hagia `b36af295` and SDK 0.3.0. The retained
`t253-native-run-bec6db1-01/run` manifest has digest
`3caee597b97ba77c4e6a87f26c5359abc3000310e82843c3147e53ff1f425b3d`.
Peer death reached observed restoration, local RolledBack settlement and the
typed proof verdict; all four stages exited zero with clean console recovery.
That evidence closes t253 within the one-card, two-head, refresh-only scope.

The t272 candidate is Sophia `ee3819e1685bc926f888c3cc8fe55904d6c21239`,
with SDK 0.4.0 `497e7e01531415078a4a3da2455ebe82ec18fd0e` and Hagia
`0de7ef229146e6abedff98d726f0afb0815bb218`. It removes the output socket,
keeps native profile authority without an output client, and requires the WM
API to name `output_transport=9p2000.L`. SDK and Hagia both test refusal of the
retired `current_ipc` API. The final Sophia follow-up repairs asynchronous EOF
and launch preparation in test fixtures; it changes no production contract.
Fresh preparation, performance and attended evidence must bind this assembled
candidate before t272 acceptance. The t253 evidence remains bound to its own
source pair.

Rollback must restore the entire previous pair: pre-retirement Sophia with
Hagia `b36af295` and its older SDK. New Hagia refuses the old API, and old Hagia
refuses the new one. The historical frame-fed runner refuses Sophia sources
without its removed startup rollback hook; archived evidence stays verifiable
for its original pins. Current native acceptance covers peer-transaction
rollback, with no physical startup-transaction rollback claim.
