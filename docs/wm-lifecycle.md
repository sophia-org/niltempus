# Offline WM lifecycle qualification

`tools/wm_lifecycle.rs` runs the current SDK-based Hagia lifecycle assertions
from Hagia's `tests/external/lifecycle.rs`. It is separate from the frozen
pre-retirement pairing runner and from installed-session rehearsal.

Compile and check the dependency-free Rust runner:

```sh
rustc +1.96.1 --edition 2024 --test tools/wm_lifecycle.rs -o /tmp/wm-lifecycle-tests
/tmp/wm-lifecycle-tests --test-threads=1
rustc +1.96.1 --edition 2024 tools/wm_lifecycle.rs -o /tmp/wm-lifecycle
```

Run the whole command under one cargo-slot lease, with five explicit absolute
paths:

```sh
cargo-slot sophia /tmp/wm-lifecycle SOPHIA_CHECKOUT HAGIA_CHECKOUT NEW_WORK BUILD_ROOT CARGO_HOME
```

The wrapper holds its slot lock until the runner exits and gives it the slot as
`CARGO_TARGET_DIR`. The runner refuses an unset target, a target that is not a
numbered child of the default `~/.cache/cargo-slots/sophia` pool, pool
overrides, and a parent process that is not `cargo-slot sophia`. It records the
wrapper path, hash and invocation in `resources`. That check is about paths
and the parent command; the lease itself is the wrapper's lock, which the
runner does not inspect. Never export a slot path by hand or nest cargo-slot
inside a leased run.

The Sophia checkout must be clean at
`a8c5fdf29c8932580b04b3de0f807327e0d0a34b`. Updating this pin requires reviewing
the test mount and owner APIs, not silently following master. The supplied
Hagia checkout supplies the external fixture and the exact committed ordinary
Hagia source; dirty production inputs are refused. The runner records remaining
working-tree changes, so development evidence stays distinguishable from a
clean committed fixture run.

`NEW_WORK` holds records only and must not exist. `BUILD_ROOT` holds build
products: the archived Sophia workspace, each run's Hagia source archive, Nim
cache and binary under `runs/<NEW_WORK name>`. Both are resolved through their
existing parents, so symlinks cannot hide an overlap. The runner refuses roots
that overlap in either direction, a build root overlapping the evidence tree,
and either root overlapping the cargo-slot pool, Cargo home, supplied checkouts
or runner executable. A build root cannot contain HOME or lie inside a Git
checkout. Existing build roots require the runner's owner marker, which records
the creator's protected paths for cleanup. Runs of one slice reuse one build
root: the Sophia archive is re-extracted at the same path, so cargo reuses
unchanged slot artifacts.
Evidence records the binary's path, hash and size, never the binary.

The runner holds a separate, exclusive nonblocking build-root lock from before
record creation through manifest closure. Its stable sibling path is
`.<BUILD_ROOT name>.wm-lifecycle.lock`; never unlink this file, even after
cleanup. A busy lock refuses the run before either root changes. Cleanup takes
the same lock before reading the marker or deleting anything. Controls and
checks that use the retained archive or binary must hold this lock too, for
their entire use of the root, for example with `flock --exclusive --nonblock
/absolute/parent/.build-name.wm-lifecycle.lock COMMAND ...`. Cargo commands
also need their own `cargo-slot sophia` lease. The build-root lock does not
replace the cargo-slot lock. `resources` records the build-root lock path,
device and inode.

Each build/test timeout supervisor inherits the build-root lease. If the runner
is killed while that step survives in its separate process group, the lock
remains held until the supervisor exits. The `git archive` and `tar` writers
also inherit the lease, so interrupted source preparation remains protected.
Read-only source and hashing helpers do not inherit it. Bubblewrap bounds the
build/test process tree; this mechanism does not extend the separate cargo-slot
wrapper's lifetime.

An externally locked command may inspect, mutate or test the retained archive;
it must not call the runner or its cleanup command, which acquire the lock
themselves and would correctly refuse a second acquisition. Invoke the runner
directly under cargo-slot, and invoke cleanup directly after external controls
release their lease.

On closure the runner checks the record set: regular files at the top level and
in `cases/<case>/`, with no symlinks, other directories, executables, ELF or
archive objects. A violation or inspection error turns the result into STOP;
`record-policy` preserves the findings alongside any original run failure.
Unhashable files also force STOP, with a manifest of the readable records.
Size is not a criterion; large logs are records.

After a slice's runs, controls and checks have all ended, remove its build root
with `wm-lifecycle cleanup BUILD_ROOT NEW_CLEANUP_RECORD`. It removes only that
canonical, marked root, refuses while a process of the user still has its
working directory or executable there, and records what it removed in its own
closed record. Cargo slots and evidence are never removed. A provisioned offline
`CARGO_HOME` must already exist. Hagia builds serially, Rust uses four jobs,
tests run serially, and both use nice 10.

The runner archives the sources, inserts exactly one test module into the
Sophia archive, and records that overlay. It builds ordinary Hagia with its
vendored SDK. Builds and tests run under bubblewrap with network, host devices,
host PID visibility and user-runtime sockets hidden. The host filesystem is
read-only except the work, build, slot and Cargo cache directories. Every build/test
has an outer timeout with five-second kill escalation. This is a development
build, not the release dependency/toolchain gate.

Nineteen tests must each appear exactly once in the ignored-test listing and then
each report one passed test. Missing tests, zero tests, a failing command, or
changed fixture/binary bytes refuse the run. `RESULT` and the self-excluding
`SHA256SUMS` close the records on success or failure. The manifest covers the
top-level records and case records, including binary paths and hashes; binaries
and archived source/build caches stay outside evidence. Source commits and the
overlay hashes identify the test source. A failed run is kept and its path is
never reused.

The assertions cover protected startup, real profile rejection/rollback, and
automatic/requested restart with checkpoint refresh on an empty headless
scene, plus occupied settlement and restart in `tests/external/occupied.rs`.
All fourteen external module hashes are bound. Occupied cases supply authority facts
and layout completion, and compare committed policy focus and checkpoint state;
they do not prove visual readiness or physical focus. See Hagia's external-test
README for exact supplied and real boundaries.
`reused_identity.rs` adds t323's recovery join: a real supervised restart,
replacement of a surface by the same numeric index with a new generation,
synchronous refusal of the old focus identity, and a correlated fresh-focus
answer from Hagia. Surface facts and layout completion are supplied; this is
not physical input or Manage admission. The index-only guard control must fail
at the old-identity admission assertion, before any later cause filtering.
The pin update from `8ce7c40ef` changes only documentation, test fixtures and
test-only module wiring. Production WM owners and all five bound GLUE spans
are unchanged; the t323 credit slice's isolated repository gate passes at this
source. The clean signed pair (Sophia `a8c5fdf29`, Hagia `b4d5702`, runner
`6592846`) passes all nineteen cases in `t323-identity-01`. The archived
index-only guard control in `t323-identity-control-01` fails first at
`old surface identity must not target its numeric replacement`, with exit 101
and one failed test. Source restoration, all fourteen modules and the Hagia
binary hash verify; strict overlay clippy passes in `t323-identity-checks-01`.
These closed records are under the development-evidence root above. They do
not qualify Manage admission, physical focus or the remaining t249 exits.
`operations.rs` adds accepted/refused typed intent, refused-action continuation,
and disconnect with an unsettled projection or operation. No returned intent is
executed. `slow_peer.rs` pauses only the fixture's own protected Hagia process
and requires Session's unmodified twelve-second response deadline to replace
it while the protected leader is still running; its detection and replacement
times are recorded observations, not latency qualification. The source pin
`65440ff5d` differs from `ea64b1f02` only in documentation: no crate, tool or
Cargo file changed, and the mount file is byte-identical.
`presentation.rs` selects Session's native presentation ceiling without a
device and commits three owned CPU buffers through one ordinary CPU production
cycle, bypassing Session admission and visual readiness for those pixels. Real
Hagia's held recent-windows publication must pass unmodified preflight, and a
supplied completion, captured key and chord release must commit the selection
expected beforehand from the checkpoint. Heads, completions and router time are
supplied; no native retirement, pixels, physical input or latency is claimed.
Its copied dispatch arms and routing call mirror four pinned production spans,
located by semantic anchors and hashed; a changed span refuses the run, and the
glue must then be compared again by review. The source pin `f77244abe` differs
from `65440ff5d` only in documentation, with the same mount file.
Its child `presentation_revocation.rs` reuses the prefix through Held without
a captured key or release debt. A completed frame missing its publication
stamp must revoke the original identity. The first exact SceneChanged answer
must close the switcher and preserve focus before a Withdrawn frame is supplied;
Ended must return chord credit without selecting another window. A refused
answer is distinguished from transport failure, but this staged-projection
rejection exposes no refusal-reason record. Its precise cause remains a source
inference. The control omits only Revoked delivery in the archived Session and
must fail a named assertion on that first answer.
The child `presentation_disconnect.rs` retains a captured Right release debt
across automatic replacement with an unsettled action. It checks debt before
recovery, rejects the old action identity, distinguishes an owed release from
an unowed duplicate using the no-focus routing counter, then commits a fresh
chord in the new epoch. Its control clears capture state at restart. This
excludes pending layout settlement, application delivery and pointer debt.
The child `presentation_cancel.rs` supplies a keyboard-matching transition to
Session's cancellation owner. It correlates the exact chord and Cancelled cause,
requires unchanged focus before withdrawal receipts, and checks a fresh chord
after cancellation. The control rebuilds an archived Hagia whose terminal
handler treats cancellation as release. Actual locking, other cancellation
triggers and AbortSettlement remain separate.
No installed process, GPU, input device, display server or release is accessed.
`abort_settlement.rs` stages a real fullscreen proposal pending frontend state
acknowledgements, with a supplied long deadline and locally retained controls.
Peer exit must force timeout settlement in the old epoch before replacement;
controls omit forced expiry or bypass that barrier. Frontend delivery, rollback
ACKs and native settlement remain outside the fixture.
`manage.rs` introduces a new surface through a supplied authority presentation
intent and the production Manage queue, staging and admission owners. It
checks queued/in-flight deduplication, exact supplied admission/state ACKs,
and settlement that ends the owner's management request. The surface remains
AwaitingPixels: no frontend delivery, pixels or visibility is claimed. Its
control replaces Manage with Relayout and must fail the named admission-request
completion assertion, not an enum comparison.
The two supplied completion-dispatch arms are reviewed against a fifth
hash-bound production span in `owner_loop/session_control.rs`.
`manage_pixels.rs` supplies a CPU backing snapshot with the new-window intent.
It checks safe-extent reconciliation, staging of that retained candidate,
admission through the same supplied ACKs, and release to the production CPU
runtime. Both cycles check exact retained bytes and an interior pixel of the
headless composed frame. A second frame must reach production without admission quarantine.
The control omits the CPU branch's transition to Managed and must fail that
second-frame assertion. A second control omits the safe recovery extent and
must fail the reconciled-size assertion. This covers backing snapshots; Present admission and
native retirement, backend presentation receipts and all-head consensus remain
separate t249 joins.
This is neither a performance result nor full t249/h006 acceptance.

`presentation_consensus.rs` uses the existing backend
`SessionPolicyPresentationFixture`: one output with two mirrored heads, real
retained composition and publication stamps, with worker/device completions
simulated per head. Session must emit Presented only after both heads complete,
and Withdrawn only after both retire the replacement without the switcher.
Repeated publication must not duplicate either receipt. Separate controls make
the backend ignore the second head during presentation or only during withdrawal.
The Session frame-service calls retain the existing hash-bound ordering; the
concrete native-selection tail of `service_presented_policy` remains a distinct
t249 coverage gap. No device completion or physical display is claimed.

`presentation_repaint.rs` checks a source-only update under an unchanged
presentation. Both mirror heads must retire preview instances naming the new
committed source generation while Session keeps the original Presented identity
and starts no WM cycle. Retained CPU bytes are checked separately. The Sophia
pin adds only a read-only retired-instance accessor to the existing generic
backend test fixture; production sources and all five GLUE spans are unchanged
from `f77244abe`. A control clamps the captured preview source generation to 1
and requires the production composition-plan guard's `StaleInstanceSource`
refusal at the repaint queue. A separately labelled compound control also
bypasses that guard to test the retired-instance readback assertion.
Recomposition is explicitly requested by the fixture, and device completions
are simulated: production repaint scheduling and preview pixels remain separate.

The first recorded run, `t249-sdk-lifecycle-01` under Sophia's development
evidence directory, passes all three cases with Hagia `870bfc68b` and runner
`dbda3009b`. Its manifest hash is
`b5ac4542073194c5ba27200a292862eb77489624236549f2b40add43c9df23c6`.
`t249-sdk-lifecycle-controls-02` binds the final test binary and a separately
built no-checkpoint-restore mutant; the restart case fails as intended when the
client no longer sends its restored-state refresh. The empty-scene limit still
applies. Production sources, release pins and installed processes are unchanged.

The occupied successor is `t249-occupied-01`: five cases pass on Hagia
`0bbfe3cee3d1efa0b191098c70e64062ab8073b0`, Sophia `9f52403be` and runner
`83023216817500f7595d5550f76d6b28016c99ba`. Manifest SHA256 is
`7a340bac1e5b4dfc9cf4458162b6c5ab1fe4077afd8f1d31d5067dc0ce2946b7`.
`t249-occupied-controls-01` binds that final fixture/executable: no checkpoint
restore fails the occupied-layout assertion, and promoting/saving a refused
candidate fails checkpoint preservation (one named failure each). Manifest:
`6dcd65ec09d3bfffd2ef3c7cd9106eb0d7832bbfc8d2a13d0705180cc55eea60`.
`t249-occupied-checks-02` records overlay/runner clippy, formatting and two
runner controls; manifest
`2855f118dd35575247f03425285c8fe1dfc1bca1395b79c973ac9a685beec5cb`.
The earlier failed development fixtures are preserved, not qualification runs.
No release pin or installed process changed.

The operation successor `t249-operations-01` passes all eight cases with Hagia
`67da6edfb2c5932b59f828b452ab9bf4e17236fe`, Sophia `ea64b1f02` and runner
`7eff07b728a41a7813bd29fb850999f222eb5d09`. Manifest:
`84e927141e8887cd9eb0ca66b7f317f9edf47aee0663110d5c139fca7dc83189`.
`t249-operations-controls-01` compiles two server mutants: accepting a refused
operation and retaining disconnected operation authority. Both fail the named
assertion; manifest
`b597e4e71aa96e00019713d12af0e50f6a92d6c3fb09ab5b1df409f49be6a78f`.
The development and final fixtures/overlay hashes match, as do the production
Hagia Git objects. No control invokes an executor.

`t249-operations-checks-01` passes overlay clippy, then stops when Nimble cannot
write metadata through the read-only sandbox, after layout itself passes.
`t249-operations-checks-02` uses the direct layout script and passes it, runner
clippy, two runner controls and formatting; manifest
`d822a3c6639d8e92d32d479c1e5b25727fa7a36d1164b492ef9144680889abd3`.
The preserved first check manifest is
`5c60c5999c026ed8fdd0f3cb55de9581ac5c1f84049b185ce452c71758a9e1e8`.
Slow-peer/credit exhaustion, presentation/input and measurement remain outside
this slice. No release or installed process changed.

The stalled-peer successor `t249-slow-peer-01` passes all nine cases with Hagia
`651763e2dff1102a0b7380b3347ce7fc19c44fea`, Sophia `65440ff5d` and runner
`e441c7fb6b8cd4738ebc3d34fbc51df48d664b85`. Manifest:
`537e570a65ce84623dbd00684f10ef304752a4bc1b25955a5cd7aabac961a390`.
The live stopped fixture peer triggers the actual twelve-second response
deadline. The new epoch clears pending authority, restores occupied state and
answers a fresh action. Observed detection/replacement times are 12051/29 ms;
they are not a latency qualification. `session-lines.txt` ends at detection,
before restart. No installed process is signalled.

`t249-slow-peer-controls-01` compiles ignored-transport-failure and
retained-stalled-request mutations; each fails its own named assertion.
Manifest `9172a63d50bb671fb8d3250e45898d7550122d261bc4e6c2eecf9935058ba4a3`.
`t249-slow-peer-checks-01` verifies both mutated files were restored and passes
strict overlay/runner clippy, two runner controls, formatting and Hagia layout;
manifest `f35280c03cd458794dce6b5124dd4cd4513419bed6d27faa145d66774a2db098`.
All manifests independently verify. AbortSettlement, peer-side ESTALE and
receipt credit remain outside this case; receipt exhaustion reachability needs
a separate review. No production change, new release or full product gate.

The presented-chord successor `t249-presented-01` passes all ten cases with
Hagia `3960c837`, Sophia `f77244abe` and runner `beb125b7`; manifest
`32e64d8b45db223c74f0a1a0d3f902070c666a8341177810d479bf1d6b11b623`.
`t249-presented-controls-01` has two compiled named failures (missing completion,
ignored Ended), manifest
`836e6638d74f264bb0b4ad91f169a5529821e6e2d48208939f312f1e02e4ff79`.
`t249-presented-checks-01` passes restored-source comparison, strict clippy,
three runner tests, fmt and layout; manifest
`a2ef23105d82f86fbb0ff03621b73655b1a0801bba485f0b9bd5593fca85fcfa`.
All manifests independently verify. The initial dev01 receipt-drain assertion
failure is preserved; bounded polling corrects fixture scheduling only. Final
fixture/overlay/glue/runner hashes equal the control-bound dev02 ones. No
bit-reproducibility claim follows from separate Nim builds of identical source.

The new global build policy applies to future executions: use `cargo-slot
sophia` to hold a pool slot for the whole runner and pass its assigned target;
never allocate a per-task target or copy built binaries into evidence. This
runner now separates build products from NEW_WORK and holds an independent
build-root lease through closure; controls and checks must take that same lease.
The frozen historical packages above remain intact. The resource change does
not change a release pin or close t249/h006.
