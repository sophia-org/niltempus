# Sophia niltempus Desktop

Personal Go installer and release assembler, maintained through chezmoi.
Sophia owns the compositor and session mechanisms; Hagia owns WM policy; Lom
and Bemenu provide this desktop's panel and launcher. Narthex is packaged for
Sophia's existing shell support. Desktop composition belongs here, outside
those repositories.

## Commands

For the next session, run this as your normal user:

```sh
~/sophia-niltempus-desktop install
```

It resolves the configured local refs, builds and validates the stack (or
verifies an already built matching release), then prompts for sudo to install
it. Select **Sophia niltempus Desktop** at your next login. A failed build or
validation stops before installation.

The normal entry runs Hagia and Bemenu over 9P2000.L. Lom remains on current
IPC until its SDK adoption. **Sophia niltempus Desktop (current IPC)** runs
the same binaries with both WM and shell roles on IPC. The release contains
separate, preflighted `desktop.kdl` and `desktop-ipc.kdl` profiles; the source
profile stays unchanged. Log out and select the IPC entry to change wires.
Installation removes the older **Sophia niltempus Desktop (9P WM)** entry.
New releases use manifest schema 2, which requires the IPC profile and entry.
Schema 1 releases remain verifiable for rollback, including releases predating
the IPC entry; their original files and checksums remain unchanged.

The individual steps remain available:

```sh
~/sophia-niltempus-desktop plan
~/sophia-niltempus-desktop build
~/sophia-niltempus-desktop verify /path/printed/by/build
~/sophia-niltempus-desktop install /path/printed/by/build
~/sophia-niltempus-desktop status
~/sophia-niltempus-desktop rollback
```

`plan` is the default and reads local refs only. It never fetches, pulls,
checks out a branch in a working repository, or takes uncommitted edits.
Update each repository's local `master` deliberately before planning if newer
sources are wanted. Change a `reference` to an exact SHA to hold that component.
Each plan records the resolved commits and Git signature status (`G`: valid
trusted signature; `N`: unsigned). Present signatures must verify. Unsigned
commits remain visible as such; currently Lom master has an unsigned docs tip.
The external desktop packager requires trusted signatures for its own source,
Sophia, Hagia and Narthex.

`build` checks out those exact commits in private clones. The signed
`sophia-desktop-integration` tool prepares the WM pair and packages Sophia;
the personal assembler adds Lom, Bemenu and the two login profiles. Changed or
unexpected cached inputs are refused, never reset over. Bemenu uses a fresh checkout because Make
does not track its embedded commit's compiler flags. Every attempt records the
source paths, plan, stage logs and elapsed times; cached checkouts may advance on
the next build, but these records and sealed releases remain.

WM-pair builds run serially and build only the packaged executables;
Cargo uses two jobs. Git checkout and compiler children use umask
022, independently of the caller, so a permissive shell cannot create a profile
that Hagia rejects as group-writable. The original repositories and profile are
never chmodded. The WM pair binds Hagia's shipped profile by its digest, and
the assembled release's profiles are checked before sealing.
Build and profile-check commands run without network access at reduced CPU
priority under Bubblewrap with DRM
devices, display sockets, runtime sockets and inherited Sophia variables hidden.
The repositories' own build dependencies must be available; Cargo builds are
offline. Build does not install, switch a release, start a session or probe GPUs.

The explicit `integration` repository entry selects the external packaging
tool. Its committed Sophia pin must equal the selected Sophia commit. Provision
that integration revision explicitly before planning; its `.provision/accepted`
must match the committed lockfile and a private Cargo home outside the source
trees. The plan records that home, lockfile digest and signed tool commit.
Build revalidates the acceptance and runs the tool's pin/provision checks;
it never downloads dependencies or provisions a cache implicitly.

Packaging and activation scripts come from that integration revision.
The outer manifest records the five product commits, the integration binding,
the source profile hash, the
installer executable hash, and every packaged file's hash and permissions.
Release IDs identify these inputs, not a claim of bit-for-bit reproducible
compiler output. A reused release is fully verified before being returned.

## Profile and login

Settings live in `~/.config/sophia-niltempus-desktop/config.json` (override with
`--config FILE` before the command). The default profile input is
`~/.config/sophia/desktop.kdl`; it is read, never edited.

Each release contains a generated copy with packaged Hagia, Lom and Bemenu paths
pinned to that exact release, `Super+o` bound to `policy:toggle-overview`, and
`session { control "host-admin"; }` enabled. A conflicting or duplicate binding
is refused, as is ambiguous or malformed control configuration. Other profile
settings remain in the copy,
including the external Lom config path and application choices. This personal
assembler expects one WM, one bar and one application launcher. Includes must
first be flattened with Sophia's `config print-effective` command.

KDL transformations use the pinned [kdl-go parser](https://github.com/calico32/kdl-go),
followed by the newly built Sophia `config check-session-profile` preflight
against the newly built Hagia. Parser success alone is not profile acceptance.
The installer supplies a Hagia-specific policy adapter to Sophia's generic
`--policy-checker` interface and requires exactly one `policy=validated`
verdict. Missing validation and explicit deferral both refuse the build.

`install` builds the configured release first; `install DIRECTORY` selects an
explicit already built release. Installation requests sudo, installs into
`/opt/sophia-niltempus-desktop`, and
registers **Sophia niltempus Desktop** as a separate login choice. It leaves the
existing `/opt/sophia` installation and session entries in place. The new launcher
selects the generated profile and explicitly overrides the WM with the user-owned
Hagia at `~/.local/state/sophia-niltempus-desktop/development/hagia`, honoring
`XDG_STATE_HOME`. Installation seeds that path from the verified package only
when it is absent. An existing owned executable and its preparation metadata
remain unchanged, including their permissions and timestamps. Use
`prepare-hagia` to replace the personal WM deliberately. Sophia's native
launcher retains native scanout for overview.
Before each launch, the selected personal binary must confirm the generic
`SOPHIA_WM_POLICY_*` environment contract through its bounded
`config check-environment-contract` probe. Older personal binaries fail closed;
the installer also checks this probe before building a paired release or
publishing a development WM.
The normal login entry needs no separate development variant; installation
removes the earlier development entry.
Activation selects `current`; the former release is retained as `previous`.
`rollback` selects that previous release and preserves the personal Hagia.
Neither command
logs out or restarts a running session. External application and Lom config
files stay user-owned and are not rolled back with the release.

## Hagia updates and IPC reload

The normal **Sophia niltempus Desktop** login uses the user-owned Hagia path.
Sophia and the other packaged components remain root-owned under `/opt`. The
packaged Hagia is retained for provenance and initial setup; restarting the WM uses
the prepared personal binary, with no temporary release swap.

For WM source changes, run:

```sh
~/sophia-niltempus-desktop reload-hagia
```

This builds only committed Hagia source, validates it against installed Sophia
and the installed desktop profile, atomically replaces the user-owned executable,
and requests restart through Sophia's control IPC. It never invokes Cargo or
rebuilds Sophia or the shells. An unchanged, hash-verified Hagia build is reused.
`prepare-hagia` performs preparation alone, without IPC or any live action.

When the desired binary is already prepared, the direct command is:

```sh
sophia msg session restart-wm
```

Sophia enables the socket from the generated profile at startup and exports
`SOPHIA_CONTROL_SOCKET` to host applications. A session started before this
configuration was installed must be logged out once and started again. Setting
an environment variable cannot turn on the endpoint in an existing session.
The `host-admin` setting deliberately admits session control from authorized
host processes; it does not expose a WM-owned socket or delegate raw input.

The helper requires an explicit control endpoint and exactly one matching
user-owned Hagia with an owned checkpoint. It checks the replacement PID and
binary hash after the CLI confirms owner settlement. An IPC refusal stops the
helper; it does not fall back to signals, sudo or pathname swaps. Overview
closes on replacement. Sophia and application processes continue running.

The first system installation needs sudo to update the release and normal login
entry. Subsequent Hagia preparation and IPC reloads require no sudo. Tests use
private processes and a CLI fixture for executable replacement, and private
mounts for the real installation/rollback scripts. Live acceptance is separate.

## Chezmoi and development

Chezmoi tracks only the entrypoint, settings and source:

```text
~/sophia-niltempus-desktop
~/.config/sophia-niltempus-desktop/config.json
~/.local/share/sophia-niltempus-desktop/
```

The entrypoint uses Go's build cache and pinned `go.mod`/`go.sum` with
`-mod=readonly`, `-trimpath` and `-buildvcs=false`. Go 1.25.5 or newer is required.
The first invocation may download the pinned Go dependency. It does not require
Rust to compile the installer itself; building Sophia and Lom still needs Rust.
There are no chezmoi install hooks.

Caches live in `~/.cache/sophia-niltempus-desktop`; clones, logs and uninstalled
release artifacts live in `~/.local/state/sophia-niltempus-desktop`, except the
private source clones under the cache's `sources` directory. XDG base
directory overrides apply. These generated files are not tracked by chezmoi.

```sh
cd ~/.local/share/sophia-niltempus-desktop
go test ./...
go vet ./...
```

Tests cover profile preservation, exact executable selection, control opt-in, overview-binding
conflicts, release tampering, provenance changes, symlink refusal, inherited
session-variable removal and exclusive build ownership. Headless validation
does not establish physical GPU or live-session acceptance.

Build regressions cover a real Git checkout under umask 0002, safe child output
permissions, unchanged source inode/mtime retention across commits, dirty-cache
refusal, source-repository isolation and failure timing in logs. Compiler caches
are disposable; removing them causes a cold build. An unchanged, already sealed
release is hash-verified and reused without invoking compilers.

The optional integration control uses a real built package and the real CLI to
exercise installation, repeated installation, a second release and rollback.
It replaces `/opt` and the display-manager entry directory with private mounts,
makes the host filesystem read-only, hides devices, and substitutes an
unprivileged sudo fixture. It cannot install into the host's `/opt`:

```sh
go build -mod=readonly -trimpath -buildvcs=false -o ~/.cache/sophia-niltempus-desktop/installer .
DESKTOP_INSTALLER_TEST_RELEASE=/path/printed/by/build \
DESKTOP_INSTALLER_TEST_BINARY="$HOME/.cache/sophia-niltempus-desktop/installer" \
    go test -run '^TestInstallAndRollbackInPrivateMounts$' -v
```

After editing, add only these paths to chezmoi; avoid importing unrelated drift
from an existing personal desktop profile:

```sh
chezmoi add ~/sophia-niltempus-desktop \
    ~/.config/sophia-niltempus-desktop/config.json \
    ~/.local/share/sophia-niltempus-desktop
```
