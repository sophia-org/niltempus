# Independent component updates

The installer supports Hagia, Lom, Bemenu and the lock provider kleis
independently of the desktop release. After one login with a component-update-enabled release:

```sh
niltempus reload lom
niltempus reload bemenu
niltempus reload hagia
```

Each command resolves only that component's configured local source ref, requires
its trusted signed commit, builds offline in private build paths, verifies the
result and selects it. Lom uses an incremental Cargo target; Bemenu gets a fresh
strict C build. Hagia uses the installed product builder with its reviewed Nim
dependency manifest and staged toolchain. The other products and Sophia are not
built. There is no implicit source pull or dependency fetch.

For an already selected binary, skip the build:

```sh
niltempus restart lom
```

`prepare-component NAME` builds and selects without signalling a process.
`rollback-component NAME` selects the previous component and restarts it.
`prepare-hagia` and `reload-hagia` are aliases for the common workflow.
A build failure leaves the active selection alone. Both versions remain on disk
when restart fails; the command reports the failure without retrying or escalating.

## Selection and compatibility

The sealed desktop remains the base for Sophia, launch policy, configuration,
GPU permissions and transport. Its packaged executables provide initial
component versions. Installation initializes absent component selections and
preserves existing ones. At login, the plan-bound installer renders a private
runtime profile that substitutes the selected executable paths. Roles, 9P
transports, configuration paths and resource policies remain unchanged.

Shell selections live under the installer's user state in
`components/NAME/current`, with retained files under `versions/SHA256` and a
current/previous identity record. Hagia keeps its existing
`development/hagia` path and metadata. Publication replaces a regular executable
atomically, so a running process retains its old inode. Unrecorded executables,
changed digests, symlinks and cross-component rollback records are refused.
A crash between the executable and selection-record publications refuses further
use instead of guessing; `pending.json` and retained binaries preserve recovery
information. Do not edit managed executables directly.

Hagia and Bemenu must vendor the same C SDK snapshot as the installed desktop.
The installed helper verifies the complete snapshot and raw commit binding.
A contract change needs a qualified desktop release. A new Hagia source revision
also needs its correctly bound reviewed dependency manifest in configuration.
Signing or compilation alone does not prove compatibility with arbitrary future
protocol changes.

## The lock provider (kleis)

kleis, Sophia's lock provider, is a component only: no release builds or
seeds it, and a profile without a `session { lock-provider { ... } }` block
never needs it. `prepare-component kleis` builds it like Hagia, through the
installed product builder with its reviewed Nim dependency manifest
(`inputs.kleis_nim_deps` in configuration, with its independently supplied
sha256). Like Hagia and Bemenu it must vendor the installed desktop's C SDK
snapshot, so it waits for a desktop whose SDK carries the lock client.
Its optional repository entry is not a packaged desktop source. Initial
preparation requires the selection, current executable and pending publication
record all to be absent; incomplete existing state requires recovery.
Validation runs the installed Sophia's session-profile preflight with the
candidate as the profile's lock provider under the selected Hagia; a Sophia
without the lock provider role refuses it there.

At login the runtime profile points an existing `lock-provider` block's
`executable` at `components/kleis/current`, keeping its `config` and `gpu`
settings, and refuses a login profile that names a lock provider while no
kleis is selected: Sophia starts its provider once per session, so a missing
executable would leave the session without one. Sophia supervises the
provider and starts it again whenever it exits. The updater therefore only
selects: `prepare-component kleis` and `rollback-component kleis` change the
selection, and Sophia runs it at the provider's next start; `reload` and
`restart` refuse kleis.

## Restart ownership

For shell components, the installer finds exactly one owned process at the
managed executable path, with the exact `--serve` argv, one 9P socket variable
and the installed Sophia binary in its ancestry. It opens a pidfd and checks the
identity again before sending one signal. Ordinarily this is SIGTERM. A
namespace PID 1 without a SIGTERM handler ignores that signal, so the updater
uses SIGKILL for that exact process and reports why. The namespace identity and
signal disposition are checked again before signalling. There is no process-name
kill, PID-reuse race, process-group signal, sudo or timed escalation. Sophia's
existing supervisor retires the old connection and resources and respawns the
component. The command observes the replacement PID, binary hash, endpoint and
supervisor. This confirms process replacement, not negotiation, a rendered
frame or visual acceptance.

Hagia uses the existing `session restart-wm` control command and its checkpoint
and first-commit confirmation. The updater does not restart Sophia or its
application clients. The WM control transport is unchanged by this work.

## Migration and verification

Existing sessions retain their sealed executable paths. They cannot adopt new
launch paths by changing an environment variable. Install a release built with
`component_updates=true`, then log in once. Subsequent component updates require
no full-desktop build, sudo or logout. Desktop rollback and component rollback
are separate: desktop install/rollback preserves explicit component selections.

Tests cover atomic publication, previous-version retention, neighbour isolation,
stale requests, malformed metadata, changed hashes, private login-profile
rendering and real CLI installation in private mounts. The signal test uses
actual pidfds and private supervised fixture processes, including a Bubblewrap
PID/user namespace and a native peer with no SIGTERM handler. The latter first
proves that SIGTERM leaves namespace init running, then verifies replacement
with the selected signal. This is not a live Lom or Bemenu rendering test. Existing WM
restart tests retain their private CLI fixture. Live desktop acceptance remains
separate. No running desktop is changed by these tests.

Implementation verification: 62 top-level Go tests passed, including the two
private-installation tests; Go vet passed. The focused Rust SDK-verification and
installed-helper suites passed 9 tests, and xtask clippy passed with warnings
denied. Logs are under `development-evidence/component-updates/`. A follow-up
Go test run also passed after version publication was changed to publish a
complete temporary file without overwriting an existing version.

The subsequent [release verification](component-update-release.md) exercised
the actual release and the component-build command for all three products.
Live desktop acceptance remains separate.
