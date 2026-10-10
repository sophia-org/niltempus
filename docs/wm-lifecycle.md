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
`0f2ad2386baa063ab92d9567a145148f5232bccb`. Updating this pin requires reviewing
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

Three tests must each appear exactly once in the ignored-test listing and then
each report one passed test. Missing tests, zero tests, a failing command, or
changed fixture/binary bytes refuse the run. `RESULT` and the self-excluding
`SHA256SUMS` close the records on success or failure. The manifest covers the
top-level records, binary and case artifacts; archived source/build caches are
not evidence-manifest entries. Source commits and the overlay hashes identify
the test source. A failed run is kept and its path is never reused.

The assertions cover protected startup, real profile rejection/rollback, and
automatic/requested restart with checkpoint refresh on an **empty headless
scene**. See Hagia's external-test README for exact supplied and real boundaries.
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
