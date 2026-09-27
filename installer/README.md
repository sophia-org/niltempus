# niltempus installer

Go installer and release assembler for the niltempus desktop. Its source lives
alongside the Rust packaging and qualification tools in this repository.
Sophia owns the compositor and session mechanisms; Hagia owns WM policy; Lom
and Bemenu provide this desktop's panel and launcher. Narthex is packaged for
Sophia's existing shell support. Desktop composition belongs here, outside
those repositories.

## Commands

For the next session, run this as your normal user:

```sh
niltempus install
```

It verifies and installs the prepared release, prompting for sudo. If no release
has been prepared, it builds from the configured sources first. Select
**Sophia niltempus Desktop** at your next login. A failed build or validation
stops before installation.

New releases are 9P-only (plan schema 3). The one login entry, **Sophia
niltempus Desktop**, runs Hagia, Lom and Bemenu over 9P2000.L. The release
contains one preflighted `desktop.kdl`. Installation writes no current-IPC
entry, and removes one left by an older release when that release is not the
selected one.

Plan schemas 1 and 2 (external release manifest schema 6, with the
**(current IPC)** entry) remain readable only so that installed releases can
be verified, re-activated when recorded, and rolled back to. Their original
files and checksums are unchanged. Plan schema 3 is the Go desktop plan. It
requires external (Sophia/integration) release manifest schema 7, a
separate numbering.

The individual steps remain available:

```sh
niltempus plan
niltempus build
niltempus prepare /path/to/an/existing/release
niltempus verify /path/printed/by/build
niltempus install /path/printed/by/build
niltempus status
niltempus rollback
```

`build` automatically selects its verified output for the next `install`.
`prepare` selects an existing release without rebuilding it. The selection in
user state binds its path, release ID and manifest digest. `install` verifies
these again; a missing or changed selected release is refused without silently
building or choosing a different one. `install DIRECTORY` remains an explicit
one-off installation and does not change the prepared selection.

### Explicit helper inputs

Plan schema 3 packages with the current helper CLI, so its configuration
names every input explicitly under `inputs`. There are no defaults:

- `hagia_nim_deps` and `narthex_nim_deps`: each is the path of the REVIEWED
  Nim dependency manifest plus its sha256, supplied independently of the
  file.
- `hagia_c_sdk_revision`: the C SDK revision that Hagia vendors.

`plan` refuses a missing, malformed or mismatched input before anything is
staged, and `build` checks them again before creating any directory. Each
manifest must:
- be a regular file at an absolute path;
- hash to its supplied digest;
- start `nim-deps schema=1 status=reviewed`;
- name its product and the plan's exact source commit.

A draft manifest authorizes nothing. The build copies the reviewed manifests
into its private directory, checks them again, and passes them to
`prepare-wm-pair` with `--build-dir`, `--hagia-nim-deps`,
`--hagia-nim-deps-sha256`, `--narthex-nim-deps`, `--narthex-nim-deps-sha256`
and `--hagia-c-sdk-rev`. It then runs `package-desktop` with
`--wm-pair-profile-sha256` and `--wm-pair-c-sdk-rev`. The plan binds the
paths, the digests and the revision, so they are part of the release
identity.

### Release verification

A schema-3 release must carry an external manifest of schema 7 that:
- records `hagia_c_sdk_revision` (equal to the plan's) and
  `hagia_c_sdk_manifest_sha256`;
- seals `share/sophia-policy/hagia/c-sdk.manifest.json`, which must hash to
  that digest and name that revision.

Missing, repeated, malformed or mismatched values are refused. With
`hagia_included=false`, the SDK fields and the sealed manifest must be
absent. This installer applies these checks itself; it never relies only on
a release's bundled verifier.

Activation follows the integration activator's history in
`/opt/sophia-niltempus-desktop/activated-releases`, whose lines are
`release_id manifest_sha256 SHA256SUMS_sha256`:
- A release activated before, and unchanged since, keeps its own
  verification. This is how an installed legacy release stays a rollback
  target.
- A recorded ID whose contents changed is refused.
- A release never activated must be a plan-schema-3 candidate.
- An installation without a ledger counts its current and previous
  releases once. After that, only ledger entries count.

`plan` is the default and reads local refs only. It never fetches, pulls,
checks out a branch in a working repository, or takes uncommitted edits.
Update each repository's local `master` deliberately before planning if newer
sources are wanted. Change a `reference` to an exact SHA to hold that component.
Each plan records the resolved commits and Git signature status (`G`: valid
trusted signature; `N`: unsigned). Present signatures must verify. Unsigned
commits remain visible as such; currently Lom master has an unsigned docs tip.
The niltempus packager requires trusted signatures for its own source,
Sophia, Hagia and Narthex.

`build` checks out those exact commits in private clones. The signed
niltempus Rust tool prepares the WM pair and packages Sophia;
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

The explicit `niltempus` repository entry selects both this installer and the
packaging tools. The retired `integration` setting is refused for new plans.
The installer's Go build information must identify the same clean Git revision;
an independently built or dirty installer cannot plan a release.
The committed Sophia pin must equal the selected Sophia commit. Provision
that niltempus revision explicitly before planning; its `.provision/accepted`
must match the committed lockfile and a private Cargo home outside the source
trees. The plan records that home, lockfile digest and signed tool commit.
Build revalidates the acceptance and runs the tool's pin/provision checks;
it never downloads dependencies or provisions a cache implicitly.

Packaging and activation scripts come from that niltempus revision.
The outer manifest records the five product commits, the niltempus binding,
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

`install` uses the prepared release; `install DIRECTORY` selects an
explicit already built release for that invocation. Installation requests sudo, installs into
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
niltempus reload-hagia
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

## Building and contributing

Build from a clean, signed niltempus revision. Go 1.25.5 or newer is required.
Provision the pinned Go dependencies explicitly first; the build below refuses
network downloads. Put the binary outside the checkout so it stays clean:

```sh
cd /path/to/niltempus/installer
go mod download
mkdir -p "$HOME/.local/bin"
GOPROXY=off GOSUMDB=off go build -mod=readonly -trimpath -buildvcs=true \
    -o "$HOME/.local/bin/niltempus" .
```

The config, cache, state and system release paths retain their historical
names so existing installations and rollback records remain usable. Chezmoi may
manage personal settings; it no longer owns the installer source or launcher.
Rust is not required to compile the Go installer itself.

Start from [examples/config.json](examples/config.json), replace every source
path and revision, and supply your own Sophia profile and application choices.
This assembles niltempus's selected components; Sophia and its independent C and
Rust SDK repositories remain usable with other desktops.

Caches live in `~/.cache/sophia-niltempus-desktop`; clones, logs and uninstalled
release artifacts live in `~/.local/state/sophia-niltempus-desktop`, except the
private source clones under the cache's `sources` directory. XDG base
directory overrides apply. These generated files are not tracked by chezmoi.

```sh
cd /path/to/niltempus/installer
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
go build -mod=readonly -trimpath -buildvcs=true -o ~/.cache/sophia-niltempus-desktop/installer .
DESKTOP_INSTALLER_TEST_RELEASE=/path/printed/by/build \
DESKTOP_INSTALLER_TEST_BINARY="$HOME/.cache/sophia-niltempus-desktop/installer" \
    go test -run '^TestInstallAndRollbackInPrivateMounts$' -v
```

Commit source changes here. Keep personal configuration, credentials, provisioned
caches and built releases out of Git. The build identity check is intentional:
after changing this installer, commit it and rebuild before planning a release.
