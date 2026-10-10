# Offline WM lifecycle qualification

`tools/wm_lifecycle.rs` runs the current SDK-based Hagia lifecycle assertions
from Hagia's `tests/external/lifecycle.rs`. It is separate from the frozen
pre-retirement pairing runner and from installed-session rehearsal.

Compile and check the dependency-free Rust runner:

```sh
rustc +1.96.1 --edition 2024 --test tools/wm_lifecycle.rs -o /tmp/wm-lifecycle-tests
/tmp/wm-lifecycle-tests
rustc +1.96.1 --edition 2024 tools/wm_lifecycle.rs -o /tmp/wm-lifecycle
```

Run with five explicit absolute paths:

```sh
/tmp/wm-lifecycle SOPHIA_CHECKOUT HAGIA_CHECKOUT NEW_WORK TARGET CARGO_HOME
```

The Sophia checkout must be clean at
`ea64b1f027ace8b9064935d966697acfac37d004`. Updating this pin requires reviewing
the test mount and owner APIs, not silently following master. The supplied
Hagia checkout supplies the external fixture and the exact committed ordinary
Hagia source; dirty production inputs are refused. The runner records remaining
working-tree changes, so development evidence stays distinguishable from a
clean committed fixture run.

`NEW_WORK` must not exist. `TARGET` and a provisioned offline `CARGO_HOME` must
already exist and be reserved for this work. Reuse the target across development
and controls; do not create a large fresh cache for every trial. Hagia builds
serially, Rust uses four jobs, tests run serially, and both use nice 10.

The runner archives the sources, inserts exactly one test module into the
Sophia archive, and records that overlay. It builds ordinary Hagia with its
vendored SDK. Builds and tests run under bubblewrap with network, host devices,
host PID visibility and user-runtime sockets hidden. The host filesystem is
read-only except the work, target and Cargo cache directories. Every build/test
has an outer timeout with five-second kill escalation. This is a development
build, not the release dependency/toolchain gate.

Eight tests must each appear exactly once in the ignored-test listing and then
each report one passed test. Missing tests, zero tests, a failing command, or
changed fixture/binary bytes refuse the run. `RESULT` and the self-excluding
`SHA256SUMS` close the records on success or failure. The manifest covers the
top-level records, binary and case artifacts; archived source/build caches are
not evidence-manifest entries. Source commits and the overlay hashes identify
the test source. A failed run is kept and its path is never reused.

The assertions cover protected startup, real profile rejection/rollback, and
automatic/requested restart with checkpoint refresh on an empty headless
scene, plus occupied settlement and restart in `tests/external/occupied.rs`.
All three external module hashes are bound. Occupied cases supply authority facts
and layout completion, and compare committed policy focus and checkpoint state;
they do not prove visual readiness or physical focus. See Hagia's external-test
README for exact supplied and real boundaries.
`operations.rs` adds accepted/refused typed intent, refused-action continuation,
and disconnect with an unsettled projection or operation. No returned intent is
executed. The source-pin change from `9f52403be` is documentation only; the mount
and production owner code are unchanged.
No installed process, GPU, input device, display server or release is accessed.
This is neither a performance result nor full t249/h006 acceptance.

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
